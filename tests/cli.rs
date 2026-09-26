use assert_cmd::Command;

fn gdget() -> Command {
    let mut cmd = Command::cargo_bin("gdget").unwrap();
    cmd.env_remove("GITHUB_ACTIONS");
    cmd
}

#[test]
fn usage_error_exits_2() {
    gdget().arg("frobnicate").assert().code(2);
}

#[test]
fn sync_rejects_force_with_check() {
    gdget()
        .args(["sync", "--force", "--check"])
        .assert()
        .code(2);
}

#[test]
fn errors_become_github_annotations_under_actions() {
    let assert = gdget()
        .env("GITHUB_ACTIONS", "true")
        .args(["-C", "does-not-exist", "status"])
        .assert()
        .code(1);
    let stderr = String::from_utf8_lossy(&assert.get_output().stderr).into_owned();
    assert!(stderr.starts_with("::error::cannot change to"), "{stderr}");
}
