use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use anyhow::{Context, Result, anyhow, bail};
use toml_edit::{DocumentMut, Item, Table, TableLike, value};

use crate::digest::Sha256;
use crate::git::{GitRev, check_git_url, check_ref};

pub const MANIFEST_FILE: &str = "addons.toml";
pub const OVERRIDES_FILE: &str = "addons.local.toml";

const NEWER_VERSION_HINT: &str = "it may need a newer version of gdget";

/// An addon's install folder name: it installs to `addons/<name>/`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AddonName(String);

impl AddonName {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for AddonName {
    type Err = anyhow::Error;

    fn from_str(name: &str) -> Result<Self> {
        let valid_chars = name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
        if name.is_empty() || !valid_chars {
            bail!("invalid addon name `{name}`: use ASCII letters, digits, `-`, `_` and `.`");
        }
        if name.starts_with('.') || name.ends_with('.') {
            bail!("invalid addon name `{name}`: it cannot start or end with `.`");
        }
        if is_reserved_on_windows(name) {
            bail!("invalid addon name `{name}`: it is a reserved file name on Windows");
        }
        Ok(Self(name.to_owned()))
    }
}

impl fmt::Display for AddonName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Windows opens these device names, with or without an extension, instead of a file.
pub(crate) fn is_reserved_on_windows(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or(name).to_ascii_uppercase();
    matches!(
        stem.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) || ((stem.starts_with("COM") || stem.starts_with("LPT"))
        && stem.len() == 4
        && stem.as_bytes()[3].is_ascii_digit())
}

/// Checks an addon source URL. Whitespace and control characters are refused because
/// URLs are echoed to CI logs, where a newline could inject a GitHub workflow command.
pub fn check_url(url: &str) -> Result<()> {
    if url.chars().any(|c| c.is_whitespace() || c.is_control()) {
        bail!("the URL {url:?} contains whitespace or control characters");
    }
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        bail!("`{url}` is not an http(s) URL");
    }
    Ok(())
}

/// Checks free text that gdget prints, such as a version label, for control characters.
pub fn check_text(text: &str) -> Result<()> {
    if text.chars().any(char::is_control) {
        bail!("{text:?} contains control characters");
    }
    Ok(())
}

/// A relative, `/`-separated folder inside an archive; no segments means the archive root.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ArchivePath(Vec<String>);

impl ArchivePath {
    pub fn root() -> Self {
        Self(Vec::new())
    }

    pub fn segments(&self) -> &[String] {
        &self.0
    }

    pub fn join(&self, segment: &str) -> Self {
        let mut segments = self.0.clone();
        segments.push(segment.to_owned());
        Self(segments)
    }
}

impl FromStr for ArchivePath {
    type Err = anyhow::Error;

    fn from_str(path: &str) -> Result<Self> {
        check_text(path)?;
        if path.contains('\\') {
            bail!("invalid path `{path}`: use `/` as the separator");
        }
        if path.starts_with('/') {
            bail!("invalid path `{path}`: it must be relative to the archive root");
        }
        let trimmed = path.trim_end_matches('/');
        let mut segments = Vec::new();
        for segment in trimmed.split('/') {
            match segment {
                "." => {}
                "" if trimmed.is_empty() => {}
                "" | ".." => bail!("invalid path `{path}`: empty or `..` segment"),
                _ => segments.push(segment.to_owned()),
            }
        }
        Ok(Self(segments))
    }
}

impl fmt::Display for ArchivePath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.0.is_empty() {
            f.write_str(".")
        } else {
            f.write_str(&self.0.join("/"))
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Addon {
    /// Display-only label; the pin is the source's hash.
    pub version: Option<String>,
    pub source: Source,
    pub path: Option<ArchivePath>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    Url {
        url: String,
        sha256: Sha256,
    },
    Git {
        url: String,
        /// The branch, tag or commit `rev` was resolved from; `None` means the remote's
        /// default branch. Display and re-pinning only.
        reference: Option<String>,
        rev: GitRev,
    },
}

impl Source {
    pub fn url(&self) -> &str {
        match self {
            Source::Url { url, .. } | Source::Git { url, .. } => url,
        }
    }

    /// What an install of this source is pinned to.
    pub fn pin(&self) -> Pin {
        match self {
            Source::Url { sha256, .. } => Pin::Sha256(*sha256),
            Source::Git { rev, .. } => Pin::Rev(rev.clone()),
        }
    }

    /// The pin as `status` shows it, with the git ref it came from.
    pub fn short_pin(&self) -> String {
        match self {
            Source::Url { sha256, .. } => sha256.short(),
            Source::Git {
                reference: Some(reference),
                rev,
                ..
            } => format!("{reference}@{}", rev.short()),
            Source::Git { rev, .. } => rev.short(),
        }
    }
}

/// Identifies exactly what gets installed: an archive by its hash, or a git commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pin {
    Sha256(Sha256),
    Rev(GitRev),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Manifest {
    pub addons: BTreeMap<AddonName, Addon>,
}

impl Manifest {
    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("cannot read {}", path.display()))?;
        Self::parse(&text).with_context(|| format!("invalid {}", path.display()))
    }

    pub fn parse(text: &str) -> Result<Self> {
        let doc: DocumentMut = text.parse()?;
        let mut manifest = Self::default();
        for (key, item) in doc.iter() {
            if key != "addons" {
                bail!("unknown top-level key `{key}` ({NEWER_VERSION_HINT})");
            }
            let addons = item
                .as_table_like()
                .ok_or_else(|| anyhow!("`addons` must be a table"))?;
            for (name, item) in addons.iter() {
                let addon = item
                    .as_table_like()
                    .ok_or_else(|| anyhow!("`addons.{name}` must be a table"))
                    .and_then(|table| parse_addon(name, table));
                manifest.addons.insert(name.parse()?, addon?);
            }
        }
        Ok(manifest)
    }
}

fn parse_addon(name: &str, table: &dyn TableLike) -> Result<Addon> {
    let get = |key: &str| -> Result<Option<&str>> {
        table
            .get(key)
            .map(|item| {
                item.as_str()
                    .ok_or_else(|| anyhow!("`addons.{name}.{key}` must be a string"))
            })
            .transpose()
    };
    if let Some((key, _)) = table.iter().find(|(key, _)| {
        !matches!(
            *key,
            "version" | "url" | "sha256" | "git" | "ref" | "rev" | "path"
        )
    }) {
        bail!("unknown key `addons.{name}.{key}` ({NEWER_VERSION_HINT})");
    }
    let only_for = |keys: &[&str], source: &str| -> Result<()> {
        match keys.iter().find(|key| table.contains_key(key)) {
            Some(key) => bail!("`addons.{name}.{key}` only applies to `{source}` sources"),
            None => Ok(()),
        }
    };
    let source = match (get("url")?, get("git")?) {
        (Some(_), Some(_)) => bail!("`addons.{name}` has both `url` and `git`; keep one"),
        (None, None) => bail!("`addons.{name}` is missing `url` or `git`"),
        (Some(url), None) => {
            only_for(&["ref", "rev"], "git")?;
            check_url(url).with_context(|| format!("invalid `addons.{name}.url`"))?;
            let sha256 = get("sha256")?
                .ok_or_else(|| {
                    anyhow!(
                        "`addons.{name}` is missing `sha256` (`gdget add` computes and pins it)"
                    )
                })?
                .parse()
                .with_context(|| format!("invalid `addons.{name}.sha256`"))?;
            Source::Url {
                url: url.to_owned(),
                sha256,
            }
        }
        (None, Some(url)) => {
            only_for(&["sha256"], "url")?;
            check_git_url(url).with_context(|| format!("invalid `addons.{name}.git`"))?;
            let reference = get("ref")?;
            if let Some(reference) = reference {
                check_ref(reference).with_context(|| format!("invalid `addons.{name}.ref`"))?;
            }
            let rev = get("rev")?
                .ok_or_else(|| {
                    anyhow!("`addons.{name}` is missing `rev` (`gdget add` resolves and pins it)")
                })?
                .parse()
                .with_context(|| format!("invalid `addons.{name}.rev`"))?;
            Source::Git {
                url: url.to_owned(),
                reference: reference.map(str::to_owned),
                rev,
            }
        }
    };
    let version = get("version")?;
    if let Some(version) = version {
        check_text(version).with_context(|| format!("invalid `addons.{name}.version`"))?;
    }
    let path = get("path")?
        .map(str::parse)
        .transpose()
        .with_context(|| format!("invalid `addons.{name}.path`"))?;
    Ok(Addon {
        version: version.map(str::to_owned),
        source,
        path,
    })
}

/// Local overrides from the gitignored `addons.local.toml`: addon name to a directory,
/// relative to the project root, that is linked in place of the pinned archive.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Overrides {
    pub addons: BTreeMap<AddonName, PathBuf>,
}

impl Overrides {
    /// Loads the overrides file; a missing file means no overrides.
    pub fn load(path: &Path) -> Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(text) => Self::parse(&text).with_context(|| format!("invalid {}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e).with_context(|| format!("cannot read {}", path.display())),
        }
    }

    pub fn parse(text: &str) -> Result<Self> {
        let doc: DocumentMut = text.parse()?;
        let mut overrides = Self::default();
        for (key, item) in doc.iter() {
            if key != "overrides" {
                bail!("unknown top-level key `{key}` ({NEWER_VERSION_HINT})");
            }
            let table = item
                .as_table_like()
                .ok_or_else(|| anyhow!("`overrides` must be a table"))?;
            for (name, item) in table.iter() {
                let dir = item
                    .as_str()
                    .filter(|dir| !dir.is_empty())
                    .ok_or_else(|| anyhow!("`overrides.{name}` must be a directory path"))?;
                check_text(dir).with_context(|| format!("invalid `overrides.{name}`"))?;
                overrides.addons.insert(name.parse()?, PathBuf::from(dir));
            }
        }
        Ok(overrides)
    }
}

/// `addons.toml` opened for editing; comments and formatting outside the edited entry are
/// preserved.
pub struct ManifestFile {
    path: PathBuf,
    doc: DocumentMut,
}

impl ManifestFile {
    /// Opens the manifest for editing; a missing file starts an empty one.
    pub fn open(path: &Path) -> Result<Self> {
        let doc = match std::fs::read_to_string(path) {
            Ok(text) => {
                Manifest::parse(&text).with_context(|| format!("invalid {}", path.display()))?;
                text.parse()?
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => DocumentMut::new(),
            Err(e) => return Err(e).with_context(|| format!("cannot read {}", path.display())),
        };
        Ok(Self {
            path: path.to_owned(),
            doc,
        })
    }

    pub fn set(&mut self, name: &AddonName, addon: &Addon) {
        let addons = self.doc.entry("addons").or_insert_with(|| {
            let mut table = Table::new();
            table.set_implicit(true);
            Item::Table(table)
        });
        let addons = addons
            .as_table_like_mut()
            .expect("validated as a table when opened");
        let entry = addons
            .entry(name.as_str())
            .or_insert_with(|| Item::Table(Table::new()));
        let entry = entry
            .as_table_like_mut()
            .expect("validated as a table when opened");

        set_or_remove(entry, "version", addon.version.clone());
        match &addon.source {
            Source::Url { url, sha256 } => {
                for key in ["git", "ref", "rev"] {
                    entry.remove(key);
                }
                entry.insert("url", value(url.as_str()));
                entry.insert("sha256", value(sha256.to_string()));
            }
            Source::Git {
                url,
                reference,
                rev,
            } => {
                for key in ["url", "sha256"] {
                    entry.remove(key);
                }
                entry.insert("git", value(url.as_str()));
                set_or_remove(entry, "ref", reference.clone());
                entry.insert("rev", value(rev.as_str()));
            }
        }
        set_or_remove(entry, "path", addon.path.as_ref().map(ToString::to_string));
    }

    /// Removes an addon's entry, returning whether it was present.
    pub fn remove(&mut self, name: &AddonName) -> bool {
        self.doc
            .get_mut("addons")
            .and_then(Item::as_table_like_mut)
            .is_some_and(|addons| addons.remove(name.as_str()).is_some())
    }

    pub fn save(&self) -> Result<()> {
        crate::fsutil::write_atomic(&self.path, self.doc.to_string().as_bytes())
            .with_context(|| format!("cannot write {}", self.path.display()))
    }
}

/// `addons.local.toml` opened for editing; comments and formatting outside the edited
/// entry are preserved.
pub struct OverridesFile {
    path: PathBuf,
    doc: DocumentMut,
}

impl OverridesFile {
    /// Opens the overrides file for editing; a missing file starts an empty one.
    pub fn open(path: &Path) -> Result<Self> {
        let doc = match std::fs::read_to_string(path) {
            Ok(text) => {
                Overrides::parse(&text).with_context(|| format!("invalid {}", path.display()))?;
                text.parse()?
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => DocumentMut::new(),
            Err(e) => return Err(e).with_context(|| format!("cannot read {}", path.display())),
        };
        Ok(Self {
            path: path.to_owned(),
            doc,
        })
    }

    /// Points `name` at `dir`, a path relative to the project root.
    pub fn set(&mut self, name: &AddonName, dir: &str) {
        self.doc
            .entry("overrides")
            .or_insert_with(|| Item::Table(Table::new()))
            .as_table_like_mut()
            .expect("validated as a table when opened")
            .insert(name.as_str(), value(dir));
    }

    /// Removes an override, returning whether it was present.
    pub fn remove(&mut self, name: &AddonName) -> bool {
        self.doc
            .get_mut("overrides")
            .and_then(Item::as_table_like_mut)
            .is_some_and(|overrides| overrides.remove(name.as_str()).is_some())
    }

    pub fn save(&self) -> Result<()> {
        crate::fsutil::write_atomic(&self.path, self.doc.to_string().as_bytes())
            .with_context(|| format!("cannot write {}", self.path.display()))
    }
}

fn set_or_remove(table: &mut dyn TableLike, key: &str, new: Option<String>) {
    match new {
        Some(new) => {
            table.insert(key, value(new));
        }
        None => {
            table.remove(key);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HASH: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

    fn name(s: &str) -> AddonName {
        s.parse().unwrap()
    }

    fn manifest(text: &str) -> Result<Manifest> {
        Manifest::parse(&text.replace("HASH", HASH))
    }

    fn error(result: Result<impl fmt::Debug>) -> String {
        format!("{:#}", result.unwrap_err())
    }

    #[test]
    fn parses_full_entry_and_inline_tables() {
        let parsed = manifest(
            r#"
            [addons.godot-slang]
            version = "0.4.1"
            url = "https://example.com/slang.zip"
            sha256 = "HASH"
            path = "addons/godot-slang/"

            [addons]
            godot-verse = { url = "http://127.0.0.1/verse.zip", sha256 = "HASH" }
            "#,
        )
        .unwrap();
        let slang = &parsed.addons[&name("godot-slang")];
        assert_eq!(slang.version.as_deref(), Some("0.4.1"));
        assert_eq!(slang.path, Some("addons/godot-slang".parse().unwrap()));
        let verse = &parsed.addons[&name("godot-verse")];
        assert_eq!(verse.path, None);
        assert_eq!(
            verse.source,
            Source::Url {
                url: "http://127.0.0.1/verse.zip".into(),
                sha256: HASH.parse().unwrap(),
            }
        );
    }

    const REV: &str = "afde39c3f0e1afde39c3f0e1afde39c3f0e1afde";

    #[test]
    fn parses_git_sources() {
        let parsed = manifest(&format!(
            r#"
            [addons.inventory]
            git = "https://github.com/DevPrice/godot-addons.git"
            ref = "main"
            rev = "{REV}"
            path = "inventory"

            [addons.flat]
            git = "git@github.com:DevPrice/flat.git"
            rev = "{REV}"
            "#
        ))
        .unwrap();
        let inventory = &parsed.addons[&name("inventory")];
        assert_eq!(
            inventory.source,
            Source::Git {
                url: "https://github.com/DevPrice/godot-addons.git".into(),
                reference: Some("main".into()),
                rev: REV.parse().unwrap(),
            }
        );
        assert_eq!(inventory.source.short_pin(), "main@afde39c3f0e1");
        assert_eq!(
            parsed.addons[&name("flat")].source.short_pin(),
            "afde39c3f0e1"
        );
    }

    #[test]
    fn git_and_url_fields_do_not_mix() {
        let git = "[addons.a]\ngit = \"https://x/a.git\"\n";
        let err = error(manifest(&format!("{git}ref = \"main\"")));
        assert!(err.contains("missing `rev`"), "{err}");

        let err = error(manifest(&format!("{git}rev = \"abc\"")));
        assert!(err.contains("invalid `addons.a.rev`"), "{err}");

        let err = error(manifest(&format!(
            "{git}rev = \"{REV}\"\nsha256 = \"HASH\""
        )));
        assert!(
            err.contains("`addons.a.sha256` only applies to `url`"),
            "{err}"
        );

        let err = error(manifest(&format!(
            "[addons.a]\nurl = \"https://x/a.zip\"\nsha256 = \"HASH\"\nrev = \"{REV}\""
        )));
        assert!(
            err.contains("`addons.a.rev` only applies to `git`"),
            "{err}"
        );

        let err = error(manifest(&format!(
            "{git}url = \"https://x/a.zip\"\nrev = \"{REV}\""
        )));
        assert!(err.contains("both `url` and `git`"), "{err}");

        let err = error(manifest(&format!(
            "[addons.a]\ngit = \"file:///a.git\"\nrev = \"{REV}\""
        )));
        assert!(err.contains("invalid `addons.a.git`"), "{err}");

        let err = error(manifest(&format!("{git}rev = \"{REV}\"\nref = \"--x\"")));
        assert!(err.contains("invalid `addons.a.ref`"), "{err}");
    }

    #[test]
    fn switching_source_kinds_drops_the_old_fields() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(MANIFEST_FILE);
        let url = Addon {
            version: None,
            source: Source::Url {
                url: "https://x/a.zip".into(),
                sha256: HASH.parse().unwrap(),
            },
            path: None,
        };
        let git = Addon {
            version: None,
            source: Source::Git {
                url: "https://x/a.git".into(),
                reference: Some("v1".into()),
                rev: REV.parse().unwrap(),
            },
            path: Some(ArchivePath::root()),
        };
        let mut file = ManifestFile::open(&path).unwrap();
        file.set(&name("a"), &url);
        file.set(&name("a"), &git);
        file.save().unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.contains("sha256"), "{text}");
        assert_eq!(Manifest::parse(&text).unwrap().addons[&name("a")], git);

        let mut file = ManifestFile::open(&path).unwrap();
        file.set(&name("a"), &url);
        file.save().unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.contains("rev") && !text.contains("ref"), "{text}");
        assert_eq!(Manifest::parse(&text).unwrap().addons[&name("a")], url);
    }

    #[test]
    fn empty_manifest_has_no_addons() {
        assert!(manifest("").unwrap().addons.is_empty());
    }

    #[test]
    fn unknown_keys_suggest_a_newer_gdget() {
        let err = error(manifest(
            "[addons.a]\nurl = \"https://x/a.zip\"\nsha256 = \"HASH\"\nplatforms = []",
        ));
        assert!(err.contains("unknown key `addons.a.platforms`"), "{err}");
        assert!(err.contains("newer version of gdget"), "{err}");

        let err = error(manifest("min_gdget = \"0.2\""));
        assert!(err.contains("unknown top-level key `min_gdget`"), "{err}");
    }

    #[test]
    fn missing_or_malformed_fields_name_the_key() {
        let err = error(manifest("[addons.a]\nsha256 = \"HASH\""));
        assert!(
            err.contains("`addons.a` is missing `url` or `git`"),
            "{err}"
        );

        let err = error(manifest("[addons.a]\nurl = \"https://x/a.zip\""));
        assert!(err.contains("missing `sha256`"), "{err}");

        let err = error(manifest(
            "[addons.a]\nurl = \"https://x\"\nsha256 = \"abc\"",
        ));
        assert!(err.contains("invalid `addons.a.sha256`"), "{err}");

        let err = error(manifest(
            "[addons.a]\nurl = \"file:///a.zip\"\nsha256 = \"HASH\"",
        ));
        assert!(err.contains("not an http(s) URL"), "{err}");

        let err = error(manifest("[addons.a]\nurl = 3\nsha256 = \"HASH\""));
        assert!(err.contains("`addons.a.url` must be a string"), "{err}");
    }

    #[test]
    fn printed_fields_reject_control_characters() {
        let err = error(manifest(
            "[addons.a]\nurl = \"https://x/a.zip\\n::error::fake\"\nsha256 = \"HASH\"",
        ));
        assert!(err.contains("invalid `addons.a.url`"), "{err}");
        assert!(!err.contains('\n'), "{err}");

        let err = error(manifest(
            "[addons.a]\nurl = \"https://x/a.zip\"\nsha256 = \"HASH\"\nversion = \"\\u001b[31m\"",
        ));
        assert!(err.contains("invalid `addons.a.version`"), "{err}");

        assert!(Overrides::parse("[overrides]\na = \"x\\ny\"").is_err());
    }

    #[test]
    fn addon_names_must_be_safe_folder_names() {
        for good in ["godot-slang", "godot_verse", "gut.v9", "A1"] {
            assert!(good.parse::<AddonName>().is_ok(), "{good}");
        }
        for bad in [
            "",
            "..",
            ".hidden",
            "trailing.",
            "a/b",
            "a\\b",
            "sp ace",
            "con",
            "Com1.x",
        ] {
            assert!(bad.parse::<AddonName>().is_err(), "{bad}");
        }
        assert!("console".parse::<AddonName>().is_ok());
    }

    #[test]
    fn archive_paths_are_normalized_and_confined() {
        assert_eq!(
            "addons/x/".parse::<ArchivePath>().unwrap().to_string(),
            "addons/x"
        );
        assert_eq!(
            "./addons/x".parse::<ArchivePath>().unwrap().to_string(),
            "addons/x"
        );
        assert_eq!(".".parse::<ArchivePath>().unwrap(), ArchivePath::root());
        assert_eq!("".parse::<ArchivePath>().unwrap(), ArchivePath::root());
        assert_eq!(ArchivePath::root().to_string(), ".");
        for bad in ["../x", "a/../b", "/abs", "a\\b", "a//b"] {
            assert!(bad.parse::<ArchivePath>().is_err(), "{bad}");
        }
    }

    #[test]
    fn overrides_parse_and_reject_other_tables() {
        let overrides =
            Overrides::parse("[overrides]\ngodot-slang = \"../slang/addons/godot-slang\"").unwrap();
        assert_eq!(
            overrides.addons[&name("godot-slang")],
            PathBuf::from("../slang/addons/godot-slang")
        );
        assert!(Overrides::parse("[addons]").is_err());
        assert!(Overrides::parse("[overrides]\na = \"\"").is_err());
    }

    #[test]
    fn missing_overrides_file_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        let overrides = Overrides::load(&dir.path().join(OVERRIDES_FILE)).unwrap();
        assert!(overrides.addons.is_empty());
    }

    #[test]
    fn editing_preserves_comments_and_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(MANIFEST_FILE);
        std::fs::write(
            &path,
            format!(
                "# Pinned addons for this game.\n\n\
                 [addons.keep] # stays as-is\n\
                 url = \"https://x/keep.zip\"\n\
                 sha256 = \"{HASH}\"\n\n\
                 [addons.gone]\n\
                 url = \"https://x/gone.zip\"\n\
                 sha256 = \"{HASH}\"\n"
            ),
        )
        .unwrap();

        let added = Addon {
            version: Some("1.0".into()),
            source: Source::Url {
                url: "https://x/new.zip".into(),
                sha256: HASH.parse().unwrap(),
            },
            path: Some("addons/new".parse().unwrap()),
        };
        let mut file = ManifestFile::open(&path).unwrap();
        file.set(&name("new"), &added);
        assert!(file.remove(&name("gone")));
        assert!(!file.remove(&name("never-there")));
        file.save().unwrap();

        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.starts_with("# Pinned addons for this game."), "{text}");
        assert!(text.contains("[addons.keep] # stays as-is"), "{text}");
        assert!(!text.contains("gone"), "{text}");
        let reparsed = Manifest::parse(&text).unwrap();
        assert_eq!(reparsed.addons.len(), 2);
        assert_eq!(reparsed.addons[&name("new")], added);
    }

    #[test]
    fn repinning_updates_entry_in_place() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(MANIFEST_FILE);
        std::fs::write(
            &path,
            format!(
                "[addons.a]\nversion = \"1\" # old\nurl = \"https://x/1.zip\"\n\
                 sha256 = \"{HASH}\"\n"
            ),
        )
        .unwrap();

        let mut file = ManifestFile::open(&path).unwrap();
        let repinned = Addon {
            version: None,
            source: Source::Url {
                url: "https://x/2.zip".into(),
                sha256: HASH.parse().unwrap(),
            },
            path: Some(ArchivePath::root()),
        };
        file.set(&name("a"), &repinned);
        file.save().unwrap();

        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.contains("version"), "{text}");
        assert!(text.contains("path = \".\""), "{text}");
        assert_eq!(Manifest::parse(&text).unwrap().addons[&name("a")], repinned);
    }

    #[test]
    fn names_with_dots_are_quoted_keys() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(MANIFEST_FILE);
        let name = name("gut.v9");
        let mut file = ManifestFile::open(&path).unwrap();
        file.set(
            &name,
            &Addon {
                version: None,
                source: Source::Url {
                    url: "https://x/gut.zip".into(),
                    sha256: HASH.parse().unwrap(),
                },
                path: None,
            },
        );
        file.save().unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("[addons.\"gut.v9\"]"), "{text}");
        assert!(Manifest::parse(&text).unwrap().addons.contains_key(&name));
    }

    #[test]
    fn editing_overrides_preserves_comments_and_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(OVERRIDES_FILE);
        let mut file = OverridesFile::open(&path).unwrap();
        file.set(&name("gut.v9"), "../gut");
        file.save().unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "[overrides]\n\"gut.v9\" = \"../gut\"\n"
        );

        std::fs::write(
            &path,
            "# my builds\n[overrides]\nkeep = \"../keep\" # stays\ngone = \"../gone\"\n",
        )
        .unwrap();
        let mut file = OverridesFile::open(&path).unwrap();
        file.set(&name("keep"), "../moved");
        assert!(file.remove(&name("gone")));
        assert!(!file.remove(&name("never-there")));
        file.save().unwrap();

        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.starts_with("# my builds\n"), "{text}");
        assert!(!text.contains("gone"), "{text}");
        let overrides = Overrides::parse(&text).unwrap();
        assert_eq!(overrides.addons[&name("keep")], PathBuf::from("../moved"));
        assert_eq!(overrides.addons.len(), 1);
    }

    #[test]
    fn new_manifest_uses_dotted_table_headers() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(MANIFEST_FILE);
        let mut file = ManifestFile::open(&path).unwrap();
        file.set(
            &name("a"),
            &Addon {
                version: None,
                source: Source::Url {
                    url: "https://x/a.zip".into(),
                    sha256: HASH.parse().unwrap(),
                },
                path: None,
            },
        );
        file.save().unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.starts_with("[addons.a]\n"), "{text}");
    }
}
