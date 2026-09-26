mod support;

use support::{Fixture, run};

fn line_for<'a>(stdout: &'a str, name: &str) -> &'a str {
    stdout
        .lines()
        .find(|line| line.split_whitespace().next() == Some(name))
        .unwrap_or_else(|| panic!("no line for {name} in:\n{stdout}"))
}

#[test]
fn reports_each_addon_state() {
    let fx = Fixture::new();
    let current = fx.publish("current", "c.zip", &[("addons/current/plugin.cfg", "c")]);
    let outdated_v1 = fx.publish("outdated", "o1.zip", &[("addons/outdated/plugin.cfg", "1")]);
    let edited = fx.publish("edited", "e.zip", &[("addons/edited/a.gd", "orig")]);
    let dropped = fx.publish("dropped", "d.zip", &[("addons/dropped/plugin.cfg", "d")]);
    fx.write_manifest(&[&current, &outdated_v1, &edited, &dropped]);
    assert_eq!(run(fx.gdget().arg("sync")).code, 0);

    let outdated_v2 = fx.publish("outdated", "o2.zip", &[("addons/outdated/plugin.cfg", "2")]);
    let missing = fx.publish("missing", "m.zip", &[("addons/missing/plugin.cfg", "m")]);
    fx.write_manifest(&[
        &format!("{current}version = \"1.2.0\"\n"),
        &outdated_v2,
        &edited,
        &missing,
    ]);
    fx.write("addons/edited/a.gd", "changed");
    fx.write("addons/handmade/plugin.cfg", "h");
    fx.write("dev/linked/plugin.cfg", "l");
    fx.write(
        "addons.local.toml",
        "[overrides]\nlinked = \"dev/linked\"\n",
    );

    let out = run(fx.gdget().arg("status"));
    assert_eq!(out.code, 0, "{out:?}");
    let stdout = &out.stdout;
    assert!(line_for(stdout, "current").contains("1.2.0 ("), "{stdout}");
    assert!(
        line_for(stdout, "current").ends_with("installed"),
        "{stdout}"
    );
    assert!(
        line_for(stdout, "outdated").contains("outdated"),
        "{stdout}"
    );
    assert!(
        line_for(stdout, "edited").contains("modified: a.gd"),
        "{stdout}"
    );
    assert!(line_for(stdout, "missing").contains("missing"), "{stdout}");
    assert!(
        line_for(stdout, "linked").contains("override not applied"),
        "{stdout}"
    );
    assert!(
        line_for(stdout, "dropped").contains("sync removes it"),
        "{stdout}"
    );
    assert!(
        line_for(stdout, "handmade").contains("not managed"),
        "{stdout}"
    );

    run(fx.gdget().arg("sync"));
    let out = run(fx.gdget().arg("status"));
    assert!(
        line_for(&out.stdout, "linked").contains("overridden ->"),
        "{out:?}"
    );
}

#[test]
fn works_without_a_manifest() {
    let fx = Fixture::new();
    let out = run(fx.gdget().arg("status"));
    assert_eq!(out.code, 0, "{out:?}");
    assert!(out.stdout.contains("no addons"), "{out:?}");
}
