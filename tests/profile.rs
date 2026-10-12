mod common;

use common::TestEnv;

const CONFIG: &str = r#"default_profile = "home"

[profiles.home]
url = "https://home.youtrack.cloud"
login = "jane"
auth = "token"

[profiles.work]
url = "https://acme.youtrack.cloud"
auth = "oauth"
"#;

#[test]
fn profile_list_marks_the_active_profile() {
    let env = TestEnv::new();
    env.write_config(CONFIG);
    let output = env.run(&["profile", "list"]);
    assert!(output.status.success(), "{}", common::stderr(&output));
    let text = common::stdout(&output);
    assert!(
        text.lines()
            .any(|line| line.starts_with("home") && line.trim_end().ends_with('*')),
        "{text}"
    );
    assert!(text.contains("https://acme.youtrack.cloud"), "{text}");
}

#[test]
fn yt_profile_selects_the_active_profile() {
    let env = TestEnv::new();
    env.write_config(CONFIG);
    let output = env.run_with(
        &["profile", "ls", "--format", "json"],
        &[("YT_PROFILE", "work")],
    );
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let work = value
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == "work")
        .unwrap();
    assert_eq!(work["active"], true);
}

#[test]
fn profile_use_changes_the_default() {
    let env = TestEnv::new();
    env.write_config(CONFIG);
    let output = env.run(&["profile", "use", "work"]);
    assert!(output.status.success(), "{}", common::stderr(&output));
    assert!(env.read_config().contains("default_profile = \"work\""));
}

#[test]
fn an_unknown_profile_suggests_a_name() {
    let env = TestEnv::new();
    env.write_config(CONFIG);
    let output = env.run(&["profile", "use", "wrok"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(common::stderr(&output).contains("did you mean 'work'?"));
}

#[test]
fn profile_remove_needs_yes_without_a_terminal() {
    let env = TestEnv::new();
    env.write_config(CONFIG);
    let output = env.run(&["profile", "remove", "work"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(common::stderr(&output).contains("refusing to remove profile work without --yes"));
}

#[test]
fn profile_remove_with_yes_removes_it() {
    let env = TestEnv::new();
    env.write_config(CONFIG);
    // `work` has no login, so no keyring entry is touched.
    let output = env.run(&["profile", "remove", "work", "--yes"]);
    assert!(output.status.success(), "{}", common::stderr(&output));
    assert!(!env.read_config().contains("[profiles.work]"));
}

#[test]
fn an_empty_yt_profile_counts_as_unset() {
    let env = TestEnv::new();
    env.write_config(CONFIG);
    let output = env.run_with(
        &["profile", "ls", "--format", "json"],
        &[("YT_PROFILE", "")],
    );
    assert!(output.status.success(), "{}", common::stderr(&output));
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let home = value
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == "home")
        .unwrap();
    assert_eq!(home["active"], true);
}
