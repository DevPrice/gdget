mod support;

use gdget::link::is_link;
use gdget::manifest::{OVERRIDES_FILE, Overrides};
use support::{Fixture, run};

fn overrides(fx: &Fixture) -> String {
    std::fs::read_to_string(fx.root().join(OVERRIDES_FILE)).unwrap()
}

#[test]
fn link_names_the_addon_from_a_build_folder_outside_the_project() {
    let fx = Fixture::new();
    let repo = tempfile::tempdir().unwrap();
    let addon = repo.path().join("addons").join("verse");
    std::fs::create_dir_all(&addon).unwrap();
    std::fs::write(addon.join("verse.gdextension"), "dev build").unwrap();

    let out = run(fx.gdget().arg("link").arg(repo.path()));
    assert_eq!(out.code, 0, "{out:?}");
    assert!(out.stdout.contains("Linked verse"), "{out:?}");
    assert!(is_link(&fx.addon("verse")));
    assert_eq!(fx.read("addons/verse/verse.gdextension"), "dev build");

    let text = overrides(&fx);
    let dir = &Overrides::parse(&text).unwrap().addons[&"verse".parse().unwrap()];
    let dir = dir.to_str().unwrap();
    assert!(dir.starts_with("../") && !dir.contains('\\'), "{text}");
    assert!(!fx.root().join("addons.toml").exists());

    let out = run(fx.gdget().arg("sync"));
    assert_eq!(out.code, 0, "{out:?}");
    assert!(out.stdout.contains("0 changed, 1 up to date"), "{out:?}");
}

#[test]
fn link_replaces_the_pinned_copy_and_unlink_restores_it() {
    let fx = Fixture::new();
    fx.write_manifest(&[&fx.publish("a", "a.zip", &[("addons/a/plugin.cfg", "pinned")])]);
    assert_eq!(run(fx.gdget().arg("sync")).code, 0);
    fx.write("dev/a/plugin.cfg", "dev build");

    let out = run(fx.gdget().args(["link", "dev/a"]));
    assert_eq!(out.code, 0, "{out:?}");
    assert!(out.stdout.contains("Overrode a with dev/a"), "{out:?}");
    assert!(is_link(&fx.addon("a")));
    assert_eq!(fx.read("addons/a/plugin.cfg"), "dev build");

    let out = run(fx.gdget().args(["unlink", "a"]));
    assert_eq!(out.code, 0, "{out:?}");
    assert!(out.stdout.contains("Updated a"), "{out:?}");
    assert!(!is_link(&fx.addon("a")));
    assert_eq!(fx.read("addons/a/plugin.cfg"), "pinned");
    assert_eq!(fx.read("dev/a/plugin.cfg"), "dev build");
    assert!(!overrides(&fx).contains("dev/a"));
}

#[test]
fn unlink_removes_an_override_only_addon() {
    let fx = Fixture::new();
    fx.write("dev/a/plugin.cfg", "dev build");
    assert_eq!(run(fx.gdget().args(["link", "dev/a"])).code, 0);

    let out = run(fx.gdget().args(["unlink", "a"]));
    assert_eq!(out.code, 0, "{out:?}");
    assert!(out.stdout.contains("Removed a"), "{out:?}");
    assert!(!fx.addon("a").exists());
    assert_eq!(fx.read("dev/a/plugin.cfg"), "dev build");

    let out = run(fx.gdget().args(["unlink", "a"]));
    assert_eq!(out.code, 1, "{out:?}");
    assert!(out.stderr.contains("`a` has no override"), "{out:?}");
}

#[test]
fn sync_without_a_manifest_cleans_up_after_a_deleted_overrides_file() {
    let fx = Fixture::new();
    fx.write("dev/a/plugin.cfg", "dev build");
    assert_eq!(run(fx.gdget().args(["link", "dev/a"])).code, 0);
    std::fs::remove_file(fx.root().join(OVERRIDES_FILE)).unwrap();

    let out = run(fx.gdget().arg("sync"));
    assert_eq!(out.code, 0, "{out:?}");
    assert!(out.stdout.contains("Removed a"), "{out:?}");
    assert!(!fx.addon("a").exists());

    let out = run(fx.gdget().arg("sync"));
    assert_eq!(out.code, 1, "{out:?}");
    assert!(out.stderr.contains("no addons.toml"), "{out:?}");
}

#[test]
fn link_paths_are_relative_to_where_it_runs() {
    let fx = Fixture::new();
    fx.write("dev/a/plugin.cfg", "dev build");
    std::fs::create_dir(fx.root().join("scenes")).unwrap();

    let out = run(fx
        .gdget()
        .current_dir(fx.root().join("scenes"))
        .args(["link", "renamed", "../dev/a"]));
    assert_eq!(out.code, 0, "{out:?}");
    assert!(
        overrides(&fx).contains("renamed = \"dev/a\""),
        "{}",
        overrides(&fx)
    );
    assert_eq!(fx.read("addons/renamed/plugin.cfg"), "dev build");
}

#[test]
fn distant_paths_are_recorded_as_absolute() {
    let fx = Fixture::new();
    fx.write("dev/a/plugin.cfg", "dev build");
    fx.write("w/x/y/game/project.godot", "");
    let game = fx.root().join("w").join("x").join("y").join("game");

    let out = run(fx
        .gdget()
        .current_dir(&game)
        .args(["link", "../../../../dev/a"]));
    assert_eq!(out.code, 0, "{out:?}");
    let text = std::fs::read_to_string(game.join(OVERRIDES_FILE)).unwrap();
    let dir = &Overrides::parse(&text).unwrap().addons[&"a".parse().unwrap()];
    assert!(dir.is_absolute(), "{text}");
    assert!(same_file(dir, &fx.root().join("dev").join("a")), "{text}");
}

#[cfg(unix)]
#[test]
fn paths_through_symlinks_are_still_relative() {
    let fx = Fixture::new();
    fx.write("dev/a/plugin.cfg", "dev build");
    let aliases = tempfile::tempdir().unwrap();
    let alias = aliases.path().join("project");
    std::os::unix::fs::symlink(fx.root(), &alias).unwrap();

    let out = run(fx.gdget().arg("link").arg(alias.join("dev").join("a")));
    assert_eq!(out.code, 0, "{out:?}");
    assert!(
        overrides(&fx).contains("a = \"dev/a\""),
        "{}",
        overrides(&fx)
    );
}

fn same_file(a: &std::path::Path, b: &std::path::Path) -> bool {
    a.canonicalize().unwrap() == b.canonicalize().unwrap()
}

#[test]
fn relinking_updates_the_entry_and_keeps_comments() {
    let fx = Fixture::new();
    fx.write("dev/a/plugin.cfg", "one");
    fx.write("dev2/a/plugin.cfg", "two");
    fx.write(OVERRIDES_FILE, "# my builds\n[overrides]\na = \"dev/a\"\n");
    assert_eq!(run(fx.gdget().arg("sync")).code, 0);

    let out = run(fx.gdget().args(["link", "dev2/a"]));
    assert_eq!(out.code, 0, "{out:?}");
    assert_eq!(fx.read("addons/a/plugin.cfg"), "two");
    let text = overrides(&fx);
    assert!(text.starts_with("# my builds\n"), "{text}");
    assert_eq!(Overrides::parse(&text).unwrap().addons.len(), 1, "{text}");
}

#[test]
fn bad_paths_change_nothing() {
    let fx = Fixture::new();
    let out = run(fx.gdget().args(["link", "missing"]));
    assert_eq!(out.code, 1, "{out:?}");
    assert!(out.stderr.contains("is not a directory"), "{out:?}");

    fx.write("dev/notes.txt", "");
    let out = run(fx.gdget().args(["link", "dev"]));
    assert_eq!(out.code, 1, "{out:?}");
    assert!(out.stderr.contains("gdget link NAME PATH"), "{out:?}");

    let out = run(fx.gdget().args(["link", "a", "dev"]));
    assert_eq!(out.code, 1, "{out:?}");
    assert!(
        out.stderr.contains("cannot find the addon folder"),
        "{out:?}"
    );
    assert!(!fx.root().join(OVERRIDES_FILE).exists());
}

#[test]
fn link_refuses_to_replace_an_unowned_folder() {
    let fx = Fixture::new();
    fx.write("addons/a/plugin.cfg", "mine");
    fx.write("dev/a/plugin.cfg", "dev build");

    let out = run(fx.gdget().args(["link", "dev/a"]));
    assert_eq!(out.code, 1, "{out:?}");
    assert!(out.stderr.contains("not installed by gdget"), "{out:?}");
    assert_eq!(fx.read("addons/a/plugin.cfg"), "mine");
    assert!(!fx.root().join(OVERRIDES_FILE).exists());
}
