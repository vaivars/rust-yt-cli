mod common;

use common::TestEnv;

#[test]
fn version_prints_the_package_version() {
    let output = TestEnv::new().run(&["--version"]);
    assert!(output.status.success());
    assert!(common::stdout(&output).contains(env!("CARGO_PKG_VERSION")));
}

#[test]
fn completion_prints_a_script_that_calls_back_into_yt() {
    let output = TestEnv::new().run(&["completion", "bash"]);
    assert!(output.status.success(), "{}", common::stderr(&output));
    assert!(common::stdout(&output).contains("__complete_word__"));
}

#[test]
fn an_unknown_flag_is_a_usage_error() {
    let output = TestEnv::new().run(&["completion", "bash", "--nope"]);
    assert_eq!(output.status.code(), Some(2));
}

#[test]
fn the_spec_lists_command_effects() {
    let output = TestEnv::new().run(&["__usage_spec__"]);
    assert!(output.status.success());
    assert!(common::stdout(&output).contains("cmd completion"));
}

#[test]
fn profile_names_complete() {
    let env = TestEnv::new();
    env.write_config("[profiles.acme]\nurl = \"https://acme.youtrack.cloud\"\nauth = \"token\"\n\n[profiles.home]\nurl = \"https://home.youtrack.cloud\"\nauth = \"token\"\n");
    for line in ["yt --profile ", "yt profile use "] {
        let output = env.run(&["__complete_word__", "--shell", "fish", "--line", line]);
        let text = common::stdout(&output);
        assert!(
            text.contains("acme") && text.contains("home"),
            "{line}: {text}"
        );
    }
}

#[test]
fn a_closed_stderr_does_not_turn_an_error_into_a_panic() {
    let env = TestEnv::new();
    let mut child = env
        .command()
        .args(["issue", "list", "-n", "0"])
        .env("YOUTRACK_URL", "https://acme.youtrack.cloud")
        .env("YOUTRACK_TOKEN", "perm-Y2xp.dGVzdA==.secret123456")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    drop(child.stderr.take());
    assert_eq!(child.wait().unwrap().code(), Some(2));
}
