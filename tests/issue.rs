mod common;

use common::TestEnv;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const TOKEN: &str = "perm-aXNzdWU=.dGVzdA==.secret123456";

async fn server_with_issue() -> MockServer {
    let server = MockServer::start().await;
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/issue-show.json")).unwrap();
    Mock::given(method("GET"))
        .and(path("/api/issues/WEB-42"))
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture))
        .mount(&server)
        .await;
    server
}

fn env_profile(server: &MockServer) -> Vec<(&'static str, String)> {
    vec![
        ("YOUTRACK_URL", server.uri()),
        ("YOUTRACK_TOKEN", TOKEN.to_string()),
    ]
}

async fn run(env: &TestEnv, args: &[&str], server: &MockServer) -> std::process::Output {
    let vars = env_profile(server);
    let vars: Vec<(&str, &str)> = vars
        .iter()
        .map(|(key, value)| (*key, value.as_str()))
        .collect();
    env.run_async(args, &vars).await
}

#[tokio::test(flavor = "multi_thread")]
async fn issue_show_prints_fields_and_the_description() {
    let server = server_with_issue().await;
    let output = run(&TestEnv::new(), &["issue", "show", "WEB-42"], &server).await;
    assert!(output.status.success(), "{}", common::stderr(&output));
    let text = common::stdout(&output);
    assert!(text.starts_with("WEB-42  Login fails with SSO\n"), "{text}");
    assert!(
        text.contains("State") && text.contains("In Progress"),
        "{text}"
    );
    assert!(
        text.contains("Reporter") && text.contains("jane (Jane Doe)"),
        "{text}"
    );
    assert!(
        !text.contains("Fix versions"),
        "empty fields are left out: {text}"
    );
    assert!(text.trim_end().ends_with("2. Choose SSO"), "{text}");
}

#[tokio::test(flavor = "multi_thread")]
async fn issue_show_accepts_an_issue_url() {
    let server = server_with_issue().await;
    let url = format!("{}/issue/WEB-42/login-fails", server.uri());
    let output = run(&TestEnv::new(), &["issue", "show", &url], &server).await;
    assert!(output.status.success(), "{}", common::stderr(&output));
}

#[tokio::test(flavor = "multi_thread")]
async fn issue_show_as_json() {
    let server = server_with_issue().await;
    let output = run(
        &TestEnv::new(),
        &["issue", "show", "WEB-42", "--format", "json"],
        &server,
    )
    .await;
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    insta::assert_json_snapshot!(value);
}

#[test]
fn issue_show_web_prints_the_url_when_no_browser_can_open() {
    let output = TestEnv::new().run_with(
        &["issue", "show", "WEB-42", "--web"],
        &[
            ("YOUTRACK_URL", "https://acme.youtrack.cloud"),
            ("YOUTRACK_TOKEN", TOKEN),
            ("SSH_TTY", "/dev/ttys001"),
        ],
    );
    assert!(output.status.success(), "{}", common::stderr(&output));
    assert_eq!(
        common::stdout(&output),
        "https://acme.youtrack.cloud/issue/WEB-42\n"
    );
}

#[test]
fn a_profile_without_a_login_is_not_signed_in() {
    let env = TestEnv::new();
    env.write_config("[profiles.acme]\nurl = \"https://acme.youtrack.cloud\"\nauth = \"oauth\"\n");
    let output = env.run(&["issue", "show", "WEB-42"]);
    assert_eq!(output.status.code(), Some(4));
    assert!(common::stderr(&output).contains("not signed in to acme"));
}

use std::io::BufRead as _;
use std::process::Stdio;
use std::time::Duration;

use serde_json::{Value, json};
use wiremock::matchers::query_param;

fn row(n: usize) -> Value {
    json!({
        "idReadable": format!("WEB-{n}"),
        "summary": format!("Issue number {n} with a long enough summary to fill the pipe quickly"),
        "project": {"shortName": "WEB", "name": "Web Site"},
        "created": 1_791_579_736_619_i64,
        "updated": 1_791_579_736_619_i64,
        "resolved": null,
        "customFields": []
    })
}

#[tokio::test(flavor = "multi_thread")]
async fn issue_list_joins_query_words_and_defaults_to_fifty() {
    let server = MockServer::start().await;
    let fixture: Value = serde_json::from_str(include_str!("fixtures/issue-list.json")).unwrap();
    Mock::given(method("GET"))
        .and(path("/api/issues"))
        .and(query_param("query", "for: me #Unresolved"))
        .and(query_param("$top", "50"))
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture))
        .expect(1)
        .mount(&server)
        .await;
    let output = run(
        &TestEnv::new(),
        &["issue", "list", "for:", "me", "#Unresolved"],
        &server,
    )
    .await;
    assert!(output.status.success(), "{}", common::stderr(&output));
    let text = common::stdout(&output);
    assert!(
        text.contains("WEB-1") && text.contains("Open") && text.contains("Login fails"),
        "{text}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_repo_config_project_is_applied() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(query_param("query", "project: {WEB} and (#Unresolved)"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
        .expect(1)
        .mount(&server)
        .await;
    let env = TestEnv::new();
    std::fs::write(env.path().join(".yt.toml"), "project = \"WEB\"\n").unwrap();
    let output = run(&env, &["issue", "ls", "--query", "#Unresolved"], &server).await;
    assert!(output.status.success(), "{}", common::stderr(&output));
    assert!(common::stderr(&output).contains("No issues found"));
}

#[tokio::test(flavor = "multi_thread")]
async fn all_adds_a_stable_sort() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(query_param("query", "project: {WEB} sort by: created asc"))
        .and(query_param("$top", "100"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([row(1)])))
        .expect(1)
        .mount(&server)
        .await;
    let output = run(
        &TestEnv::new(),
        &["issue", "list", "-p", "WEB", "--all", "--format", "json"],
        &server,
    )
    .await;
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value.as_array().unwrap().len(), 1);
}

#[test]
fn query_words_and_the_query_flag_conflict() {
    let output = TestEnv::new().run_with(
        &["issue", "list", "for:", "me", "--query", "x"],
        &[
            ("YOUTRACK_URL", "https://acme.youtrack.cloud"),
            ("YOUTRACK_TOKEN", TOKEN),
        ],
    );
    assert_eq!(output.status.code(), Some(2));
}

#[test]
fn a_zero_limit_is_refused() {
    let output = TestEnv::new().run_with(
        &["issue", "list", "-n", "0"],
        &[
            ("YOUTRACK_URL", "https://acme.youtrack.cloud"),
            ("YOUTRACK_TOKEN", TOKEN),
        ],
    );
    assert_eq!(output.status.code(), Some(2));
    assert!(common::stderr(&output).contains("--limit"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_closed_pipe_stops_the_listing() {
    let server = MockServer::start().await;
    // Every page is full, so only the closed pipe can end the listing.
    Mock::given(method("GET"))
        .and(path("/api/issues"))
        .respond_with(|request: &wiremock::Request| {
            let skip: usize = request
                .url
                .query_pairs()
                .find(|(key, _)| key == "$skip")
                .and_then(|(_, value)| value.parse().ok())
                .unwrap_or(0);
            ResponseTemplate::new(200)
                .set_body_json((skip..skip + 100).map(row).collect::<Vec<_>>())
        })
        .mount(&server)
        .await;
    let env = TestEnv::new();
    let mut command = env.command();
    command
        .args(["issue", "list", "--all", "--format", "ndjson"])
        .env("YOUTRACK_URL", server.uri())
        .env("YOUTRACK_TOKEN", TOKEN)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let finished = tokio::time::timeout(
        Duration::from_secs(60),
        tokio::task::spawn_blocking(move || {
            let mut child = command.spawn().unwrap();
            let mut reader = std::io::BufReader::new(child.stdout.take().unwrap());
            let mut first = String::new();
            reader.read_line(&mut first).unwrap();
            drop(reader);
            let output = child.wait_with_output().unwrap();
            (first, output)
        }),
    )
    .await
    .expect("yt kept listing after its reader went away")
    .unwrap();
    let (first, output) = finished;
    assert!(first.contains("WEB-0"), "{first}");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!String::from_utf8_lossy(&output.stderr).contains("panicked"));
}

async fn server_with_summary(summary: &str, description: &str) -> MockServer {
    let server = MockServer::start().await;
    let mut fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/issue-show.json")).unwrap();
    fixture["summary"] = summary.into();
    fixture["description"] = description.into();
    Mock::given(method("GET"))
        .and(path("/api/issues/WEB-42"))
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture))
        .mount(&server)
        .await;
    server
}

fn has_control_characters(text: &str) -> bool {
    text.chars().any(|c| c.is_control() && c != '\n')
}

#[tokio::test(flavor = "multi_thread")]
async fn server_text_cannot_control_the_terminal() {
    let server = server_with_summary(
        "Login\u{1b}]8;;https://example.com\u{7}fails\u{9b}2J",
        "line one\r\nline two\u{1b}[1A",
    )
    .await;
    let output = run(&TestEnv::new(), &["issue", "show", "WEB-42"], &server).await;
    assert!(output.status.success(), "{}", common::stderr(&output));
    let text = common::stdout(&output);
    assert!(!has_control_characters(&text), "{text:?}");
    assert!(text.contains("line one\nline two"), "{text:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn json_output_escapes_control_characters_without_changing_them() {
    let summary = "a\u{9b}b\u{7f}c\u{1b}d";
    let server = server_with_summary(summary, "").await;
    let output = run(
        &TestEnv::new(),
        &["issue", "show", "WEB-42", "--format", "json"],
        &server,
    )
    .await;
    let text = common::stdout(&output);
    assert!(!has_control_characters(&text), "{text:?}");
    let value: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(value["summary"], summary);
}

#[tokio::test(flavor = "multi_thread")]
async fn server_errors_cannot_control_the_terminal() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/issues/WEB-42"))
        .respond_with(ResponseTemplate::new(400).set_body_json(serde_json::json!({
            "error": "bad_request",
            "error_description": "No issue\u{1b}]0;title\u{7}\u{1b}[2J here",
        })))
        .mount(&server)
        .await;
    let output = run(&TestEnv::new(), &["issue", "show", "WEB-42"], &server).await;
    assert!(!output.status.success());
    let text = common::stderr(&output);
    assert!(text.contains("No issue"), "{text:?}");
    assert!(!has_control_characters(&text), "{text:?}");
}
