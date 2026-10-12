mod common;

use std::io::Write as _;
use std::process::Stdio;

use common::TestEnv;
use serde_json::json;
use wiremock::matchers::{body_json, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

const TOKEN: &str = "perm-YXBp.dGVzdA==.secret123456";

async fn run(env: &TestEnv, args: &[&str], server: &MockServer) -> std::process::Output {
    let url = server.uri();
    env.run_async(
        args,
        &[("YOUTRACK_URL", url.as_str()), ("YOUTRACK_TOKEN", TOKEN)],
    )
    .await
}

#[tokio::test(flavor = "multi_thread")]
async fn get_passes_the_path_and_fields_through() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/admin/projects"))
        .and(query_param("$top", "2"))
        .and(query_param("fields", "shortName"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!([{"shortName": "WEB", "$type": "Project"}])),
        )
        .expect(1)
        .mount(&server)
        .await;
    let output = run(
        &TestEnv::new(),
        &["api", "admin/projects?$top=2", "--fields", "shortName"],
        &server,
    )
    .await;
    assert!(output.status.success(), "{}", common::stderr(&output));
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value[0]["shortName"], "WEB");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_response_keeps_youtracks_key_order() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/issues/WEB-1"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            r#"{"idReadable":"WEB-1","$type":"Issue","tags":[],"summary":"a, b: {c}"}"#,
            "application/json",
        ))
        .mount(&server)
        .await;
    let output = run(
        &TestEnv::new(),
        &["api", "issues/WEB-1", "--format", "ndjson"],
        &server,
    )
    .await;
    assert!(output.status.success(), "{}", common::stderr(&output));
    assert_eq!(
        common::stdout(&output),
        "{\n  \"idReadable\": \"WEB-1\",\n  \"$type\": \"Issue\",\n  \"tags\": [],\n  \"summary\": \"a, b: {c}\"\n}\n"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn post_sends_the_body_from_stdin_once() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/commands"))
        .and(body_json(
            json!({"query": "State Open", "issues": [{"idReadable": "WEB-1"}]}),
        ))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&server)
        .await;
    let env = TestEnv::new();
    let mut command = env.command();
    command
        .args(["api", "commands", "-X", "POST", "--input", "-"])
        .env("YOUTRACK_URL", server.uri())
        .env("YOUTRACK_TOKEN", TOKEN)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let output = tokio::task::spawn_blocking(move || {
        let mut child = command.spawn().unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(br#"{"query": "State Open", "issues": [{"idReadable": "WEB-1"}]}"#)
            .unwrap();
        child.wait_with_output().unwrap()
    })
    .await
    .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stdout.is_empty());
}

#[test]
fn invalid_input_json_is_a_usage_error() {
    let env = TestEnv::new();
    let file = env.path().join("body.json");
    std::fs::write(&file, "{not json").unwrap();
    let output = env.run_with(
        &[
            "api",
            "commands",
            "-X",
            "post",
            "--input",
            file.to_str().unwrap(),
        ],
        &[
            ("YOUTRACK_URL", "https://acme.youtrack.cloud"),
            ("YOUTRACK_TOKEN", TOKEN),
        ],
    );
    assert_eq!(output.status.code(), Some(2));
}

#[tokio::test(flavor = "multi_thread")]
async fn delete_needs_yes_without_a_terminal() {
    let server = MockServer::start().await;
    Mock::given(method("DELETE"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&server)
        .await;
    let output = run(
        &TestEnv::new(),
        &["api", "-X", "DELETE", "issues/WEB-1"],
        &server,
    )
    .await;
    assert_eq!(output.status.code(), Some(2));
    assert!(common::stderr(&output).contains("--yes"));
}

#[tokio::test(flavor = "multi_thread")]
async fn delete_with_yes_is_sent() {
    let server = MockServer::start().await;
    Mock::given(method("DELETE"))
        .and(path("/api/issues/WEB-1"))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&server)
        .await;
    let output = run(
        &TestEnv::new(),
        &["api", "-X", "DELETE", "issues/WEB-1", "--yes"],
        &server,
    )
    .await;
    assert!(output.status.success(), "{}", common::stderr(&output));
}

#[tokio::test(flavor = "multi_thread")]
async fn control_characters_in_the_response_are_escaped() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/admin/projects"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_raw("[{\"name\":\"a\u{9b}b\u{7f}\"}]", "application/json"),
        )
        .mount(&server)
        .await;
    let output = run(&TestEnv::new(), &["api", "admin/projects"], &server).await;
    assert!(output.status.success(), "{}", common::stderr(&output));
    let text = common::stdout(&output);
    assert!(
        !text.chars().any(|c| c.is_control() && c != '\n'),
        "{text:?}"
    );
    let value: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(value[0]["name"], "a\u{9b}b\u{7f}");
}
