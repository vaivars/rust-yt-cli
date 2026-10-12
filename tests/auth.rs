mod common;

use common::TestEnv;

#[test]
fn token_stdin_needs_a_url() {
    let output = TestEnv::new().run(&["auth", "login", "--token-stdin"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(common::stderr(&output).contains("pass <URL>"));
}

use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const TOKEN: &str = "perm-c3RhdHVz.dGVzdA==.secret123456";

#[tokio::test(flavor = "multi_thread")]
async fn status_checks_the_credential() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/users/me"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({"id": "1-1", "login": "jane", "name": "Jane"})),
        )
        .mount(&server)
        .await;
    let url = server.uri();
    let output = TestEnv::new()
        .run_async(
            &["auth", "status"],
            &[("YOUTRACK_URL", url.as_str()), ("YOUTRACK_TOKEN", TOKEN)],
        )
        .await;
    assert!(output.status.success(), "{}", common::stderr(&output));
    let text = common::stdout(&output);
    assert!(text.contains("YOUTRACK_URL and YOUTRACK_TOKEN"), "{text}");
    assert!(text.contains("ok (signed in as jane)"), "{text}");
    assert!(!text.contains(TOKEN));
}

#[test]
fn status_offline_does_not_contact_the_server() {
    let output = TestEnv::new().run_with(
        &["auth", "status", "--offline", "--format", "json"],
        &[
            ("YOUTRACK_URL", "https://unreachable.invalid"),
            ("YOUTRACK_TOKEN", TOKEN),
        ],
    );
    assert!(output.status.success(), "{}", common::stderr(&output));
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["profile"], "YOUTRACK_URL");
    assert_eq!(value["check"], serde_json::Value::Null);
}

#[test]
fn logout_refuses_environment_credentials() {
    let output = TestEnv::new().run_with(
        &["auth", "logout", "--yes"],
        &[
            ("YOUTRACK_URL", "https://acme.youtrack.cloud"),
            ("YOUTRACK_TOKEN", TOKEN),
        ],
    );
    assert_eq!(output.status.code(), Some(2));
    assert!(common::stderr(&output).contains("unset them"));
}

#[test]
fn logout_of_a_profile_without_a_login_is_a_no_op() {
    let env = TestEnv::new();
    env.write_config("[profiles.acme]\nurl = \"https://acme.youtrack.cloud\"\nauth = \"oauth\"\n");
    let output = env.run(&["auth", "logout", "--yes"]);
    assert!(output.status.success(), "{}", common::stderr(&output));
    assert!(common::stderr(&output).contains("Not signed in to acme"));
}
