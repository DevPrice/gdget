mod support;

use gdget::digest::Sha256;
use gdget::fetch::{Cache, Fetcher};
use gdget::report::Reporter;
use support::TestServer;

fn fetcher(cache_dir: &std::path::Path) -> Fetcher {
    Fetcher::new(Cache::new(cache_dir.to_owned()), Reporter::from_env()).with_github_token(None)
}

fn cached_archives(cache_dir: &std::path::Path) -> usize {
    std::fs::read_dir(cache_dir.join("sha256")).map_or(0, |entries| entries.count())
}

#[test]
fn downloads_verifies_and_reuses_the_cache() {
    let server = TestServer::start();
    let url = server.serve("/a.zip", b"archive bytes".to_vec());
    let cache = tempfile::tempdir().unwrap();
    let expected = Sha256::of_bytes(b"archive bytes");

    let path = fetcher(cache.path()).fetch_pinned(&url, &expected).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), b"archive bytes");

    let again = fetcher(cache.path()).fetch_pinned(&url, &expected).unwrap();
    assert_eq!(again, path);
    assert_eq!(
        server.requests().len(),
        1,
        "second fetch must hit the cache"
    );
}

#[test]
fn hash_mismatch_is_an_error_and_nothing_is_cached() {
    let server = TestServer::start();
    let url = server.serve("/a.zip", b"tampered".to_vec());
    let cache = tempfile::tempdir().unwrap();
    let pinned = Sha256::of_bytes(b"original");

    let err = fetcher(cache.path())
        .fetch_pinned(&url, &pinned)
        .unwrap_err();
    let message = format!("{err:#}");
    assert!(message.contains("sha256 mismatch"), "{message}");
    assert!(message.contains(&pinned.to_string()), "{message}");
    assert!(
        message.contains(&Sha256::of_bytes(b"tampered").to_string()),
        "{message}"
    );
    assert_eq!(cached_archives(cache.path()), 0);
}

#[test]
fn corrupt_cache_entry_is_downloaded_again() {
    let server = TestServer::start();
    let url = server.serve("/a.zip", b"good".to_vec());
    let cache = tempfile::tempdir().unwrap();
    let expected = Sha256::of_bytes(b"good");
    let fetcher = fetcher(cache.path());
    let cached = fetcher.cache().archive_path(&expected);
    std::fs::create_dir_all(cached.parent().unwrap()).unwrap();
    std::fs::write(&cached, b"bit rot").unwrap();

    let path = fetcher.fetch_pinned(&url, &expected).unwrap();
    assert_eq!(std::fs::read(path).unwrap(), b"good");
    assert_eq!(server.requests().len(), 1);
}

#[test]
fn unpinned_fetch_reports_the_hash() {
    let server = TestServer::start();
    let url = server.serve("/a.zip", b"new addon".to_vec());
    let cache = tempfile::tempdir().unwrap();

    let (sha256, path) = fetcher(cache.path()).fetch_unpinned(&url).unwrap();
    assert_eq!(sha256, Sha256::of_bytes(b"new addon"));
    assert_eq!(
        path,
        Cache::new(cache.path().to_owned()).archive_path(&sha256)
    );
}

#[test]
fn http_errors_name_the_url() {
    let server = TestServer::start();
    let url = server.url("/missing.zip");
    let cache = tempfile::tempdir().unwrap();

    let err = fetcher(cache.path()).fetch_unpinned(&url).unwrap_err();
    let message = format!("{err:#}");
    assert!(message.contains(&url), "{message}");
    assert!(message.contains("404"), "{message}");
}

#[test]
fn token_is_not_sent_to_other_hosts() {
    let server = TestServer::start();
    let url = server.serve("/a.zip", b"x".to_vec());
    let cache = tempfile::tempdir().unwrap();

    fetcher(cache.path())
        .with_github_token(Some("secret".into()))
        .fetch_unpinned(&url)
        .unwrap();
    assert_eq!(server.requests()[0].authorization, None);
}
