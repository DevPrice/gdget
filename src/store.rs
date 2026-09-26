use std::fmt;
use std::str::FromStr;

use anyhow::{Context, Result, anyhow, bail};
use serde::Deserialize;

use crate::fetch::Fetcher;
use crate::manifest::{check_text, check_url};

pub const DEFAULT_STORE_URL: &str = "https://store.godotengine.org";

/// Caps the release list reply, which gdget holds in memory to parse.
const MAX_RESPONSE_BYTES: u64 = 4 << 20;

/// A Godot Asset Store asset, written `PUBLISHER/ASSET[@VERSION]`. Slugs are lowercased
/// because the store matches them case-insensitively and pinned URLs should be canonical.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoreRef {
    pub publisher: String,
    pub asset: String,
    /// `None` means the latest stable release.
    pub version: Option<String>,
}

impl FromStr for StoreRef {
    type Err = anyhow::Error;

    fn from_str(spec: &str) -> Result<Self> {
        let (slugs, version) = match spec.split_once('@') {
            Some((slugs, version)) => (slugs, Some(version)),
            None => (spec, None),
        };
        let Some((publisher, asset)) = slugs.split_once('/') else {
            bail!("`{spec}` is not a URL or an Asset Store PUBLISHER/ASSET[@VERSION]");
        };
        let version = version
            .map(|version| {
                if version.is_empty() || version.chars().any(|c| c.is_whitespace()) {
                    bail!("invalid version in `{spec}`");
                }
                check_text(version)?;
                Ok(version.to_owned())
            })
            .transpose()?;
        Ok(Self {
            publisher: slug(spec, publisher)?,
            asset: slug(spec, asset)?,
            version,
        })
    }
}

/// Slugs are pasted into API and download URLs, so only URL-safe path segments pass.
fn slug(spec: &str, slug: &str) -> Result<String> {
    let valid = !slug.is_empty()
        && !slug.starts_with('.')
        && slug
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
    if !valid {
        bail!(
            "`{spec}` is not a URL or an Asset Store PUBLISHER/ASSET[@VERSION]: `{slug}` must \
             use ASCII letters, digits, `-`, `_` and `.`"
        );
    }
    Ok(slug.to_ascii_lowercase())
}

impl fmt::Display for StoreRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.publisher, self.asset)?;
        if let Some(version) = &self.version {
            write!(f, "@{version}")?;
        }
        Ok(())
    }
}

/// A release chosen from the store, with a download URL that stays valid for pinning.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    pub version: String,
    pub url: String,
}

#[derive(Debug, Deserialize)]
struct ApiRelease {
    id: u64,
    version: String,
    stable: Option<bool>,
}

/// The Godot Asset Store's API, as used by the Godot editor.
#[derive(Debug, Clone)]
pub struct Store {
    base: String,
}

impl Store {
    /// The store at `GDGET_STORE_URL`, or the official one.
    pub fn from_env() -> Result<Self> {
        let base = std::env::var("GDGET_STORE_URL")
            .ok()
            .filter(|url| !url.is_empty())
            .unwrap_or_else(|| DEFAULT_STORE_URL.to_owned());
        Self::new(&base).context("invalid GDGET_STORE_URL")
    }

    pub fn new(base: &str) -> Result<Self> {
        check_url(base)?;
        Ok(Self {
            base: base.trim_end_matches('/').to_owned(),
        })
    }

    /// Finds the release `asset` asks for.
    pub fn resolve(&self, fetcher: &Fetcher, asset: &StoreRef) -> Result<Release> {
        let StoreRef {
            publisher,
            asset: slug,
            version,
        } = asset;
        let url = format!("{}/api/v1/releases/{publisher}/{slug}/", self.base);
        let body = fetcher
            .fetch_bytes(&url, MAX_RESPONSE_BYTES)
            .map_err(|err| {
                if is_not_found(&err) {
                    anyhow!(
                        "`{publisher}/{slug}` is not on the Asset Store at {}",
                        self.base
                    )
                } else {
                    err
                }
            })?;
        let releases: Vec<ApiRelease> = serde_json::from_slice(&body)
            .with_context(|| format!("unexpected reply from {url}"))?;
        let release = pick(&releases, version.as_deref())
            .with_context(|| format!("cannot add `{publisher}/{slug}`"))?;
        check_text(&release.version).with_context(|| format!("unexpected reply from {url}"))?;
        Ok(Release {
            version: release.version.clone(),
            // The API's own download_url is a signed link that expires in minutes; this
            // page link redirects to a fresh one on every request.
            url: format!(
                "{}/asset/{publisher}/{slug}/download/{}/",
                self.base, release.id
            ),
        })
    }
}

fn is_not_found(err: &anyhow::Error) -> bool {
    err.chain().any(|cause| {
        matches!(
            cause.downcast_ref::<ureq::Error>(),
            Some(ureq::Error::StatusCode(404))
        )
    })
}

/// Picks `version`, or else the first stable release; the store lists newest first.
fn pick<'a>(releases: &'a [ApiRelease], version: Option<&str>) -> Result<&'a ApiRelease> {
    let found = match version {
        Some(wanted) => releases.iter().find(|r| same_version(&r.version, wanted)),
        None => releases.iter().find(|r| r.stable != Some(false)),
    };
    if let Some(release) = found {
        return Ok(release);
    }
    if releases.is_empty() {
        bail!("the asset has no releases");
    }
    let available = releases
        .iter()
        .map(|r| r.version.as_str())
        .filter(|v| check_text(v).is_ok())
        .collect::<Vec<_>>()
        .join(", ");
    match version {
        Some(wanted) => bail!("no release `{wanted}`; available: {available}"),
        None => bail!("no stable release; name a version with @VERSION: {available}"),
    }
}

/// Versions match exactly, except that a leading `v` is optional on either side.
fn same_version(a: &str, b: &str) -> bool {
    fn bare(version: &str) -> &str {
        version.strip_prefix(['v', 'V']).unwrap_or(version)
    }
    bare(a) == bare(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release(id: u64, version: &str, stable: Option<bool>) -> ApiRelease {
        ApiRelease {
            id,
            version: version.to_owned(),
            stable,
        }
    }

    #[test]
    fn parses_specs_and_lowercases_slugs() {
        let spec: StoreRef = "DevPrice/godot-slang@v6.0.0".parse().unwrap();
        assert_eq!(spec.publisher, "devprice");
        assert_eq!(spec.asset, "godot-slang");
        assert_eq!(spec.version.as_deref(), Some("v6.0.0"));
        assert_eq!(spec.to_string(), "devprice/godot-slang@v6.0.0");

        let spec: StoreRef = "a/b".parse().unwrap();
        assert_eq!(spec.version, None);
    }

    #[test]
    fn rejects_specs_that_are_not_safe_url_segments() {
        for bad in [
            "slang",
            "a/",
            "/b",
            "a/b/c",
            "a/..",
            "a/.b",
            "a/b?x",
            "a/b#x",
            "a/b%2F",
            "a/b@",
            "a/b@v 1",
            "a/b@\u{1b}[31m",
        ] {
            assert!(bad.parse::<StoreRef>().is_err(), "{bad}");
        }
    }

    #[test]
    fn picks_the_first_stable_release_by_default() {
        let releases = [
            release(3, "v3.0.0-beta", Some(false)),
            release(2, "v2.0.0", Some(true)),
            release(1, "v1.0.0", None),
        ];
        assert_eq!(pick(&releases, None).unwrap().id, 2);
        assert_eq!(pick(&releases, Some("v3.0.0-beta")).unwrap().id, 3);
    }

    #[test]
    fn version_match_ignores_a_leading_v() {
        let releases = [release(2, "v2.0.0", None), release(1, "1.0.0", None)];
        assert_eq!(pick(&releases, Some("2.0.0")).unwrap().id, 2);
        assert_eq!(pick(&releases, Some("v1.0.0")).unwrap().id, 1);
        assert!(pick(&releases, Some("2.0")).is_err());
    }

    #[test]
    fn missing_versions_list_what_exists() {
        let releases = [release(2, "v2.0.0", None), release(1, "v1.0\n::x", None)];
        let err = format!("{:#}", pick(&releases, Some("v9")).unwrap_err());
        assert_eq!(err, "no release `v9`; available: v2.0.0");

        let err = format!("{:#}", pick(&[], None).unwrap_err());
        assert_eq!(err, "the asset has no releases");

        let unstable = [release(1, "v1.0.0-rc", Some(false))];
        let err = format!("{:#}", pick(&unstable, None).unwrap_err());
        assert!(err.contains("no stable release"), "{err}");
        assert!(err.contains("v1.0.0-rc"), "{err}");
    }

    #[test]
    fn store_url_must_be_http() {
        assert!(Store::new("ftp://x").is_err());
        assert_eq!(
            Store::new("https://store.example/").unwrap().base,
            "https://store.example"
        );
    }
}
