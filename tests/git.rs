mod support;

use gdget::manifest::{Addon, Manifest, Source};
use support::{Fixture, run};

fn entry(name: &str, url: &str, rev: &str, extra: &str) -> String {
    format!("[addons.{name}]\ngit = \"{url}\"\nrev = \"{rev}\"\n{extra}")
}

fn pinned(fx: &Fixture, name: &str) -> Addon {
    Manifest::load(&fx.root().join("addons.toml"))
        .unwrap()
        .addons[&name.parse().unwrap()]
        .clone()
}

fn git_source(url: &str, reference: Option<&str>, rev: &str) -> Source {
    Source::Git {
        url: url.to_owned(),
        reference: reference.map(str::to_owned),
        rev: rev.parse().unwrap(),
    }
}

#[test]
fn add_pins_one_folder_of_a_repo_named_after_the_folder() {
    let fx = Fixture::new();
    let rev = fx.remotes.commit(
        "godot-addons",
        &[
            ("inventory/inventory.gd", "inventory"),
            ("utils/utils.gd", "utils"),
        ],
    );
    let url = fx.remotes.url("godot-addons");

    let out = run(fx.gdget().args(["add", &url, "--path", "inventory"]));
    assert_eq!(out.code, 0, "{out:?}");
    assert!(
        out.stdout
            .contains(&format!("Pinned inventory ({})", &rev[..12])),
        "{out:?}"
    );
    assert_eq!(out.stdout.matches("Fetching").count(), 1, "{out:?}");
    assert_eq!(fx.read("addons/inventory/inventory.gd"), "inventory");
    assert!(!fx.addon("utils").exists());

    let addon = pinned(&fx, "inventory");
    assert_eq!(addon.source, git_source(&url, None, &rev));
    assert_eq!(addon.path.unwrap().to_string(), "inventory");
}

#[test]
fn add_records_the_ref_and_readding_moves_the_pin() {
    let fx = Fixture::new();
    let url = fx.remotes.url("slang");
    let tagged = fx
        .remotes
        .commit("slang", &[("addons/slang/plugin.cfg", "1")]);
    fx.remotes.git("slang", &["tag", "-a", "v1", "-m", "v1"]);
    let head = fx
        .remotes
        .commit("slang", &[("addons/slang/plugin.cfg", "2")]);

    let out = run(fx.gdget().args(["add", &url, "--ref", "v1"]));
    assert_eq!(out.code, 0, "{out:?}");
    assert_eq!(fx.read("addons/slang/plugin.cfg"), "1");
    let addon = pinned(&fx, "slang");
    assert_eq!(addon.source, git_source(&url, Some("v1"), &tagged));
    assert_eq!(addon.path.unwrap().to_string(), "addons/slang");

    let out = run(fx.gdget().args(["add", "slang", &url, "--ref", "main"]));
    assert_eq!(out.code, 0, "{out:?}");
    assert!(
        out.stdout
            .contains(&format!("Updated slang (main@{})", &head[..12])),
        "{out:?}"
    );
    assert_eq!(fx.read("addons/slang/plugin.cfg"), "2");
    assert_eq!(
        pinned(&fx, "slang").source,
        git_source(&url, Some("main"), &head)
    );
}

#[test]
fn add_installs_a_flat_repo_under_the_repo_name() {
    let fx = Fixture::new();
    let rev = fx
        .remotes
        .commit("message_bus", &[("message_bus.gd", "bus")]);

    let out = run(fx.gdget().args(["add", &fx.remotes.url("message_bus")]));
    assert_eq!(out.code, 0, "{out:?}");
    assert_eq!(fx.read("addons/message_bus/message_bus.gd"), "bus");
    let addon = pinned(&fx, "message_bus");
    assert_eq!(addon.path.unwrap().to_string(), ".");
    assert_eq!(
        addon.source,
        git_source(&fx.remotes.url("message_bus"), None, &rev)
    );
}

#[test]
fn git_flag_treats_a_url_without_dot_git_as_a_repository() {
    let fx = Fixture::new();
    fx.remotes.commit("plain", &[("plugin.cfg", "p")]);
    let url = "https://example.test/plain";

    let out = run(fx.gdget().args(["add", "plain", url, "--git"]));
    assert_eq!(out.code, 0, "{out:?}");
    assert_eq!(fx.read("addons/plain/plugin.cfg"), "p");
    assert!(matches!(pinned(&fx, "plain").source, Source::Git { .. }));
}

#[test]
fn add_of_an_unknown_ref_changes_nothing() {
    let fx = Fixture::new();
    fx.remotes.commit("a", &[("plugin.cfg", "a")]);

    let out = run(fx
        .gdget()
        .args(["add", &fx.remotes.url("a"), "--ref", "nope"]));
    assert_eq!(out.code, 1, "{out:?}");
    assert!(out.stderr.contains("cannot fetch `nope`"), "{out:?}");
    assert!(!fx.root().join("addons.toml").exists());
}

#[test]
fn ref_without_a_repository_is_a_usage_error() {
    let fx = Fixture::new();
    let out = run(fx
        .gdget()
        .args(["add", "https://example.test/a.zip", "--ref", "main"]));
    assert_eq!(out.code, 2, "{out:?}");
    assert!(out.stderr.contains("--ref applies only to git"), "{out:?}");
}

#[test]
fn sync_installs_the_pinned_commit_and_reinstalls_from_the_cache() {
    let fx = Fixture::new();
    let rev = fx
        .remotes
        .commit("slang", &[("plugin.cfg", "[plugin]"), ("slang.gd", "one")]);
    fx.remotes.commit("slang", &[("slang.gd", "two")]);
    fx.write_manifest(&[&entry("slang", &fx.remotes.url("slang"), &rev, "")]);

    let out = run(fx.gdget().arg("sync"));
    assert_eq!(out.code, 0, "{out:?}");
    assert!(
        out.stdout
            .contains(&format!("Installed slang ({})", &rev[..12])),
        "{out:?}"
    );
    assert_eq!(fx.read("addons/slang/slang.gd"), "one");
    assert!(
        fx.read("addons/slang/.gdget.toml")
            .contains(&format!("rev = \"{rev}\"")),
    );

    let out = run(fx.gdget().arg("sync"));
    assert!(out.stdout.contains("1 up to date"), "{out:?}");

    std::fs::remove_dir_all(fx.addon("slang")).unwrap();
    std::fs::rename(fx.remotes.path("slang"), fx.remotes.path("gone")).unwrap();
    let out = run(fx.gdget().arg("sync"));
    assert_eq!(out.code, 0, "{out:?}");
    assert!(!out.stdout.contains("Fetching"), "{out:?}");
    assert_eq!(fx.read("addons/slang/slang.gd"), "one");
}

#[test]
fn a_flat_gdscript_repo_installs_its_root() {
    let fx = Fixture::new();
    let rev = fx.remotes.commit(
        "message_bus",
        &[
            ("message_bus.gd", "bus"),
            ("message_bus.gd.uid", "uid://bus"),
        ],
    );
    fx.write_manifest(&[&entry(
        "message_bus",
        &fx.remotes.url("message_bus"),
        &rev,
        "",
    )]);

    let out = run(fx.gdget().arg("sync"));
    assert_eq!(out.code, 0, "{out:?}");
    assert_eq!(fx.read("addons/message_bus/message_bus.gd"), "bus");
}

#[test]
fn folders_of_one_repo_install_separately_from_one_fetch() {
    let fx = Fixture::new();
    let rev = fx.remotes.commit(
        "godot-addons",
        &[
            ("README.md", "readme"),
            ("inventory/inventory.gd", "inventory"),
            ("inventory/inventory.gd.uid", "uid://inventory"),
            ("utils/utils.gd", "utils"),
        ],
    );
    let url = fx.remotes.url("godot-addons");
    fx.write_manifest(&[
        &entry("inventory", &url, &rev, "path = \"inventory\"\n"),
        &entry("utils", &url, &rev, "path = \"utils\"\n"),
    ]);

    let out = run(fx.gdget().arg("sync"));
    assert_eq!(out.code, 0, "{out:?}");
    assert_eq!(out.stdout.matches("Fetching").count(), 1, "{out:?}");
    assert_eq!(fx.read("addons/inventory/inventory.gd"), "inventory");
    assert_eq!(
        fx.read("addons/inventory/inventory.gd.uid"),
        "uid://inventory"
    );
    assert_eq!(fx.read("addons/utils/utils.gd"), "utils");
    assert!(!fx.root().join("addons/README.md").exists());

    let out = run(fx.gdget().arg("status"));
    assert_eq!(out.code, 0, "{out:?}");
    assert_eq!(out.stdout.matches("installed").count(), 2, "{out:?}");
}

#[test]
fn moving_the_pin_updates_unless_the_install_was_modified() {
    let fx = Fixture::new();
    let url = fx.remotes.url("a");
    let first = fx.remotes.commit("a", &[("plugin.cfg", "1")]);
    fx.write_manifest(&[&entry("a", &url, &first, "ref = \"main\"\n")]);
    assert_eq!(run(fx.gdget().arg("sync")).code, 0);

    let second = fx.remotes.commit("a", &[("plugin.cfg", "2")]);
    fx.write_manifest(&[&entry("a", &url, &second, "ref = \"main\"\n")]);
    fx.write("addons/a/plugin.cfg", "mine");
    let out = run(fx.gdget().arg("sync"));
    assert_eq!(out.code, 1, "{out:?}");
    assert!(out.stderr.contains("local changes"), "{out:?}");
    assert_eq!(fx.read("addons/a/plugin.cfg"), "mine");

    let out = run(fx.gdget().args(["sync", "--force"]));
    assert_eq!(out.code, 0, "{out:?}");
    assert!(
        out.stdout
            .contains(&format!("Updated a (main@{})", &second[..12])),
        "{out:?}"
    );
    assert_eq!(fx.read("addons/a/plugin.cfg"), "2");
}

#[test]
fn check_reports_a_missing_git_addon_without_fetching() {
    let fx = Fixture::new();
    let rev = fx.remotes.commit("a", &[("plugin.cfg", "1")]);
    fx.write_manifest(&[&entry("a", &fx.remotes.url("a"), &rev, "")]);

    let out = run(fx.gdget().args(["sync", "--check"]));
    assert_eq!(out.code, 1, "{out:?}");
    assert!(out.stdout.contains("Missing a"), "{out:?}");
    assert!(!out.stdout.contains("Fetching"), "{out:?}");
    assert!(!fx.addon("a").exists());
}

#[test]
fn a_commit_the_remote_lacks_fails_before_touching_addons() {
    let fx = Fixture::new();
    let good = fx.remotes.commit("a", &[("plugin.cfg", "a")]);
    let url = fx.remotes.url("a");
    fx.write_manifest(&[
        &entry("a", &url, &good, ""),
        &entry("b", &url, &"0".repeat(40), ""),
    ]);

    let out = run(fx.gdget().arg("sync"));
    assert_eq!(out.code, 1, "{out:?}");
    assert!(out.stderr.contains("cannot fetch `b`"), "{out:?}");
    assert!(
        out.stderr.contains("not in the history of `HEAD`"),
        "{out:?}"
    );
    assert!(!fx.addon("a").exists());
}
