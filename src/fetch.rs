use std::ffi::OsString;
use std::io::Read;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use tempfile::NamedTempFile;
use ureq::config::RedirectAuthHeaders;

use crate::digest::{Sha256, copy_hashed};
use crate::report::Reporter;

/// Per-user cache of downloaded archives, keyed by sha256.
#[derive(Debug, Clone)]
pub struct Cache {
    dir: PathBuf,
}

impl Cache {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    pub fn from_env() -> Result<Self> {
        cache_dir_from(|key| std::env::var_os(key))
            .map(Self::new)
            .ok_or_else(|| anyhow!("cannot find a cache directory; set GDGET_CACHE_DIR"))
    }

    pub fn archive_path(&self, sha256: &Sha256) -> PathBuf {
        self.dir.join("sha256").join(format!("{sha256}.zip"))
    }

    fn temp_dir(&self) -> PathBuf {
        self.dir.join("tmp")
    }
}

fn cache_dir_from(env: impl Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    let var = |key: &str| env(key).filter(|v| !v.is_empty()).map(PathBuf::from);
    if let Some(dir) = var("GDGET_CACHE_DIR") {
        return Some(dir);
    }
    if cfg!(windows) {
        return var("LOCALAPPDATA").map(|dir| dir.join("gdget"));
    }
    if let Some(dir) = var("XDG_CACHE_HOME") {
        return Some(dir.join("gdget"));
    }
    let home = var("HOME")?;
    if cfg!(target_os = "macos") {
        Some(home.join("Library").join("Caches").join("gdget"))
    } else {
        Some(home.join(".cache").join("gdget"))
    }
}

/// The largest archive gdget downloads. The hash is only checked once a download ends, so
/// without a cap a hostile server could fill the disk before being caught.
pub const MAX_DOWNLOAD_BYTES: u64 = 2 << 30;

/// Downloads archives into the cache, verifying every archive it hands out.
pub struct Fetcher {
    agent: ureq::Agent,
    cache: Cache,
    github_token: Option<String>,
    max_bytes: u64,
    reporter: Reporter,
}

impl Fetcher {
    /// Creates a fetcher that uses `GITHUB_TOKEN` from the environment, if set.
    pub fn new(cache: Cache, reporter: Reporter) -> Self {
        let config = ureq::Agent::config_builder()
            .user_agent(concat!("gdget/", env!("CARGO_PKG_VERSION")))
            .timeout_connect(Some(Duration::from_secs(30)))
            .timeout_recv_response(Some(Duration::from_secs(60)))
            .timeout_recv_body(Some(Duration::from_secs(30 * 60)))
            // Release downloads redirect from github.com to a storage CDN; the token must
            // not follow.
            .redirect_auth_headers(RedirectAuthHeaders::Never)
            .build();
        Self {
            agent: config.into(),
            cache,
            github_token: std::env::var("GITHUB_TOKEN").ok().filter(|t| !t.is_empty()),
            max_bytes: MAX_DOWNLOAD_BYTES,
            reporter,
        }
    }

    pub fn with_github_token(mut self, token: Option<String>) -> Self {
        self.github_token = token;
        self
    }

    pub fn with_max_bytes(mut self, max_bytes: u64) -> Self {
        self.max_bytes = max_bytes;
        self
    }

    pub fn cache(&self) -> &Cache {
        &self.cache
    }

    /// Returns the path of a cached archive whose hash is `expected`, downloading it
    /// from `url` if it is not cached. A download with any other hash is an error.
    pub fn fetch_pinned(&self, url: &str, expected: &Sha256) -> Result<PathBuf> {
        let cached = self.cache.archive_path(expected);
        if cached.is_file() {
            let actual = Sha256::of_file(&cached)
                .with_context(|| format!("cannot read {}", cached.display()))?;
            if actual == *expected {
                return Ok(cached);
            }
            self.reporter.warn(format!(
                "cached archive {} is corrupt; downloading it again",
                cached.display()
            ));
            std::fs::remove_file(&cached)
                .with_context(|| format!("cannot remove {}", cached.display()))?;
        }

        let (actual, temp) = self.download(url)?;
        if actual != *expected {
            bail!(
                "sha256 mismatch for {url}\n  expected {expected}\n  got      {actual}\n\
                 The file changed after it was pinned. If the new file is intended, \
                 re-pin it with `gdget add`."
            );
        }
        self.store(temp, &actual)
    }

    /// Downloads `url` into the cache and returns its hash and cached path.
    pub fn fetch_unpinned(&self, url: &str) -> Result<(Sha256, PathBuf)> {
        let (sha256, temp) = self.download(url)?;
        Ok((sha256, self.store(temp, &sha256)?))
    }

    fn download(&self, url: &str) -> Result<(Sha256, NamedTempFile)> {
        self.reporter.action("Downloading", url);
        let mut request = self.agent.get(url);
        if let Some(token) = &self.github_token
            && sends_github_token(url)
        {
            request = request.header("Authorization", format!("Bearer {token}"));
        }
        let response = request
            .call()
            .with_context(|| format!("cannot download {url}"))?;

        let temp_dir = self.cache.temp_dir();
        std::fs::create_dir_all(&temp_dir)
            .with_context(|| format!("cannot create {}", temp_dir.display()))?;
        let mut temp = NamedTempFile::new_in(&temp_dir)
            .with_context(|| format!("cannot create a file in {}", temp_dir.display()))?;
        let mut body = response.into_body().into_reader().take(self.max_bytes + 1);
        let (size, sha256) = copy_hashed(&mut body, temp.as_file_mut())
            .with_context(|| format!("cannot download {url}"))?;
        if size > self.max_bytes {
            bail!(
                "cannot download {url}: it is larger than the {} MiB limit",
                self.max_bytes >> 20
            );
        }
        Ok((sha256, temp))
    }

    fn store(&self, temp: NamedTempFile, sha256: &Sha256) -> Result<PathBuf> {
        let path = self.cache.archive_path(sha256);
        let dir = path.parent().expect("archive paths have a parent");
        std::fs::create_dir_all(dir).with_context(|| format!("cannot create {}", dir.display()))?;
        temp.persist(&path)
            .map_err(|e| e.error)
            .with_context(|| format!("cannot write {}", path.display()))?;
        Ok(path)
    }
}

/// Whether `GITHUB_TOKEN` may be sent to `url`: only GitHub's own hosts, over https.
fn sends_github_token(url: &str) -> bool {
    let Ok(uri) = url.parse::<ureq::http::Uri>() else {
        return false;
    };
    let Some(host) = uri.host().map(str::to_ascii_lowercase) else {
        return false;
    };
    uri.scheme_str() == Some("https")
        && (host == "github.com"
            || host == "api.github.com"
            || host.ends_with(".githubusercontent.com"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_goes_only_to_github_over_https() {
        assert!(sends_github_token(
            "https://github.com/o/r/releases/download/v1/a.zip"
        ));
        assert!(sends_github_token("https://API.github.com/repos/o/r"));
        assert!(sends_github_token(
            "https://objects.githubusercontent.com/x"
        ));
        assert!(!sends_github_token("http://github.com/o/r/a.zip"));
        assert!(!sends_github_token("https://github.com.evil.example/a.zip"));
        assert!(!sends_github_token("https://notgithub.com/a.zip"));
        assert!(!sends_github_token(
            "https://evilgithubusercontent.com/a.zip"
        ));
        assert!(!sends_github_token("https://127.0.0.1/a.zip"));
    }

    #[test]
    fn explicit_cache_dir_wins() {
        let env = |key: &str| match key {
            "GDGET_CACHE_DIR" => Some(OsString::from("/explicit")),
            _ => Some(OsString::from("/other")),
        };
        assert_eq!(cache_dir_from(env), Some(PathBuf::from("/explicit")));
    }

    #[test]
    fn platform_cache_dir() {
        let env = |key: &str| match key {
            "LOCALAPPDATA" => Some(OsString::from("C:\\Users\\me\\AppData\\Local")),
            "XDG_CACHE_HOME" => Some(OsString::from("/xdg")),
            "HOME" => Some(OsString::from("/home/me")),
            _ => None,
        };
        let expected = if cfg!(windows) {
            PathBuf::from("C:\\Users\\me\\AppData\\Local").join("gdget")
        } else {
            PathBuf::from("/xdg").join("gdget")
        };
        assert_eq!(cache_dir_from(env), Some(expected));
    }

    #[test]
    fn empty_vars_are_ignored() {
        let env = |key: &str| match key {
            "GDGET_CACHE_DIR" | "XDG_CACHE_HOME" => Some(OsString::new()),
            "LOCALAPPDATA" => Some(OsString::from("L")),
            "HOME" => Some(OsString::from("H")),
            _ => None,
        };
        let expected = if cfg!(windows) {
            PathBuf::from("L").join("gdget")
        } else if cfg!(target_os = "macos") {
            PathBuf::from("H")
                .join("Library")
                .join("Caches")
                .join("gdget")
        } else {
            PathBuf::from("H").join(".cache").join("gdget")
        };
        assert_eq!(cache_dir_from(env), Some(expected));
    }
}
