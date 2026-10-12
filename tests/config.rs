mod common;

use common::TestEnv;

#[test]
fn config_list_shows_defaults() {
    let output = TestEnv::new().run(&["config", "list"]);
    assert!(output.status.success(), "{}", common::stderr(&output));
    let text = common::stdout(&output);
    assert!(text.contains("http_retries = 3"), "{text}");
    assert!(
        text.contains("http_timeout = \"30s\"") || text.contains("http_timeout = 30s"),
        "{text}"
    );
}

#[test]
fn config_list_reflects_the_environment_and_the_file() {
    let env = TestEnv::new();
    env.write_config("[settings]\nhttp_timeout = \"10s\"\n");
    let output = env.run_with(&["config", "ls"], &[("YT_HTTP_RETRIES", "9")]);
    let text = common::stdout(&output);
    assert!(text.contains("http_retries = 9"), "{text}");
    assert!(text.contains("10s"), "{text}");
}

#[test]
fn config_list_as_json() {
    let output = TestEnv::new().run(&["config", "list", "--format", "json"]);
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["http_retries"], "3");
}

#[test]
fn config_get_names_the_source() {
    let output = TestEnv::new().run_with(
        &["config", "get", "http_retries"],
        &[("YT_HTTP_RETRIES", "4")],
    );
    assert!(output.status.success(), "{}", common::stderr(&output));
    assert!(common::stdout(&output).contains("YT_HTTP_RETRIES"));
}

#[test]
fn config_get_of_an_unknown_setting_is_a_usage_error() {
    let output = TestEnv::new().run(&["config", "get", "nope"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(common::stderr(&output).contains("unknown setting 'nope'"));
}

#[test]
fn config_set_writes_the_file() {
    let env = TestEnv::new();
    let output = env.run(&["config", "set", "http_retries", "5"]);
    assert!(output.status.success(), "{}", common::stderr(&output));
    assert!(env.read_config().contains("http_retries = 5"));
    let output = env.run(&["config", "get", "http_retries"]);
    assert!(common::stdout(&output).contains('5'));
}

#[test]
fn config_set_rejects_invalid_values() {
    let output = TestEnv::new().run(&["config", "set", "color", "sometimes"]);
    assert_eq!(output.status.code(), Some(2));
}

#[test]
fn empty_environment_values_count_as_unset() {
    let output = TestEnv::new().run_with(
        &["config", "list"],
        &[("YT_FORMAT", ""), ("YT_NO_INPUT", "")],
    );
    assert!(output.status.success(), "{}", common::stderr(&output));
    assert!(common::stdout(&output).contains("http_timeout"));
}

#[test]
fn yt_format_still_selects_the_format() {
    let output = TestEnv::new().run_with(&["config", "list"], &[("YT_FORMAT", "json")]);
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["http_timeout"], "30s");
}
