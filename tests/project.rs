mod common;

use common::TestEnv;
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const TOKEN: &str = "perm-cHJvamVjdA==.dGVzdA==.secret123456";

async fn run(server: &MockServer, args: &[&str]) -> std::process::Output {
    let url = server.uri();
    TestEnv::new()
        .run_async(
            args,
            &[("YOUTRACK_URL", url.as_str()), ("YOUTRACK_TOKEN", TOKEN)],
        )
        .await
}

async fn mount_projects(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/api/admin/projects"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {"shortName": "TEST", "name": "Test project", "leader": {"login": "jane", "fullName": "Jane"}, "description": "Scratch space", "archived": false},
            {"shortName": "OLD", "name": "Old project", "leader": null, "description": null, "archived": true}
        ])))
        .mount(server)
        .await;
}

async fn mount_test_details(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/api/admin/projects/TEST/timeTrackingSettings"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"enabled": true, "workItemTypes": []})),
        )
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/admin/projects/TEST/customFields"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {"canBeEmpty": false, "defaultValues": [{"name": "Normal"}], "field": {"name": "Priority", "aliases": [], "fieldType": {"id": "enum[1]", "isMultiValue": false}}, "bundle": {"values": [{"name": "Major", "archived": false}, {"name": "Normal", "archived": false}]}},
            {"canBeEmpty": true, "defaultValues": [], "field": {"name": "Fix versions", "aliases": [], "fieldType": {"id": "version[*]", "isMultiValue": true}}, "bundle": {"values": []}}
        ])))
        .mount(server)
        .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn project_list_marks_archived_projects() {
    let server = MockServer::start().await;
    mount_projects(&server).await;
    let output = run(&server, &["project", "ls"]).await;
    assert!(output.status.success(), "{}", common::stderr(&output));
    assert_eq!(
        common::stdout(&output),
        "TEST  Test project            jane\nOLD   Old project (archived)\n"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn project_list_json_has_urls() {
    let server = MockServer::start().await;
    mount_projects(&server).await;
    let output = run(&server, &["project", "list", "-n", "1", "--format", "json"]).await;
    assert!(output.status.success(), "{}", common::stderr(&output));
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        value,
        json!([{"short_name": "TEST", "name": "Test project", "leader": "jane", "archived": false, "url": format!("{}/projects/TEST", server.uri())}])
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn project_show_finds_the_project_in_any_case() {
    let server = MockServer::start().await;
    mount_projects(&server).await;
    mount_test_details(&server).await;
    let output = run(&server, &["project", "show", "test"]).await;
    assert!(output.status.success(), "{}", common::stderr(&output));
    let stdout = common::stdout(&output);
    assert!(stdout.starts_with("TEST  Test project\n"), "{stdout}");
    assert!(stdout.contains("Time tracking:  on"), "{stdout}");
    assert!(
        stdout.contains("Fields:         Priority, Fix versions"),
        "{stdout}"
    );
    assert!(stdout.trim_end().ends_with("Scratch space"), "{stdout}");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unknown_project_suggests_the_closest() {
    let server = MockServer::start().await;
    mount_projects(&server).await;
    let output = run(&server, &["project", "show", "TES"]).await;
    assert_eq!(output.status.code(), Some(2));
    assert!(common::stderr(&output).contains("did you mean 'TEST'"));
}

#[tokio::test(flavor = "multi_thread")]
async fn project_fields_lists_types_and_values() {
    let server = MockServer::start().await;
    mount_projects(&server).await;
    mount_test_details(&server).await;
    let output = run(&server, &["project", "fields", "TEST"]).await;
    assert!(output.status.success(), "{}", common::stderr(&output));
    assert_eq!(
        common::stdout(&output),
        "Priority      enum     no   no  Major, Normal\nFix versions  version  yes  no\n"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn project_fields_json_has_every_value() {
    let server = MockServer::start().await;
    mount_projects(&server).await;
    mount_test_details(&server).await;
    let output = run(&server, &["project", "fields", "TEST", "--format", "json"]).await;
    assert!(output.status.success(), "{}", common::stderr(&output));
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        value[0],
        json!({"name": "Priority", "aliases": [], "type": "enum", "multi": false, "required": false, "values": ["Major", "Normal"]})
    );
}

async fn mount_fields_and_settings(server: &MockServer, status: u16) {
    Mock::given(method("GET"))
        .and(path("/api/admin/projects/TEST/timeTrackingSettings"))
        .respond_with(
            ResponseTemplate::new(status)
                .set_body_json(json!({"error": "Denied", "error_description": "No access"})),
        )
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/admin/projects/TEST/customFields"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/issues"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
        .mount(server)
        .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn hidden_time_tracking_settings_leave_the_row_out() {
    let server = MockServer::start().await;
    mount_projects(&server).await;
    mount_fields_and_settings(&server, 403).await;
    let output = run(&server, &["project", "show", "TEST"]).await;
    assert!(output.status.success(), "{}", common::stderr(&output));
    assert!(!common::stdout(&output).contains("Time tracking"));
}

#[tokio::test(flavor = "multi_thread")]
async fn other_time_tracking_failures_are_reported() {
    let server = MockServer::start().await;
    mount_projects(&server).await;
    mount_fields_and_settings(&server, 400).await;
    let output = run(&server, &["project", "show", "TEST"]).await;
    assert_eq!(output.status.code(), Some(1));
    assert!(
        common::stderr(&output).contains("No access"),
        "{}",
        common::stderr(&output)
    );
}
