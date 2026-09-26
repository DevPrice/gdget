#![allow(dead_code, reason = "each tests/*.rs crate uses a different subset")]

use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use gdget::digest::Sha256;

/// Builds a zip archive in memory from `(path, contents)` pairs.
pub(crate) fn zip_bytes(files: &[(&str, &str)]) -> Vec<u8> {
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (path, contents) in files {
        zip.start_file(*path, zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(contents.as_bytes()).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

/// A Godot project, a private archive cache, a fixture server and git remotes for one
/// test.
pub(crate) struct Fixture {
    pub project: tempfile::TempDir,
    pub cache: tempfile::TempDir,
    pub server: TestServer,
    pub remotes: GitRemotes,
}

impl Fixture {
    pub(crate) fn new() -> Self {
        let project = tempfile::tempdir().unwrap();
        std::fs::write(project.path().join("project.godot"), "").unwrap();
        Self {
            project,
            cache: tempfile::tempdir().unwrap(),
            server: TestServer::start(),
            remotes: GitRemotes::new(),
        }
    }

    pub(crate) fn root(&self) -> &Path {
        self.project.path()
    }

    pub(crate) fn addon(&self, name: &str) -> PathBuf {
        self.root().join("addons").join(name)
    }

    /// Serves a zip of `files` at `/<file>` and returns a manifest entry pinning it.
    pub(crate) fn publish(&self, name: &str, file: &str, files: &[(&str, &str)]) -> String {
        let bytes = zip_bytes(files);
        let sha256 = Sha256::of_bytes(&bytes);
        let url = self.server.serve(&format!("/{file}"), bytes);
        format!("[addons.{name}]\nurl = \"{url}\"\nsha256 = \"{sha256}\"\n")
    }

    pub(crate) fn write_manifest(&self, entries: &[&str]) {
        std::fs::write(self.root().join("addons.toml"), entries.join("\n")).unwrap();
    }

    pub(crate) fn write(&self, relative: &str, contents: &str) {
        let path = self.root().join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, contents).unwrap();
    }

    pub(crate) fn read(&self, relative: &str) -> String {
        std::fs::read_to_string(self.root().join(relative)).unwrap()
    }

    pub(crate) fn gdget(&self) -> assert_cmd::Command {
        let mut cmd = assert_cmd::Command::cargo_bin("gdget").unwrap();
        cmd.current_dir(self.root())
            .env("GDGET_CACHE_DIR", self.cache.path())
            .env("GDGET_STORE_URL", self.server.url(""))
            .env("NO_COLOR", "1")
            .env_remove("GITHUB_ACTIONS")
            .env_remove("GITHUB_TOKEN")
            .envs(self.remotes.env());
        cmd
    }
}

/// Local repositories that git reaches at `https://example.test/<name>.git` through an
/// `insteadOf` rewrite to `file://`, so tests never touch the network.
pub(crate) struct GitRemotes {
    dir: tempfile::TempDir,
}

impl GitRemotes {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("empty.gitconfig"), "").unwrap();
        Self { dir }
    }

    pub(crate) fn url(&self, repo: &str) -> String {
        format!("https://example.test/{repo}.git")
    }

    pub(crate) fn path(&self, repo: &str) -> PathBuf {
        self.dir.path().join(format!("{repo}.git"))
    }

    /// Writes `files` into `repo`, creating it on first use, commits everything and
    /// returns the new commit's hash.
    pub(crate) fn commit(&self, repo: &str, files: &[(&str, &str)]) -> String {
        let path = self.path(repo);
        if !path.exists() {
            std::fs::create_dir_all(&path).unwrap();
            self.git(repo, &["init", "--quiet", "-b", "main"]);
        }
        for (file, contents) in files {
            let file = path.join(file);
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(file, contents).unwrap();
        }
        self.git(repo, &["add", "--all"]);
        self.git(
            repo,
            &["commit", "--quiet", "--allow-empty", "-m", "commit"],
        );
        self.git(repo, &["rev-parse", "HEAD"]).trim().to_owned()
    }

    pub(crate) fn git(&self, repo: &str, args: &[&str]) -> String {
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(self.path(repo))
            .args([
                "-c",
                "user.name=gdget",
                "-c",
                "user.email=gdget@example.com",
            ])
            .args(args)
            .envs(self.env())
            .output()
            .unwrap();
        assert!(output.status.success(), "git {args:?}: {output:?}");
        String::from_utf8(output.stdout).unwrap()
    }

    /// Environment that isolates git from the user's configuration and routes
    /// `https://example.test/` here.
    fn env(&self) -> Vec<(String, String)> {
        let dir = self.dir.path().to_str().unwrap().replace('\\', "/");
        let file_url = if dir.starts_with('/') {
            format!("file://{dir}/")
        } else {
            format!("file:///{dir}/")
        };
        let global = self.dir.path().join("empty.gitconfig");
        [
            ("GIT_ALLOW_PROTOCOL", "file"),
            ("GIT_CONFIG_NOSYSTEM", "1"),
            ("GIT_CONFIG_GLOBAL", global.to_str().unwrap()),
            ("GIT_CONFIG_COUNT", "1"),
            ("GIT_CONFIG_KEY_0", &format!("url.{file_url}.insteadOf")),
            ("GIT_CONFIG_VALUE_0", "https://example.test/"),
        ]
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value.to_owned()))
        .collect()
    }
}

/// Output of a finished command, for asserting on text.
pub(crate) struct Run {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

pub(crate) fn run(cmd: &mut assert_cmd::Command) -> Run {
    let output = cmd.output().unwrap();
    Run {
        code: output.status.code().unwrap(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

impl std::fmt::Debug for Run {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "exit {}\n--- stdout\n{}--- stderr\n{}",
            self.code, self.stdout, self.stderr
        )
    }
}

#[derive(Debug, Clone)]
pub(crate) struct RecordedRequest {
    pub path: String,
    pub authorization: Option<String>,
}

/// A local HTTP server for fixture archives, so tests never touch the network.
pub(crate) struct TestServer {
    server: Arc<tiny_http::Server>,
    base: String,
    routes: Arc<Mutex<HashMap<String, Vec<u8>>>>,
    requests: Arc<Mutex<Vec<RecordedRequest>>>,
    thread: Option<JoinHandle<()>>,
}

impl TestServer {
    pub(crate) fn start() -> Self {
        let server = Arc::new(tiny_http::Server::http("127.0.0.1:0").unwrap());
        let base = format!("http://{}", server.server_addr().to_ip().unwrap());
        let routes = Arc::new(Mutex::new(HashMap::<String, Vec<u8>>::new()));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let thread = {
            let server = Arc::clone(&server);
            let routes = Arc::clone(&routes);
            let requests = Arc::clone(&requests);
            std::thread::spawn(move || {
                for request in server.incoming_requests() {
                    let path = request.url().to_owned();
                    let authorization = request
                        .headers()
                        .iter()
                        .find(|h| h.field.equiv("Authorization"))
                        .map(|h| h.value.to_string());
                    requests.lock().unwrap().push(RecordedRequest {
                        path: path.clone(),
                        authorization,
                    });
                    let body = routes.lock().unwrap().get(&path).cloned();
                    let _ = match body {
                        Some(body) => request.respond(tiny_http::Response::from_data(body)),
                        None => request.respond(tiny_http::Response::empty(404)),
                    };
                }
            })
        };
        Self {
            server,
            base,
            routes,
            requests,
            thread: Some(thread),
        }
    }

    /// Serves `body` at `path` (e.g. "/slang.zip") and returns its full URL.
    pub(crate) fn serve(&self, path: &str, body: impl Into<Vec<u8>>) -> String {
        self.routes
            .lock()
            .unwrap()
            .insert(path.to_owned(), body.into());
        self.url(path)
    }

    pub(crate) fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base)
    }

    pub(crate) fn requests(&self) -> Vec<RecordedRequest> {
        self.requests.lock().unwrap().clone()
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.server.unblock();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
