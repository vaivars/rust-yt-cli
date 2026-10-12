mod common;

use std::io::Write as _;
use std::process::Stdio;

use common::TestEnv;
use serde_json::json;
use wiremock::matchers::{body_json, body_partial_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const TOKEN: &str = "perm-d3JpdGU=.dGVzdA==.secret123456";

fn schema() -> serde_json::Value {
    json!([
        {"canBeEmpty": false, "defaultValues": [{"name": "Normal"}],
         "field": {"name": "Priority", "aliases": "", "fieldType": {"id": "enum[1]", "isMultiValue": false}},
         "bundle": {"values": [{"name": "Normal", "archived": false}, {"name": "Major", "archived": false}]}},
        {"canBeEmpty": false, "defaultValues": [{"name": "Open"}],
         "field": {"name": "State", "fieldType": {"id": "state[1]", "isMultiValue": false}},
         "bundle": {"values": [{"name": "Open", "archived": false}, {"name": "In Progress", "archived": false}]}},
        {"canBeEmpty": true, "defaultValues": [],
         "field": {"name": "Assignee", "fieldType": {"id": "user[1]", "isMultiValue": false}},
         "bundle": {"aggregatedUsers": [{"login": "jane"}, {"login": "bob"}]}},
        {"canBeEmpty": true, "defaultValues": [],
         "field": {"name": "Fix versions", "fieldType": {"id": "version[*]", "isMultiValue": true}},
         "bundle": {"values": [{"name": "1.0", "archived": false}, {"name": "1.1", "archived": false}]}}
    ])
}

async fn mount_schema(server: &MockServer, project: &str, fields: serde_json::Value) {
    Mock::given(method("GET"))
        .and(path(format!("/api/admin/projects/{project}/customFields")))
        .respond_with(ResponseTemplate::new(200).set_body_json(fields))
        .mount(server)
        .await;
}

fn issue(id: &str) -> serde_json::Value {
    json!({"idReadable": id, "summary": "Login fails", "description": null, "project": {"shortName": "WEB"},
           "reporter": null, "created": 0, "updated": 0, "resolved": null, "tags": [], "customFields": []})
}

async fn run(env: &TestEnv, server: &MockServer, args: &[&str]) -> std::process::Output {
    let url = server.uri();
    env.run_async(
        args,
        &[("YOUTRACK_URL", url.as_str()), ("YOUTRACK_TOKEN", TOKEN)],
    )
    .await
}

#[tokio::test(flavor = "multi_thread")]
async fn create_prints_the_url_and_sets_the_state_after() {
    let server = MockServer::start().await;
    mount_schema(&server, "WEB", schema()).await;
    Mock::given(method("POST"))
        .and(path("/api/issues"))
        .and(body_partial_json(json!({
            "project": {"shortName": "WEB"},
            "summary": "Login fails",
            "customFields": [{"name": "Priority", "$type": "SingleEnumIssueCustomField", "value": {"name": "Major"}}]
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"idReadable": "WEB-43"})))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/commands"))
        .and(body_json(
            json!({"query": "State In Progress", "issues": [{"idReadable": "WEB-43"}]}),
        ))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&server)
        .await;
    let output = run(
        &TestEnv::new(),
        &server,
        &[
            "issue",
            "create",
            "-p",
            "WEB",
            "-s",
            "Login fails",
            "-f",
            "Priority=major",
            "-f",
            "State=in progress",
        ],
    )
    .await;
    assert!(output.status.success(), "{}", common::stderr(&output));
    assert_eq!(
        common::stdout(&output).trim(),
        format!("{}/issue/WEB-43", server.uri())
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn create_json_prints_the_new_issue() {
    let server = MockServer::start().await;
    mount_schema(&server, "WEB", schema()).await;
    Mock::given(method("POST"))
        .and(path("/api/issues"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"idReadable": "WEB-43"})))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/issues/WEB-43"))
        .respond_with(ResponseTemplate::new(200).set_body_json(issue("WEB-43")))
        .mount(&server)
        .await;
    let output = run(
        &TestEnv::new(),
        &server,
        &[
            "issue",
            "create",
            "-p",
            "WEB",
            "-s",
            "Login fails",
            "--format",
            "json",
        ],
    )
    .await;
    assert!(output.status.success(), "{}", common::stderr(&output));
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["id"], "WEB-43");
}

#[tokio::test(flavor = "multi_thread")]
async fn create_needs_a_summary_without_a_terminal() {
    let server = MockServer::start().await;
    mount_schema(&server, "WEB", schema()).await;
    let output = run(&TestEnv::new(), &server, &["issue", "create", "-p", "WEB"]).await;
    assert_eq!(output.status.code(), Some(2));
    assert!(common::stderr(&output).contains("-s"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_required_field_without_a_value_is_named() {
    let server = MockServer::start().await;
    let mut fields = schema();
    fields.as_array_mut().unwrap().push(json!(
        {"canBeEmpty": false, "defaultValues": [],
         "field": {"name": "Type", "fieldType": {"id": "enum[1]", "isMultiValue": false}},
         "bundle": {"values": [{"name": "Bug", "archived": false}]}}
    ));
    mount_schema(&server, "WEB", fields).await;
    let output = run(
        &TestEnv::new(),
        &server,
        &["issue", "create", "-p", "WEB", "-s", "x"],
    )
    .await;
    assert_eq!(output.status.code(), Some(2));
    let stderr = common::stderr(&output);
    assert!(stderr.contains("Type") && stderr.contains("-f"), "{stderr}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_mistyped_value_suggests_the_closest() {
    let server = MockServer::start().await;
    mount_schema(&server, "WEB", schema()).await;
    let output = run(
        &TestEnv::new(),
        &server,
        &[
            "issue",
            "create",
            "-p",
            "WEB",
            "-s",
            "x",
            "-f",
            "Priority=Majr",
        ],
    )
    .await;
    assert_eq!(output.status.code(), Some(2));
    assert!(common::stderr(&output).contains("'Major'"));
}

#[tokio::test(flavor = "multi_thread")]
async fn create_dry_run_creates_nothing() {
    let server = MockServer::start().await;
    mount_schema(&server, "WEB", schema()).await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&server)
        .await;
    let output = run(
        &TestEnv::new(),
        &server,
        &[
            "issue",
            "create",
            "-p",
            "WEB",
            "-s",
            "Login fails",
            "-f",
            "Priority=Major",
            "--dry-run",
        ],
    )
    .await;
    assert!(output.status.success(), "{}", common::stderr(&output));
    let stdout = common::stdout(&output);
    assert!(
        stdout.contains("create in WEB") && stdout.contains("set Priority to Major"),
        "{stdout}"
    );
}

async fn run_with_stdin(
    env: &TestEnv,
    server: &MockServer,
    args: &[&str],
    input: &str,
) -> std::process::Output {
    let mut command = env.command();
    command
        .args(args)
        .env("YOUTRACK_URL", server.uri())
        .env("YOUTRACK_TOKEN", TOKEN)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let input = input.to_string();
    tokio::task::spawn_blocking(move || {
        let mut child = command.spawn().unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(input.as_bytes())
            .unwrap();
        child.wait_with_output().unwrap()
    })
    .await
    .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn edit_sends_rest_updates_then_commands() {
    let server = MockServer::start().await;
    mount_schema(&server, "WEB", schema()).await;
    for id in ["WEB-1", "WEB-2"] {
        Mock::given(method("POST"))
            .and(path(format!("/api/issues/{id}")))
            .and(body_json(json!({"customFields": [
                {"name": "Fix versions", "$type": "MultiVersionIssueCustomField", "value": [{"name": "1.0"}]},
                {"name": "Assignee", "$type": "SingleUserIssueCustomField", "value": null}
            ]})))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
            .expect(1)
            .mount(&server)
            .await;
    }
    Mock::given(method("POST"))
        .and(path("/api/commands"))
        .and(body_json(json!({"query": "Priority Major", "issues": [{"idReadable": "WEB-1"}, {"idReadable": "WEB-2"}]})))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&server)
        .await;
    let output = run(
        &TestEnv::new(),
        &server,
        &[
            "issue",
            "edit",
            "WEB-1",
            "WEB-2",
            "-f",
            "Fix versions=1.0",
            "-f",
            "Priority=major",
            "-f",
            "Assignee=",
        ],
    )
    .await;
    assert!(output.status.success(), "{}", common::stderr(&output));
    assert!(output.stdout.is_empty());
    assert!(common::stderr(&output).contains("Updated WEB-1, WEB-2"));
}

#[tokio::test(flavor = "multi_thread")]
async fn edit_uses_each_projects_own_schema() {
    let server = MockServer::start().await;
    mount_schema(&server, "WEB", schema()).await;
    let mut ops = schema();
    ops[0]["bundle"]["values"] = json!([{"name": "MAJOR", "archived": false}]);
    mount_schema(&server, "OPS", ops).await;
    for (id, value) in [("WEB-1", "Priority Major"), ("OPS-2", "Priority MAJOR")] {
        Mock::given(method("POST"))
            .and(path("/api/commands"))
            .and(body_json(
                json!({"query": value, "issues": [{"idReadable": id}]}),
            ))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;
    }
    let output = run(
        &TestEnv::new(),
        &server,
        &["issue", "edit", "WEB-1", "OPS-2", "-f", "Priority=major"],
    )
    .await;
    assert!(output.status.success(), "{}", common::stderr(&output));
}

#[tokio::test(flavor = "multi_thread")]
async fn edit_reads_ids_from_stdin() {
    let server = MockServer::start().await;
    mount_schema(&server, "WEB", schema()).await;
    Mock::given(method("POST"))
        .and(path("/api/commands"))
        .and(body_json(json!({"query": "Priority Major", "issues": [{"idReadable": "WEB-1"}, {"idReadable": "WEB-2"}]})))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&server)
        .await;
    let output = run_with_stdin(
        &TestEnv::new(),
        &server,
        &["issue", "edit", "-", "-f", "Priority=Major"],
        "WEB-1\n\nWEB-2\n",
    )
    .await;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn several_ids_take_only_fields() {
    let server = MockServer::start().await;
    let output = run(
        &TestEnv::new(),
        &server,
        &["issue", "edit", "WEB-1", "WEB-2", "-s", "x"],
    )
    .await;
    assert_eq!(output.status.code(), Some(2));
}

#[tokio::test(flavor = "multi_thread")]
async fn nothing_to_change_without_a_terminal_is_an_error() {
    let server = MockServer::start().await;
    let output = run(&TestEnv::new(), &server, &["issue", "edit", "WEB-1"]).await;
    assert_eq!(output.status.code(), Some(2));
    assert!(common::stderr(&output).contains("nothing to change"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_failed_edit_says_what_was_already_applied() {
    let server = MockServer::start().await;
    mount_schema(&server, "WEB", schema()).await;
    Mock::given(method("POST"))
        .and(path("/api/issues/WEB-1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/commands"))
        .respond_with(ResponseTemplate::new(400).set_body_json(
            json!({"error": "bad", "error_description": "Workflow rejected the change"}),
        ))
        .mount(&server)
        .await;
    let output = run(
        &TestEnv::new(),
        &server,
        &[
            "issue",
            "edit",
            "WEB-1",
            "-s",
            "New title",
            "-f",
            "Priority=Major",
        ],
    )
    .await;
    assert_eq!(output.status.code(), Some(1));
    assert!(common::stderr(&output).contains("stopped after a failure; already applied"));
}

#[tokio::test(flavor = "multi_thread")]
async fn edit_dry_run_checks_commands_with_youtrack() {
    let server = MockServer::start().await;
    mount_schema(&server, "WEB", schema()).await;
    Mock::given(method("POST"))
        .and(path("/api/commands/assist"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"commands": [
                {"description": "Workflow forbids <b>Major</b>", "error": true}
            ]})),
        )
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/commands"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&server)
        .await;
    let output = run(
        &TestEnv::new(),
        &server,
        &[
            "issue",
            "edit",
            "WEB-1",
            "-f",
            "Priority=Major",
            "--dry-run",
        ],
    )
    .await;
    assert_eq!(output.status.code(), Some(1));
    let stdout = common::stdout(&output);
    assert!(
        stdout.contains("WEB-1: command \"Priority Major\""),
        "{stdout}"
    );
    assert!(
        stdout.contains("problem: Workflow forbids Major"),
        "{stdout}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn command_applies_raw_text_to_every_issue() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/commands"))
        .and(body_json(json!({"query": "for me", "issues": [{"idReadable": "WEB-1"}, {"idReadable": "WEB-2"}]})))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&server)
        .await;
    let output = run(
        &TestEnv::new(),
        &server,
        &["issue", "command", "for me", "WEB-1", "WEB-2"],
    )
    .await;
    assert!(output.status.success(), "{}", common::stderr(&output));
    assert!(common::stderr(&output).contains("Updated WEB-1, WEB-2"));
}

#[tokio::test(flavor = "multi_thread")]
async fn command_json_prints_the_issues() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/commands"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/issues/WEB-1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(issue("WEB-1")))
        .mount(&server)
        .await;
    let output = run(
        &TestEnv::new(),
        &server,
        &["issue", "command", "for me", "WEB-1", "--format", "json"],
    )
    .await;
    assert!(output.status.success(), "{}", common::stderr(&output));
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value[0]["id"], "WEB-1");
}

#[tokio::test(flavor = "multi_thread")]
async fn command_dry_run_changes_nothing() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/commands/assist"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"commands": [
                {"description": "Set Assignee to <b>jane</b>", "error": false}
            ]})),
        )
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/commands"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&server)
        .await;
    let output = run(
        &TestEnv::new(),
        &server,
        &["issue", "command", "for me", "WEB-1", "--dry-run"],
    )
    .await;
    assert!(output.status.success(), "{}", common::stderr(&output));
    assert!(common::stdout(&output).contains("WEB-1: command \"for me\""));
}

#[tokio::test(flavor = "multi_thread")]
async fn delete_needs_yes_without_a_terminal() {
    let server = MockServer::start().await;
    Mock::given(method("DELETE"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&server)
        .await;
    let output = run(&TestEnv::new(), &server, &["issue", "delete", "WEB-1"]).await;
    assert_eq!(output.status.code(), Some(2));
    assert!(common::stderr(&output).contains("--yes"));
}

#[tokio::test(flavor = "multi_thread")]
async fn delete_with_yes_prints_the_deleted_ids() {
    let server = MockServer::start().await;
    for id in ["WEB-1", "WEB-2"] {
        Mock::given(method("DELETE"))
            .and(path(format!("/api/issues/{id}")))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;
    }
    let output = run(
        &TestEnv::new(),
        &server,
        &[
            "issue", "delete", "WEB-1", "WEB-2", "--yes", "--format", "json",
        ],
    )
    .await;
    assert!(output.status.success(), "{}", common::stderr(&output));
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value, json!(["WEB-1", "WEB-2"]));
}

#[tokio::test(flavor = "multi_thread")]
async fn edit_dry_run_needs_a_change_to_show() {
    let server = MockServer::start().await;
    let output = run(
        &TestEnv::new(),
        &server,
        &["issue", "edit", "WEB-1", "--dry-run"],
    )
    .await;
    assert_eq!(output.status.code(), Some(2));
    assert!(
        common::stderr(&output).contains("--dry-run"),
        "{}",
        common::stderr(&output)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn edit_refuses_a_blank_summary() {
    let server = MockServer::start().await;
    let output = run(
        &TestEnv::new(),
        &server,
        &["issue", "edit", "WEB-1", "-s", "  "],
    )
    .await;
    assert_eq!(output.status.code(), Some(2));
    assert!(
        common::stderr(&output).contains("summary cannot be empty"),
        "{}",
        common::stderr(&output)
    );
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn ids_from_stdin_take_the_first_word_of_each_line() {
    let server = MockServer::start().await;
    mount_schema(&server, "WEB", schema()).await;
    Mock::given(method("POST"))
        .and(path("/api/commands"))
        .and(body_json(json!({"query": "Priority Major", "issues": [{"idReadable": "WEB-1"}, {"idReadable": "WEB-2"}]})))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&server)
        .await;
    let output = run_with_stdin(
        &TestEnv::new(),
        &server,
        &["issue", "edit", "-", "-f", "Priority=Major"],
        "WEB-1  Login fails  Open\nWEB-2\tSecond\n",
    )
    .await;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
