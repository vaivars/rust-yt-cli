mod common;

use common::TestEnv;
use serde_json::json;
use wiremock::matchers::{body_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const TOKEN: &str = "perm-cmVsYXRl.dGVzdA==.secret123456";

async fn run(env: &TestEnv, server: &MockServer, args: &[&str]) -> std::process::Output {
    let url = server.uri();
    env.run_async(
        args,
        &[("YOUTRACK_URL", url.as_str()), ("YOUTRACK_TOKEN", TOKEN)],
    )
    .await
}

fn comment(id: &str, text: &str) -> serde_json::Value {
    json!({"id": id, "text": text, "author": {"login": "jane", "fullName": "Jane"}, "created": 1_000, "updated": null})
}

#[tokio::test(flavor = "multi_thread")]
async fn comment_list_shows_the_first_line() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/issues/WEB-1/comments"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!([comment("4-1", "First line\nsecond line")])),
        )
        .mount(&server)
        .await;
    let output = run(
        &TestEnv::new(),
        &server,
        &["issue", "comment", "ls", "WEB-1"],
    )
    .await;
    assert!(output.status.success(), "{}", common::stderr(&output));
    let stdout = common::stdout(&output);
    assert!(
        stdout.contains("4-1") && stdout.contains("jane") && stdout.contains("First line"),
        "{stdout}"
    );
    assert!(!stdout.contains("second line"), "{stdout}");
}

#[tokio::test(flavor = "multi_thread")]
async fn comment_list_json_has_urls() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/issues/WEB-1/comments"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([comment("4-1", "Hi")])))
        .mount(&server)
        .await;
    let output = run(
        &TestEnv::new(),
        &server,
        &["issue", "comment", "list", "WEB-1", "--format", "json"],
    )
    .await;
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value[0]["text"], "Hi");
    assert_eq!(value[0]["author"], "jane");
    assert_eq!(
        value[0]["url"],
        format!("{}/issue/WEB-1#focus=Comments-4-1.0-0", server.uri())
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn comment_add_prints_the_comment_url() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/issues/WEB-1/comments"))
        .and(body_json(json!({"text": "Hello"})))
        .respond_with(ResponseTemplate::new(200).set_body_json(comment("4-2", "Hello")))
        .expect(1)
        .mount(&server)
        .await;
    let output = run(
        &TestEnv::new(),
        &server,
        &["issue", "comment", "add", "WEB-1", "-b", "Hello"],
    )
    .await;
    assert!(output.status.success(), "{}", common::stderr(&output));
    assert_eq!(
        common::stdout(&output).trim(),
        format!("{}/issue/WEB-1#focus=Comments-4-2.0-0", server.uri())
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn comment_add_without_text_needs_it() {
    let server = MockServer::start().await;
    let output = run(
        &TestEnv::new(),
        &server,
        &["issue", "comment", "add", "WEB-1"],
    )
    .await;
    assert_eq!(output.status.code(), Some(2));
    assert!(common::stderr(&output).contains("-b"));
}

#[tokio::test(flavor = "multi_thread")]
async fn comment_edit_sends_the_new_text() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/issues/WEB-1/comments/4-2"))
        .and(body_json(json!({"text": "Fixed"})))
        .respond_with(ResponseTemplate::new(200).set_body_json(comment("4-2", "Fixed")))
        .expect(1)
        .mount(&server)
        .await;
    let output = run(
        &TestEnv::new(),
        &server,
        &["issue", "comment", "edit", "WEB-1", "4-2", "-b", "Fixed"],
    )
    .await;
    assert!(output.status.success(), "{}", common::stderr(&output));
    assert!(common::stderr(&output).contains("Updated comment 4-2"));
}

#[tokio::test(flavor = "multi_thread")]
async fn comment_delete_needs_yes_and_then_deletes() {
    let server = MockServer::start().await;
    Mock::given(method("DELETE"))
        .and(path("/api/issues/WEB-1/comments/4-2"))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&server)
        .await;
    let env = TestEnv::new();
    let refused = run(
        &env,
        &server,
        &["issue", "comment", "delete", "WEB-1", "4-2"],
    )
    .await;
    assert_eq!(refused.status.code(), Some(2));
    let output = run(
        &env,
        &server,
        &[
            "issue", "comment", "delete", "WEB-1", "4-2", "--yes", "--format", "json",
        ],
    )
    .await;
    assert!(output.status.success(), "{}", common::stderr(&output));
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value, json!(["4-2"]));
}

async fn mount_tags(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/api/tags"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!([{"name": "Needs review"}, {"name": "ui"}])),
        )
        .mount(server)
        .await;
}

async fn expect_command(server: &MockServer, query: &str, issue: &str) {
    Mock::given(method("POST"))
        .and(path("/api/commands"))
        .and(body_json(
            json!({"query": query, "issues": [{"idReadable": issue}]}),
        ))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(server)
        .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn tag_add_sends_one_command_per_tag() {
    let server = MockServer::start().await;
    mount_tags(&server).await;
    expect_command(&server, "tag Needs review", "WEB-1").await;
    expect_command(&server, "tag ui", "WEB-1").await;
    let output = run(
        &TestEnv::new(),
        &server,
        &["issue", "tag", "add", "WEB-1", "needs review", "UI"],
    )
    .await;
    assert!(output.status.success(), "{}", common::stderr(&output));
    assert!(common::stderr(&output).contains("Updated WEB-1"));
}

async fn expect_tag_created(server: &MockServer, name: &str, times: u64) {
    Mock::given(method("POST"))
        .and(path("/api/tags"))
        .and(body_json(json!({"name": name})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "6-9", "name": name})))
        .expect(times)
        .mount(server)
        .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unknown_tag_is_refused_unless_validation_is_off() {
    let server = MockServer::start().await;
    mount_tags(&server).await;
    let refused = run(
        &TestEnv::new(),
        &server,
        &["issue", "tag", "add", "WEB-1", "brand-new"],
    )
    .await;
    assert_eq!(refused.status.code(), Some(2));
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unknown_tag_is_created_before_it_is_applied() {
    let server = MockServer::start().await;
    mount_tags(&server).await;
    expect_tag_created(&server, "Brand new", 1).await;
    expect_command(&server, "tag Brand new", "WEB-1").await;
    let output = run(
        &TestEnv::new(),
        &server,
        &["issue", "tag", "add", "WEB-1", "Brand new", "--no-validate"],
    )
    .await;
    assert!(output.status.success(), "{}", common::stderr(&output));
    let posts: Vec<String> = server
        .received_requests()
        .await
        .unwrap()
        .into_iter()
        .filter(|request| request.method.as_str() == "POST")
        .map(|request| request.url.path().to_string())
        .collect();
    assert_eq!(posts, ["/api/tags", "/api/commands"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_known_tag_is_not_created_again_without_validation() {
    let server = MockServer::start().await;
    mount_tags(&server).await;
    expect_tag_created(&server, "UI", 0).await;
    expect_command(&server, "tag ui", "WEB-1").await;
    let output = run(
        &TestEnv::new(),
        &server,
        &["issue", "tag", "add", "WEB-1", "UI", "--no-validate"],
    )
    .await;
    assert!(output.status.success(), "{}", common::stderr(&output));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_dry_run_names_the_tags_it_would_create() {
    let server = MockServer::start().await;
    mount_tags(&server).await;
    expect_tag_created(&server, "Brand new", 0).await;
    Mock::given(method("POST"))
        .and(path("/api/commands/assist"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"commands": []})))
        .expect(1)
        .mount(&server)
        .await;
    let output = run(
        &TestEnv::new(),
        &server,
        &[
            "issue",
            "tag",
            "add",
            "WEB-1",
            "Brand new",
            "ui",
            "--no-validate",
            "--dry-run",
        ],
    )
    .await;
    assert!(output.status.success(), "{}", common::stderr(&output));
    assert_eq!(
        common::stdout(&output),
        "create tag \"Brand new\"\nWEB-1: command \"tag Brand new\"\nWEB-1: command \"tag ui\"\n"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn removing_an_unknown_tag_does_not_suggest_creating_it() {
    let server = MockServer::start().await;
    mount_tags(&server).await;
    let output = run(
        &TestEnv::new(),
        &server,
        &["issue", "tag", "remove", "WEB-1", "brand-new"],
    )
    .await;
    assert_eq!(output.status.code(), Some(2));
    let stderr = common::stderr(&output);
    assert!(!stderr.contains("create"), "{stderr}");
    assert!(stderr.contains("send it as given"), "{stderr}");
}

#[tokio::test(flavor = "multi_thread")]
async fn tag_remove_untags() {
    let server = MockServer::start().await;
    mount_tags(&server).await;
    expect_command(&server, "untag ui", "WEB-1").await;
    let output = run(
        &TestEnv::new(),
        &server,
        &["issue", "tag", "remove", "WEB-1", "ui"],
    )
    .await;
    assert!(output.status.success(), "{}", common::stderr(&output));
}

fn link_types() -> serde_json::Value {
    json!([{"name": "Depend", "sourceToTarget": "depends on", "targetToSource": "is required for", "directed": true}])
}

#[tokio::test(flavor = "multi_thread")]
async fn link_list_shows_direction_words_and_states() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/issues/WEB-1/links"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {"direction": "OUTWARD", "linkType": link_types()[0],
             "issues": [{"idReadable": "WEB-2", "summary": "Backend", "customFields": [
                 {"name": "State", "$type": "StateIssueCustomField", "value": {"name": "Open", "isResolved": false}}]}]},
            {"direction": "INWARD", "linkType": link_types()[0], "issues": []}
        ])))
        .mount(&server)
        .await;
    let output = run(
        &TestEnv::new(),
        &server,
        &["issue", "link", "ls", "WEB-1", "--format", "json"],
    )
    .await;
    assert!(output.status.success(), "{}", common::stderr(&output));
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        value,
        json!([{"type": "depends on", "direction": "outward",
                "issue": {"id": "WEB-2", "summary": "Backend", "state": "Open", "url": format!("{}/issue/WEB-2", server.uri())}}])
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn link_add_and_remove_send_direction_words() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/issueLinkTypes"))
        .respond_with(ResponseTemplate::new(200).set_body_json(link_types()))
        .mount(&server)
        .await;
    expect_command(&server, "depends on WEB-2", "WEB-1").await;
    expect_command(&server, "remove depends on WEB-2", "WEB-1").await;
    let env = TestEnv::new();
    let added = run(
        &env,
        &server,
        &["issue", "link", "add", "WEB-1", "Depends On", "WEB-2"],
    )
    .await;
    assert!(added.status.success(), "{}", common::stderr(&added));
    let removed = run(
        &env,
        &server,
        &["issue", "link", "remove", "WEB-1", "depends on", "WEB-2"],
    )
    .await;
    assert!(removed.status.success(), "{}", common::stderr(&removed));
}

#[tokio::test(flavor = "multi_thread")]
async fn link_types_lists_names_and_words() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/issueLinkTypes"))
        .respond_with(ResponseTemplate::new(200).set_body_json(link_types()))
        .mount(&server)
        .await;
    let output = run(
        &TestEnv::new(),
        &server,
        &["issue", "link", "types", "--format", "json"],
    )
    .await;
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        value,
        json!([{"name": "Depend", "outward": "depends on", "inward": "is required for", "directed": true}])
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_dot_segment_is_not_a_comment_id() {
    let server = MockServer::start().await;
    Mock::given(method("DELETE"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&server)
        .await;
    let output = run(
        &TestEnv::new(),
        &server,
        &["issue", "comment", "delete", "WEB-1", "..", "--yes"],
    )
    .await;
    assert_eq!(output.status.code(), Some(2));
}

#[tokio::test(flavor = "multi_thread")]
async fn comment_add_checks_the_issue_id_before_asking_for_text() {
    let server = MockServer::start().await;
    let output = run(
        &TestEnv::new(),
        &server,
        &["issue", "comment", "add", "WEB1"],
    )
    .await;
    assert_eq!(output.status.code(), Some(2));
    assert!(
        common::stderr(&output).contains("not an issue ID"),
        "{}",
        common::stderr(&output)
    );
}
