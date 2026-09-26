mod support;

use gdget::link::is_link;
use support::{Fixture, run};

const PLUGIN: (&str, &str) = ("addons/a/plugin.cfg", "[plugin]\nname=\"a\"\n");

#[test]
fn fresh_install() {
    let fx = Fixture::new();
    let entry = fx.publish(
        "a",
        "a-1.zip",
        &[PLUGIN, ("addons/a/a.gd", "v1"), ("README.md", "x")],
    );
    fx.write_manifest(&[&entry]);

    let out = run(fx.gdget().arg("sync"));
    assert_eq!(out.code, 0, "{out:?}");
    assert!(out.stdout.contains("Installed a"), "{out:?}");
    assert_eq!(fx.read("addons/a/a.gd"), "v1");
    assert!(fx.addon("a").join(".gdget.toml").is_file());
    assert!(!fx.root().join("addons").join("README.md").exists());
}

#[test]
fn resync_is_a_no_op_without_network() {
    let fx = Fixture::new();
    fx.write_manifest(&[&fx.publish("a", "a.zip", &[PLUGIN])]);
    assert_eq!(run(fx.gdget().arg("sync")).code, 0);

    let out = run(fx.gdget().arg("sync"));
    assert_eq!(out.code, 0, "{out:?}");
    assert!(out.stdout.contains("0 changed, 1 up to date"), "{out:?}");
    assert_eq!(fx.server.requests().len(), 1);
}

#[test]
fn reinstall_after_deleting_uses_the_cache() {
    let fx = Fixture::new();
    fx.write_manifest(&[&fx.publish("a", "a.zip", &[PLUGIN])]);
    assert_eq!(run(fx.gdget().arg("sync")).code, 0);
    std::fs::remove_dir_all(fx.addon("a")).unwrap();

    let out = run(fx.gdget().arg("sync"));
    assert_eq!(out.code, 0, "{out:?}");
    assert!(out.stdout.contains("Installed a"), "{out:?}");
    assert_eq!(fx.server.requests().len(), 1);
}

#[test]
fn hash_mismatch_fails_before_touching_addons() {
    let fx = Fixture::new();
    let entry = fx.publish("a", "a.zip", &[PLUGIN]);
    let wrong = format!("sha256 = \"{}\"", "0".repeat(64));
    let entry = replace_sha(&entry, &wrong);
    fx.write_manifest(&[&entry]);

    let out = run(fx.gdget().arg("sync"));
    assert_eq!(out.code, 1, "{out:?}");
    assert!(out.stderr.contains("sha256 mismatch"), "{out:?}");
    assert!(!fx.addon("a").exists());
}

#[test]
fn update_replaces_the_pinned_version() {
    let fx = Fixture::new();
    fx.write_manifest(&[&fx.publish("a", "a-1.zip", &[PLUGIN, ("addons/a/a.gd", "v1")])]);
    assert_eq!(run(fx.gdget().arg("sync")).code, 0);
    fx.write_manifest(&[&fx.publish("a", "a-2.zip", &[PLUGIN, ("addons/a/b.gd", "v2")])]);

    let out = run(fx.gdget().arg("sync"));
    assert_eq!(out.code, 0, "{out:?}");
    assert!(out.stdout.contains("Updated a"), "{out:?}");
    assert_eq!(fx.read("addons/a/b.gd"), "v2");
    assert!(!fx.addon("a").join("a.gd").exists());
}

#[test]
fn removing_from_manifest_uninstalls_only_what_gdget_installed() {
    let fx = Fixture::new();
    let a = fx.publish("a", "a.zip", &[PLUGIN]);
    let b = fx.publish("b", "b.zip", &[("addons/b/plugin.cfg", "b")]);
    fx.write_manifest(&[&a, &b]);
    fx.write("addons/mine/plugin.cfg", "hand-made");
    assert_eq!(run(fx.gdget().arg("sync")).code, 0);

    fx.write_manifest(&[&a]);
    let out = run(fx.gdget().arg("sync"));
    assert_eq!(out.code, 0, "{out:?}");
    assert!(out.stdout.contains("Removed b"), "{out:?}");
    assert!(!fx.addon("b").exists());
    assert!(fx.addon("a").exists());
    assert_eq!(fx.read("addons/mine/plugin.cfg"), "hand-made");
}

#[test]
fn local_override_links_and_unlinks() {
    let fx = Fixture::new();
    fx.write_manifest(&[&fx.publish("a", "a.zip", &[PLUGIN, ("addons/a/a.gd", "pinned")])]);
    assert_eq!(run(fx.gdget().arg("sync")).code, 0);
    fx.write("dev/a/plugin.cfg", "[plugin]");
    fx.write("dev/a/a.gd", "dev build");
    fx.write("addons.local.toml", "[overrides]\na = \"dev/a\"\n");

    let out = run(fx.gdget().arg("sync"));
    assert_eq!(out.code, 0, "{out:?}");
    assert!(out.stdout.contains("Linked a"), "{out:?}");
    assert!(is_link(&fx.addon("a")));
    assert_eq!(fx.read("addons/a/a.gd"), "dev build");

    let out = run(fx.gdget().arg("sync"));
    assert!(out.stdout.contains("0 changed, 1 up to date"), "{out:?}");

    std::fs::remove_file(fx.root().join("addons.local.toml")).unwrap();
    let out = run(fx.gdget().arg("sync"));
    assert_eq!(out.code, 0, "{out:?}");
    assert!(out.stdout.contains("Updated a"), "{out:?}");
    assert!(!is_link(&fx.addon("a")));
    assert_eq!(fx.read("addons/a/a.gd"), "pinned");
    assert_eq!(fx.read("dev/a/a.gd"), "dev build");
}

#[test]
fn override_can_point_at_a_build_output_root() {
    let fx = Fixture::new();
    fx.write_manifest(&[&fx.publish("a", "a.zip", &[PLUGIN])]);
    fx.write("build/addons/a/a.gdextension", "[configuration]");
    fx.write("addons.local.toml", "[overrides]\na = \"build\"\n");

    let out = run(fx.gdget().arg("sync"));
    assert_eq!(out.code, 0, "{out:?}");
    assert_eq!(fx.read("addons/a/a.gdextension"), "[configuration]");
    assert_eq!(
        fx.server.requests().len(),
        0,
        "an override needs no download"
    );
}

#[test]
fn locally_modified_addon_is_kept_until_forced() {
    let fx = Fixture::new();
    fx.write_manifest(&[&fx.publish("a", "a-1.zip", &[PLUGIN, ("addons/a/a.gd", "v1")])]);
    assert_eq!(run(fx.gdget().arg("sync")).code, 0);
    fx.write("addons/a/a.gd", "my fix");
    fx.write("addons/a/a.gd.uid", "uid://godot-made");
    fx.write_manifest(&[&fx.publish("a", "a-2.zip", &[PLUGIN, ("addons/a/a.gd", "v2")])]);

    let out = run(fx.gdget().arg("sync"));
    assert_eq!(out.code, 1, "{out:?}");
    assert!(out.stderr.contains("addons/a has local changes"), "{out:?}");
    assert!(out.stderr.contains("modified: a.gd"), "{out:?}");
    assert!(!out.stderr.contains(".uid"), "{out:?}");
    assert_eq!(fx.read("addons/a/a.gd"), "my fix");

    let out = run(fx.gdget().args(["sync", "--force"]));
    assert_eq!(out.code, 0, "{out:?}");
    assert!(out.stderr.contains("discarding local changes"), "{out:?}");
    assert_eq!(fx.read("addons/a/a.gd"), "v2");
}

#[test]
fn ambiguous_archive_layout_asks_for_path() {
    let fx = Fixture::new();
    let entry = fx.publish(
        "c",
        "c.zip",
        &[("addons/x/plugin.cfg", "x"), ("addons/y/plugin.cfg", "y")],
    );
    fx.write_manifest(&[&entry]);

    let out = run(fx.gdget().arg("sync"));
    assert_eq!(out.code, 1, "{out:?}");
    assert!(out.stderr.contains("cannot tell which folder"), "{out:?}");
    assert!(!fx.addon("c").exists());

    fx.write_manifest(&[&format!("{entry}path = \"addons/y\"\n")]);
    let out = run(fx.gdget().arg("sync"));
    assert_eq!(out.code, 0, "{out:?}");
    assert_eq!(fx.read("addons/c/plugin.cfg"), "y");
}

#[test]
fn unowned_folder_in_the_way_is_left_alone() {
    let fx = Fixture::new();
    fx.write_manifest(&[&fx.publish("a", "a.zip", &[PLUGIN])]);
    fx.write("addons/a/plugin.cfg", "vendored by hand");

    let out = run(fx.gdget().arg("sync"));
    assert_eq!(out.code, 1, "{out:?}");
    assert!(out.stderr.contains("not installed by gdget"), "{out:?}");
    assert_eq!(fx.read("addons/a/plugin.cfg"), "vendored by hand");
}

#[test]
fn check_reports_drift_without_changing_anything() {
    let fx = Fixture::new();
    fx.write_manifest(&[&fx.publish("a", "a.zip", &[PLUGIN])]);

    let out = run(fx.gdget().args(["sync", "--check"]));
    assert_eq!(out.code, 1, "{out:?}");
    assert!(out.stdout.contains("Missing a"), "{out:?}");
    assert!(!fx.addon("a").exists());
    assert_eq!(fx.server.requests().len(), 0);

    assert_eq!(run(fx.gdget().arg("sync")).code, 0);
    let out = run(fx.gdget().args(["sync", "--check"]));
    assert_eq!(out.code, 0, "{out:?}");
}

#[test]
fn warns_when_installed_addons_are_not_gitignored() {
    let fx = Fixture::new();
    let git = std::process::Command::new("git")
        .args(["init", "-q"])
        .current_dir(fx.root())
        .status();
    if !git.is_ok_and(|status| status.success()) {
        eprintln!("git is unavailable; skipping");
        return;
    }
    fx.write_manifest(&[&fx.publish("a", "a.zip", &[PLUGIN])]);

    let out = run(fx.gdget().arg("sync"));
    assert_eq!(out.code, 0, "{out:?}");
    assert!(
        out.stderr.contains("addons/a/ is not ignored by git"),
        "{out:?}"
    );
    assert!(
        out.stderr.contains(".gdget/ is not ignored by git"),
        "{out:?}"
    );

    fx.write(".gitignore", "/addons/a/\n/.gdget/\n");
    let out = run(fx.gdget().arg("sync"));
    assert!(!out.stderr.contains("not ignored"), "{out:?}");
}

#[test]
fn refuses_an_addons_folder_that_is_a_link() {
    let fx = Fixture::new();
    fx.write_manifest(&[&fx.publish("confd", "a.zip", &[("plugin.cfg", "payload")])]);
    let outside = tempfile::tempdir().unwrap();
    gdget::link::create(outside.path(), &fx.root().join("addons")).unwrap();

    let out = run(fx.gdget().arg("sync"));
    assert_eq!(out.code, 1, "{out:?}");
    assert!(out.stderr.contains("is a link"), "{out:?}");
    assert!(!outside.path().join("confd").exists());
    gdget::link::remove(&fx.root().join("addons")).unwrap();
}

#[test]
fn missing_manifest_is_an_error() {
    let fx = Fixture::new();
    let out = run(fx.gdget().arg("sync"));
    assert_eq!(out.code, 1, "{out:?}");
    assert!(out.stderr.contains("no addons.toml"), "{out:?}");
}

fn replace_sha(entry: &str, replacement: &str) -> String {
    entry
        .lines()
        .map(|line| {
            if line.starts_with("sha256 = ") {
                replacement
            } else {
                line
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}
