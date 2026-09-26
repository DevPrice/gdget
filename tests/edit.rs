mod support;

use gdget::digest::Sha256;
use gdget::manifest::{Manifest, Source};
use support::{Fixture, run, zip_bytes};

fn manifest(fx: &Fixture) -> Manifest {
    Manifest::load(&fx.root().join("addons.toml")).unwrap()
}

#[test]
fn add_pins_the_hash_and_detected_path_then_installs() {
    let fx = Fixture::new();
    let bytes = zip_bytes(&[("slang-v1/addons/slang/slang.gdextension", "[configuration]")]);
    let sha256 = Sha256::of_bytes(&bytes);
    let url = fx.server.serve("/slang.zip", bytes);

    let out = run(fx
        .gdget()
        .args(["add", "slang", &url, "--version-label", "1.0.0"]));
    assert_eq!(out.code, 0, "{out:?}");
    assert!(out.stdout.contains("Pinned slang 1.0.0"), "{out:?}");
    assert!(out.stdout.contains("Installed slang"), "{out:?}");
    assert_eq!(fx.read("addons/slang/slang.gdextension"), "[configuration]");

    let addon = &manifest(&fx).addons[&"slang".parse().unwrap()];
    assert_eq!(addon.version.as_deref(), Some("1.0.0"));
    assert_eq!(addon.source, Source::Url { url, sha256 });
    assert_eq!(
        addon.path.as_ref().unwrap().to_string(),
        "slang-v1/addons/slang"
    );
    assert_eq!(
        fx.server.requests().len(),
        1,
        "install reuses the downloaded archive"
    );
}

#[test]
fn add_repins_an_existing_entry_and_keeps_others() {
    let fx = Fixture::new();
    let other = fx.publish("other", "o.zip", &[("addons/other/plugin.cfg", "o")]);
    let old = fx.publish("a", "a1.zip", &[("addons/a/plugin.cfg", "1")]);
    fx.write_manifest(&["# my addons\n", &other, &old]);
    assert_eq!(run(fx.gdget().arg("sync")).code, 0);

    let url = fx
        .server
        .serve("/a2.zip", zip_bytes(&[("addons/a/plugin.cfg", "2")]));
    let out = run(fx.gdget().args(["add", "a", &url]));
    assert_eq!(out.code, 0, "{out:?}");
    assert!(out.stdout.contains("Updated a"), "{out:?}");
    assert_eq!(fx.read("addons/a/plugin.cfg"), "2");
    assert!(fx.read("addons.toml").starts_with("# my addons"));
    assert_eq!(manifest(&fx).addons.len(), 2);
}

#[test]
fn add_with_ambiguous_layout_changes_nothing_until_path_is_given() {
    let fx = Fixture::new();
    let url = fx.server.serve(
        "/pack.zip",
        zip_bytes(&[("addons/x/plugin.cfg", "x"), ("addons/y/plugin.cfg", "y")]),
    );

    let out = run(fx.gdget().args(["add", "y2", &url]));
    assert_eq!(out.code, 1, "{out:?}");
    assert!(out.stderr.contains("cannot tell which folder"), "{out:?}");
    assert!(!fx.root().join("addons.toml").exists());

    let out = run(fx.gdget().args(["add", "y2", &url, "--path", "addons/y"]));
    assert_eq!(out.code, 0, "{out:?}");
    assert_eq!(fx.read("addons/y2/plugin.cfg"), "y");
}

#[test]
fn add_refuses_to_replace_an_unowned_folder() {
    let fx = Fixture::new();
    fx.write("addons/a/plugin.cfg", "mine");
    let url = fx
        .server
        .serve("/a.zip", zip_bytes(&[("addons/a/plugin.cfg", "theirs")]));

    let out = run(fx.gdget().args(["add", "a", &url]));
    assert_eq!(out.code, 1, "{out:?}");
    assert!(out.stderr.contains("not installed by gdget"), "{out:?}");
    assert_eq!(fx.server.requests().len(), 0);
    assert_eq!(fx.read("addons/a/plugin.cfg"), "mine");
}

#[test]
fn add_rejects_bad_names_and_urls() {
    let fx = Fixture::new();
    let out = run(fx
        .gdget()
        .args(["add", "../evil", "https://example.com/a.zip"]));
    assert_eq!(out.code, 1, "{out:?}");
    assert!(out.stderr.contains("invalid addon name"), "{out:?}");

    let out = run(fx.gdget().args(["add", "a", "file:///a.zip"]));
    assert_eq!(out.code, 1, "{out:?}");
    assert!(out.stderr.contains("not an http(s) URL"), "{out:?}");
}

#[test]
fn remove_unpins_and_uninstalls_only_that_addon() {
    let fx = Fixture::new();
    let a = fx.publish("a", "a.zip", &[("addons/a/plugin.cfg", "a")]);
    let b = fx.publish("b", "b.zip", &[("addons/b/plugin.cfg", "b")]);
    fx.write_manifest(&[&a, &b]);
    assert_eq!(run(fx.gdget().arg("sync")).code, 0);

    let out = run(fx.gdget().args(["remove", "a"]));
    assert_eq!(out.code, 0, "{out:?}");
    assert!(out.stdout.contains("Removed a"), "{out:?}");
    assert!(!fx.addon("a").exists());
    assert!(fx.addon("b").exists());
    let names: Vec<String> = manifest(&fx)
        .addons
        .keys()
        .map(ToString::to_string)
        .collect();
    assert_eq!(names, ["b"]);

    let out = run(fx.gdget().args(["remove", "a"]));
    assert_eq!(out.code, 1, "{out:?}");
    assert!(out.stderr.contains("`a` is not in addons.toml"), "{out:?}");
}

#[test]
fn remove_keeps_a_modified_copy_until_forced() {
    let fx = Fixture::new();
    fx.write_manifest(&[&fx.publish("a", "a.zip", &[("addons/a/plugin.cfg", "a")])]);
    assert_eq!(run(fx.gdget().arg("sync")).code, 0);
    fx.write("addons/a/plugin.cfg", "edited");

    let out = run(fx.gdget().args(["remove", "a"]));
    assert_eq!(out.code, 1, "{out:?}");
    assert!(out.stderr.contains("has local changes"), "{out:?}");
    assert_eq!(fx.read("addons/a/plugin.cfg"), "edited");

    let out = run(fx.gdget().args(["sync", "--force"]));
    assert_eq!(out.code, 0, "{out:?}");
    assert!(!fx.addon("a").exists());
}
