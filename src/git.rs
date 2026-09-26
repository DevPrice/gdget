use std::ffi::OsString;
use std::fmt;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::str::FromStr;
use std::sync::atomic::{AtomicUsize, Ordering};

use anyhow::{Context, Result, anyhow, bail};
use tempfile::{NamedTempFile, TempPath};

use crate::fetch::Cache;
use crate::report::Reporter;

/// A full git commit hash: 40 hex digits for SHA-1 repositories, 64 for SHA-256 ones.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GitRev(String);

impl GitRev {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The first 12 hex digits, matching the length `status` shows for archive hashes.
    pub fn short(&self) -> String {
        self.0[..12].to_owned()
    }
}

impl FromStr for GitRev {
    type Err = anyhow::Error;

    fn from_str(rev: &str) -> Result<Self> {
        if !matches!(rev.len(), 40 | 64) || !rev.bytes().all(|b| b.is_ascii_hexdigit()) {
            bail!("`{rev}` is not a full commit hash (expected 40 or 64 hex digits)");
        }
        Ok(Self(rev.to_ascii_lowercase()))
    }
}

impl fmt::Display for GitRev {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Checks a repository URL. Only https and ssh are accepted: git's other transports
/// include `ext::`, which runs arbitrary commands, and `file://`, which reads outside the
/// project. Whitespace and control characters are refused since URLs reach CI logs.
pub fn check_git_url(url: &str) -> Result<()> {
    if url.chars().any(|c| c.is_whitespace() || c.is_control()) {
        bail!("the URL {url:?} contains whitespace or control characters");
    }
    if !(url.starts_with("https://") || url.starts_with("ssh://") || is_scp_like(url)) {
        bail!("`{url}` is not a git URL gdget accepts: use https://, ssh://, or USER@HOST:PATH");
    }
    Ok(())
}

/// Whether `url` is git's scp-style SSH syntax, `USER@HOST:PATH`, as in
/// `git@github.com:owner/repo.git`.
pub fn is_scp_like(url: &str) -> bool {
    let Some((head, _)) = url.split_once(':') else {
        return false;
    };
    let Some((user, host)) = head.split_once('@') else {
        return false;
    };
    !user.is_empty()
        && !host.is_empty()
        && !head.contains('/')
        && !user.starts_with('-')
        && !host.starts_with('-')
}

/// Checks a branch, tag or commit name before it is passed to git.
pub fn check_ref(reference: &str) -> Result<()> {
    if reference.is_empty()
        || reference.starts_with('-')
        || reference
            .chars()
            .any(|c| c.is_whitespace() || c.is_control())
    {
        bail!("`{reference}` is not a valid git branch, tag or commit");
    }
    Ok(())
}

/// Variables git sets while running hooks, such as the post-checkout hook that runs
/// `gdget sync`. Inherited, they would point these commands at the game's repository.
const REPO_ENV: &[&str] = &[
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_IMPLICIT_WORK_TREE",
    "GIT_INDEX_FILE",
    "GIT_OBJECT_DIRECTORY",
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    "GIT_COMMON_DIR",
    "GIT_GRAFT_FILE",
    "GIT_SHALLOW_FILE",
    "GIT_NAMESPACE",
    "GIT_PREFIX",
    "GIT_QUARANTINE_PATH",
];

/// The protocols git may use unless the user sets `GIT_ALLOW_PROTOCOL` themselves.
const ALLOWED_PROTOCOLS: &str = "https:ssh";

/// Git's documented spelling of an unlimited `--depth`, which also works on a repository
/// that is not shallow, unlike `--unshallow`.
const FULL_DEPTH: &str = "--depth=2147483647";

/// Fetches commits into per-remote bare repositories in the cache with the system `git`,
/// so the user's credential helpers, SSH setup and proxy settings apply.
pub struct Git {
    cache: Cache,
    github_token: Option<String>,
    /// `None` leaves the user's own `GIT_ALLOW_PROTOCOL` in charge.
    allowed_protocols: Option<String>,
    /// Lets git prompt for credentials; off without a terminal so CI fails instead of
    /// hanging.
    interactive: bool,
    reporter: Reporter,
}

impl Git {
    /// Creates a fetcher that uses `GITHUB_TOKEN` from the environment, if set.
    pub fn new(cache: Cache, reporter: Reporter) -> Self {
        Self {
            cache,
            github_token: std::env::var("GITHUB_TOKEN").ok().filter(|t| !t.is_empty()),
            allowed_protocols: std::env::var_os("GIT_ALLOW_PROTOCOL")
                .is_none()
                .then(|| ALLOWED_PROTOCOLS.to_owned()),
            interactive: std::io::stdin().is_terminal(),
            reporter,
        }
    }

    pub fn with_github_token(mut self, token: Option<String>) -> Self {
        self.github_token = token;
        self
    }

    pub fn with_allowed_protocols(mut self, protocols: &str) -> Self {
        self.allowed_protocols = Some(protocols.to_owned());
        self
    }

    /// Fetches `reference` (a branch, tag or full commit hash, or the remote's default
    /// branch if `None`) and returns the commit it names.
    pub fn resolve(&self, url: &str, reference: Option<&str>) -> Result<GitRev> {
        let repo = self.repo(url)?;
        let wanted = reference.unwrap_or("HEAD");
        self.reporter.action("Fetching", format!("{url} {wanted}"));
        let rev = self
            .fetch(&repo, url, "--depth=1", wanted)
            .with_context(|| format!("cannot fetch `{wanted}` from {url}"))?;
        self.keep(&repo, &rev)?;
        Ok(rev)
    }

    /// Makes sure commit `rev` from `url` is cached. `reference` is fetched with its full
    /// history as a fallback, for servers that refuse to send a commit by its hash.
    pub fn fetch_pinned(&self, url: &str, rev: &GitRev, reference: Option<&str>) -> Result<()> {
        let repo = self.repo(url)?;
        if self.has_commit(&repo, rev) {
            return Ok(());
        }
        self.reporter.action("Fetching", format!("{url} {rev}"));
        if self.fetch(&repo, url, "--depth=1", rev.as_str()).is_err() {
            let fallback = reference.unwrap_or("HEAD");
            self.fetch(&repo, url, FULL_DEPTH, fallback)
                .with_context(|| format!("cannot fetch commit {rev} from {url}"))?;
            if !self.has_commit(&repo, rev) {
                bail!(
                    "cannot fetch commit {rev} from {url}: it is not in the history of `{fallback}`"
                );
            }
        }
        self.keep(&repo, rev)
    }

    /// Writes the files of cached commit `rev` as a zip, which is deleted when the
    /// returned path is dropped.
    pub fn export(&self, url: &str, rev: &GitRev) -> Result<TempPath> {
        let repo = self.cache.git_repo_path(url);
        let temp_dir = self.cache.temp_dir();
        std::fs::create_dir_all(&temp_dir)
            .with_context(|| format!("cannot create {}", temp_dir.display()))?;
        let temp = NamedTempFile::new_in(&temp_dir)
            .with_context(|| format!("cannot create a file in {}", temp_dir.display()))?;
        let out = temp
            .as_file()
            .try_clone()
            .with_context(|| format!("cannot write {}", temp.path().display()))?;
        self.run(
            self.command(Some(&repo))
                .args(["archive", "--format=zip", rev.as_str()])
                .stdout(out),
        )
        .with_context(|| format!("cannot export commit {rev} of {url}"))?;
        Ok(temp.into_temp_path())
    }

    /// The cache's bare repository for `url`, created on first use.
    fn repo(&self, url: &str) -> Result<PathBuf> {
        let path = self.cache.git_repo_path(url);
        if path.join("HEAD").is_file() {
            return Ok(path);
        }
        let parent = path.parent().expect("repo paths have a parent");
        std::fs::create_dir_all(parent)
            .with_context(|| format!("cannot create {}", parent.display()))?;
        // Initialized aside and renamed in, so a concurrent gdget never sees half a repo.
        let temp = tempfile::Builder::new()
            .prefix(".init-")
            .tempdir_in(parent)
            .with_context(|| format!("cannot create a folder in {}", parent.display()))?;
        self.run(
            self.command(None)
                .args(["init", "--bare", "--quiet"])
                .arg(temp.path()),
        )
        .context("cannot create the git cache")?;
        match std::fs::rename(temp.path(), &path) {
            Ok(()) => {
                let _ = temp.keep();
            }
            Err(_) if path.join("HEAD").is_file() => {}
            Err(e) => {
                return Err(e).with_context(|| format!("cannot create {}", path.display()));
            }
        }
        Ok(path)
    }

    /// Fetches `wanted` from `url` and returns the commit it names. The fetch lands in a
    /// ref unique to this call, rather than FETCH_HEAD, so concurrent runs can't mix up
    /// their results.
    fn fetch(&self, repo: &Path, url: &str, depth: &str, wanted: &str) -> Result<GitRev> {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let incoming = format!(
            "refs/gdget/incoming/{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        );
        let mut command = self.command(Some(repo));
        command
            .args(["fetch", "--no-tags", "--quiet", depth, "--", url])
            .arg(format!("+{wanted}:{incoming}"));
        if let Some(token) = &self.github_token
            && let Some(config) =
                github_token_config(url, token, std::env::var_os("GIT_CONFIG_COUNT").as_deref())
        {
            command.envs(config);
        }
        self.run(&mut command)?;
        let rev = self.run(
            self.command(Some(repo))
                .args(["rev-parse", "--verify", "--quiet"])
                .arg(format!("{incoming}^{{commit}}")),
        );
        let _ = self.run(
            self.command(Some(repo))
                .args(["update-ref", "-d", &incoming]),
        );
        let rev = rev.with_context(|| format!("`{wanted}` is not a commit"))?;
        rev.trim()
            .parse()
            .with_context(|| format!("git printed an unexpected commit for `{wanted}`"))
    }

    fn has_commit(&self, repo: &Path, rev: &GitRev) -> bool {
        self.run(
            self.command(Some(repo))
                .args(["cat-file", "-e"])
                .arg(format!("{rev}^{{commit}}")),
        )
        .is_ok()
    }

    /// Points a ref at `rev` so git's automatic cleanup never deletes it.
    fn keep(&self, repo: &Path, rev: &GitRev) -> Result<()> {
        self.run(self.command(Some(repo)).args([
            "update-ref",
            &format!("refs/gdget/pins/{rev}"),
            rev.as_str(),
        ]))
        .context("cannot update the git cache")?;
        Ok(())
    }

    fn command(&self, repo: Option<&Path>) -> Command {
        let mut command = Command::new("git");
        for var in REPO_ENV {
            command.env_remove(var);
        }
        if let Some(repo) = repo {
            command.arg("--git-dir").arg(repo);
        }
        if let Some(protocols) = &self.allowed_protocols {
            command.env("GIT_ALLOW_PROTOCOL", protocols);
        }
        if !self.interactive {
            command.env("GIT_TERMINAL_PROMPT", "0").stdin(Stdio::null());
        }
        command
    }

    /// Runs `command` and returns its stdout, or an error carrying git's message.
    fn run(&self, command: &mut Command) -> Result<String> {
        let output = command.output().map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                anyhow!("git sources need git, which was not found on PATH")
            } else {
                anyhow::Error::new(e).context("cannot run git")
            }
        })?;
        if !output.status.success() {
            bail!("{}", describe_failure(&output.stderr));
        }
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }
}

/// Git configuration, as environment variables appended after any the user already set,
/// that sends `token` with requests to github.com. It goes through the environment so it
/// never appears in a process listing, and only when `url` is on github.com over https.
fn github_token_config(
    url: &str,
    token: &str,
    existing_count: Option<&std::ffi::OsStr>,
) -> Option<Vec<(OsString, OsString)>> {
    let host = url.strip_prefix("https://")?.split('/').next()?;
    if !host.eq_ignore_ascii_case("github.com") {
        return None;
    }
    let count: usize = match existing_count {
        None => 0,
        Some(count) => count.to_str()?.parse().ok()?,
    };
    let credentials = base64(format!("x-access-token:{token}").as_bytes());
    Some(vec![
        ("GIT_CONFIG_COUNT".into(), (count + 1).to_string().into()),
        (
            format!("GIT_CONFIG_KEY_{count}").into(),
            "http.https://github.com/.extraHeader".into(),
        ),
        (
            format!("GIT_CONFIG_VALUE_{count}").into(),
            format!("Authorization: Basic {credentials}").into(),
        ),
    ])
}

fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let bits = chunk
            .iter()
            .enumerate()
            .fold(0u32, |bits, (i, &b)| bits | u32::from(b) << (16 - 8 * i));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(ALPHABET[(bits >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// Git's stderr as one line. The remote controls part of it, so control characters are
/// dropped and lines are joined, keeping it from forging CI workflow commands.
fn describe_failure(stderr: &[u8]) -> String {
    let lines: Vec<String> = String::from_utf8_lossy(stderr)
        .lines()
        .map(|line| {
            line.chars()
                .filter(|c| !c.is_control())
                .collect::<String>()
                .trim()
                .to_owned()
        })
        .filter(|line| !line.is_empty())
        .collect();
    if lines.is_empty() {
        "git failed".to_owned()
    } else {
        lines.join("; ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::archive::Archive;

    fn git(dir: &Path, args: &[&str]) -> String {
        let output = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args([
                "-c",
                "user.name=gdget",
                "-c",
                "user.email=gdget@example.com",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .output()
            .unwrap();
        assert!(output.status.success(), "git {args:?}: {output:?}");
        String::from_utf8(output.stdout).unwrap()
    }

    fn commit(repo: &Path, file: &str, contents: &str) -> GitRev {
        if !repo.exists() {
            std::fs::create_dir_all(repo).unwrap();
            git(repo, &["init", "--quiet", "-b", "main"]);
        }
        std::fs::write(repo.join(file), contents).unwrap();
        git(repo, &["add", "--all"]);
        git(repo, &["commit", "--quiet", "-m", contents]);
        git(repo, &["rev-parse", "HEAD"]).trim().parse().unwrap()
    }

    fn file_url(path: &Path) -> String {
        let path = path.to_str().unwrap().replace('\\', "/");
        if path.starts_with('/') {
            format!("file://{path}")
        } else {
            format!("file:///{path}")
        }
    }

    #[test]
    fn fetches_by_ref_and_by_hash_then_exports_a_zip() {
        let temp = tempfile::tempdir().unwrap();
        let remote = temp.path().join("remote");
        let first = commit(&remote, "a.txt", "one");
        let second = commit(&remote, "a.txt", "two");
        let url = file_url(&remote);
        let cache = Cache::new(temp.path().join("cache"));
        let git = Git::new(cache.clone(), Reporter::from_env())
            .with_github_token(None)
            .with_allowed_protocols("file");

        assert_eq!(git.resolve(&url, None).unwrap(), second);
        assert_eq!(git.resolve(&url, Some("main")).unwrap(), second);
        let err = format!("{:#}", git.resolve(&url, Some("nope")).unwrap_err());
        assert!(err.contains("cannot fetch `nope`"), "{err}");

        git.fetch_pinned(&url, &first, None).unwrap();
        let zip = git.export(&url, &first).unwrap();
        let out = temp.path().join("out");
        Archive::open(&zip)
            .unwrap()
            .extract(&crate::manifest::ArchivePath::root(), &out)
            .unwrap();
        assert_eq!(std::fs::read_to_string(out.join("a.txt")).unwrap(), "one");

        let strict =
            Git::new(cache, Reporter::from_env()).with_allowed_protocols(ALLOWED_PROTOCOLS);
        let err = format!("{:#}", strict.resolve(&url, None).unwrap_err());
        assert!(err.contains("not allowed"), "{err}");
    }

    #[test]
    fn revs_must_be_full_hashes() {
        let rev: GitRev = "AFDE39C3F0E1AFDE39C3F0E1AFDE39C3F0E1AFDE".parse().unwrap();
        assert_eq!(rev.as_str(), "afde39c3f0e1afde39c3f0e1afde39c3f0e1afde");
        assert_eq!(rev.short(), "afde39c3f0e1");
        assert!("a".repeat(64).parse::<GitRev>().is_ok());
        for bad in ["afde39c", "", &"g".repeat(40), &"a".repeat(41)] {
            assert!(bad.parse::<GitRev>().is_err(), "{bad}");
        }
    }

    #[test]
    fn accepts_only_https_and_ssh_urls() {
        for good in [
            "https://github.com/DevPrice/godot-addons.git",
            "ssh://git@github.com/DevPrice/godot-addons.git",
            "git@github.com:DevPrice/godot-addons.git",
        ] {
            assert!(check_git_url(good).is_ok(), "{good}");
        }
        for bad in [
            "http://github.com/o/r.git",
            "file:///tmp/r.git",
            "ext::sh -c touch% /tmp/x",
            "/tmp/r.git",
            "C:/r.git",
            "github.com:o/r.git",
            "-oProxyCommand=x@host:r",
            "git@-host:r",
            "https://github.com/o/r.git\n::error::x",
        ] {
            assert!(check_git_url(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn scp_syntax_needs_a_user_and_no_slash_before_the_colon() {
        assert!(is_scp_like("git@host:o/r"));
        assert!(!is_scp_like("devprice/godot-slang@v1"));
        assert!(!is_scp_like("a/b@c:d"));
        assert!(!is_scp_like("host:o/r"));
    }

    #[test]
    fn refs_cannot_look_like_options() {
        assert!(check_ref("v1.2.0").is_ok());
        assert!(check_ref("feature/x").is_ok());
        for bad in ["", "--upload-pack=x", "a b", "a\nb"] {
            assert!(check_ref(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn base64_pads_partial_groups() {
        assert_eq!(base64(b"Man"), "TWFu");
        assert_eq!(base64(b"Ma"), "TWE=");
        assert_eq!(base64(b"M"), "TQ==");
        assert_eq!(base64(b""), "");
    }

    #[test]
    fn token_goes_only_to_github_and_after_existing_config() {
        let config = github_token_config("https://GitHub.com/o/r.git", "t", None).unwrap();
        assert_eq!(config[0], ("GIT_CONFIG_COUNT".into(), "1".into()));
        assert_eq!(
            config[1],
            (
                "GIT_CONFIG_KEY_0".into(),
                "http.https://github.com/.extraHeader".into()
            )
        );
        assert_eq!(
            config[2].1,
            OsString::from(format!(
                "Authorization: Basic {}",
                base64(b"x-access-token:t")
            ))
        );

        let config =
            github_token_config("https://github.com/o/r", "t", Some("2".as_ref())).unwrap();
        assert_eq!(config[0].1, "3");
        assert_eq!(config[1].0, "GIT_CONFIG_KEY_2");

        for url in [
            "http://github.com/o/r.git",
            "https://github.com.evil.example/o/r.git",
            "https://user@github.com/o/r.git",
            "git@github.com:o/r.git",
            "https://gitlab.com/o/r.git",
        ] {
            assert!(github_token_config(url, "t", None).is_none(), "{url}");
        }
        assert!(github_token_config("https://github.com/o/r", "t", Some("x".as_ref())).is_none());
    }

    #[test]
    fn failures_are_one_line_without_control_characters() {
        assert_eq!(
            describe_failure(b"remote: hi\r\n\n::error::fake\x1b[31m\nfatal: nope\n"),
            "remote: hi; ::error::fake[31m; fatal: nope"
        );
        assert_eq!(describe_failure(b""), "git failed");
    }
}
