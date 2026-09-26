mod support;

use support::{Fixture, run};

fn entry(name: &str, url: &str, rev: &str, extra: &str) -> String {
    format!("[addons.{name}]\ngit = \"{url}\"\nrev = \"{rev}\"\n{extra}")
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
