mod common;

use common::TestEnv;
use serde_json::json;
use wiremock::matchers::{body_json, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

const TOKEN: &str = "perm-dGltZQ==.dGVzdA==.secret123456";

async fn run(server: &MockServer, args: &[&str]) -> std::process::Output {
    let url = server.uri();
    TestEnv::new()
        .run_async(
            args,
            &[("YOUTRACK_URL", url.as_str()), ("YOUTRACK_TOKEN", TOKEN)],
        )
        .await
}

fn item(id: &str, date: i64, minutes: i64, presentation: &str) -> serde_json::Value {
    json!({"id": id, "date": date, "duration": {"minutes": minutes, "presentation": presentation}, "type": {"id": "85-0", "name": "Development"}, "text": "Login form", "author": {"login": "jane", "fullName": "Jane"}, "issue": {"idReadable": "WEB-1"}, "created": 1})
}

async fn mount_types(server: &MockServer, enabled: bool) {
    Mock::given(method("GET"))
        .and(path("/api/issues/WEB-1"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"project": {"shortName": "WEB"}})),
        )
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/admin/projects/WEB/timeTrackingSettings"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "enabled": enabled,
            "workItemTypes": [{"id": "85-0", "name": "Development"}, {"id": "85-1", "name": "Testing"}]
        })))
        .mount(server)
        .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn time_log_sends_the_day_duration_type_and_text() {
    let server = MockServer::start().await;
    mount_types(&server, true).await;
    Mock::given(method("POST"))
        .and(path("/api/issues/WEB-1/timeTracking/workItems"))
        .and(body_json(json!({
            "date": 1_791_590_400_000_i64,
            "duration": {"presentation": "1h 30m"},
            "type": {"id": "85-0"},
            "text": "Login form"
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(item(
            "270-1",
            1_791_590_400_000,
            90,
            "1h 30m",
        )))
        .expect(1)
        .mount(&server)
        .await;
    let output = run(
        &server,
        &[
            "time",
            "log",
            "WEB-1",
            "1h 30m",
            "--date",
            "2026-10-10",
            "--type",
            "development",
            "-b",
            "Login form",
        ],
    )
    .await;
    assert!(output.status.success(), "{}", common::stderr(&output));
    assert!(common::stdout(&output).is_empty());
    assert!(common::stderr(&output).contains("Logged 1h 30m on WEB-1"));
}

#[tokio::test(flavor = "multi_thread")]
async fn time_log_json_prints_the_work_item() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/issues/WEB-1/timeTracking/workItems"))
        .respond_with(ResponseTemplate::new(200).set_body_json(item(
            "270-1",
            1_791_590_400_000,
            90,
            "1h 30m",
        )))
        .mount(&server)
        .await;
    let output = run(
        &server,
        &[
            "time",
            "log",
            "WEB-1",
            "90m",
            "--date",
            "2026-10-10",
            "--format",
            "json",
        ],
    )
    .await;
    assert!(output.status.success(), "{}", common::stderr(&output));
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        value,
        json!({"id": "270-1", "issue": "WEB-1", "date": "2026-10-10", "duration": {"minutes": 90, "presentation": "1h 30m"}, "type": "Development", "author": "jane", "text": "Login form"})
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn time_log_dry_run_sends_nothing() {
    let server = MockServer::start().await;
    mount_types(&server, true).await;
    let output = run(
        &server,
        &[
            "time",
            "log",
            "WEB-1",
            "2h",
            "--date",
            "2026-10-10",
            "--type",
            "Testing",
            "--dry-run",
        ],
    )
    .await;
    assert!(output.status.success(), "{}", common::stderr(&output));
    assert_eq!(
        common::stdout(&output),
        "WEB-1: log 2h on 2026-10-10 (Testing)\n"
    );
    let posts = server
        .received_requests()
        .await
        .unwrap()
        .into_iter()
        .filter(|request| request.method.as_str() == "POST")
        .count();
    assert_eq!(posts, 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_bare_number_is_not_a_duration() {
    let server = MockServer::start().await;
    let output = run(&server, &["time", "log", "WEB-1", "90"]).await;
    assert_eq!(output.status.code(), Some(2));
    assert!(
        common::stderr(&output).contains("write 90m"),
        "{}",
        common::stderr(&output)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn time_log_says_when_time_tracking_is_off() {
    let server = MockServer::start().await;
    mount_types(&server, false).await;
    let output = run(
        &server,
        &["time", "log", "WEB-1", "1h", "--type", "Testing"],
    )
    .await;
    assert_eq!(output.status.code(), Some(2));
    assert!(common::stderr(&output).contains("time tracking is turned off in project WEB"));
}

#[tokio::test(flavor = "multi_thread")]
async fn time_types_lists_the_projects_types() {
    let server = MockServer::start().await;
    mount_types(&server, true).await;
    let output = run(&server, &["time", "types", "-p", "WEB"]).await;
    assert!(output.status.success(), "{}", common::stderr(&output));
    assert_eq!(common::stdout(&output), "Development\nTesting\n");
}

async fn mount_me(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/api/users/me"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"id": "1-1", "login": "jane", "name": "Jane"})),
        )
        .mount(server)
        .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn time_list_shows_my_items_newest_first() {
    let server = MockServer::start().await;
    mount_me(&server).await;
    Mock::given(method("GET"))
        .and(path("/api/workItems"))
        .and(query_param("author", "jane"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            item("270-1", 1_791_331_200_000, 90, "1h 30m"),
            item("270-2", 1_791_590_400_000, 120, "2h")
        ])))
        .mount(&server)
        .await;
    let output = run(&server, &["time", "ls"]).await;
    assert!(output.status.success(), "{}", common::stderr(&output));
    assert_eq!(
        common::stdout(&output),
        "270-2  2026-10-10  WEB-1  2h      Development  jane  Login form\n270-1  2026-10-07  WEB-1  1h 30m  Development  jane  Login form\n"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn time_list_passes_the_query_dates_and_author() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/workItems"))
        .and(query_param("query", "WEB-1"))
        .and(query_param("startDate", "2026-10-01"))
        .and(query_param("endDate", "2026-10-31"))
        .and(query_param("author", "bob"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([item(
            "270-1",
            1_791_590_400_000,
            90,
            "1h 30m"
        )])))
        .expect(1)
        .mount(&server)
        .await;
    let output = run(
        &server,
        &[
            "time",
            "list",
            "WEB-1",
            "--from",
            "2026-10-01",
            "--to",
            "2026-10-31",
            "--author",
            "bob",
            "--format",
            "json",
        ],
    )
    .await;
    assert!(output.status.success(), "{}", common::stderr(&output));
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value[0]["id"], "270-1");
}

#[tokio::test(flavor = "multi_thread")]
async fn author_all_sends_no_author() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/workItems"))
        .respond_with(|request: &wiremock::Request| {
            assert!(!request.url.query_pairs().any(|(key, _)| key == "author"));
            ResponseTemplate::new(200).set_body_json(json!([]))
        })
        .mount(&server)
        .await;
    let output = run(&server, &["time", "list", "--author", "all"]).await;
    assert!(output.status.success(), "{}", common::stderr(&output));
    assert!(common::stderr(&output).contains("No work items found."));
}

#[tokio::test(flavor = "multi_thread")]
async fn the_newest_item_wins_even_on_the_last_page() {
    let server = MockServer::start().await;
    mount_me(&server).await;
    let page = 500;
    let old: Vec<serde_json::Value> = (0..page)
        .map(|n| item(&format!("270-{n}"), 1_791_331_200_000, 30, "30m"))
        .collect();
    Mock::given(method("GET"))
        .and(path("/api/workItems"))
        .and(query_param("$skip", "0"))
        .respond_with(ResponseTemplate::new(200).set_body_json(old))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/workItems"))
        .and(query_param("$skip", page.to_string()))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([item(
            "999-1",
            1_791_590_400_000,
            60,
            "1h"
        )])))
        .mount(&server)
        .await;
    let output = run(&server, &["time", "list", "-n", "1", "--format", "json"]).await;
    assert!(output.status.success(), "{}", common::stderr(&output));
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value.as_array().unwrap().len(), 1);
    assert_eq!(value[0]["id"], "999-1");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_bad_date_is_a_usage_error() {
    let server = MockServer::start().await;
    let output = run(&server, &["time", "list", "--from", "10/01"]).await;
    assert_eq!(output.status.code(), Some(2));
    assert!(common::stderr(&output).contains("--from"));
}

#[tokio::test(flavor = "multi_thread")]
async fn time_edit_sends_only_the_changes() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/issues/WEB-1/timeTracking/workItems/270-1"))
        .and(body_json(
            json!({"duration": {"presentation": "2h"}, "text": "Reviewed"}),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(item(
            "270-1",
            1_791_590_400_000,
            120,
            "2h",
        )))
        .expect(1)
        .mount(&server)
        .await;
    let output = run(
        &server,
        &[
            "time",
            "edit",
            "WEB-1",
            "270-1",
            "--duration",
            "2h",
            "-b",
            "Reviewed",
        ],
    )
    .await;
    assert!(output.status.success(), "{}", common::stderr(&output));
    assert!(common::stderr(&output).contains("Updated work item 270-1 on WEB-1"));
}

#[tokio::test(flavor = "multi_thread")]
async fn time_edit_needs_a_change() {
    let server = MockServer::start().await;
    let output = run(&server, &["time", "edit", "WEB-1", "270-1"]).await;
    assert_eq!(output.status.code(), Some(2));
    assert!(common::stderr(&output).contains("nothing to change"));
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn time_edit_dry_run_sends_nothing() {
    let server = MockServer::start().await;
    let output = run(
        &server,
        &[
            "time",
            "edit",
            "WEB-1",
            "270-1",
            "--date",
            "2026-10-09",
            "--dry-run",
        ],
    )
    .await;
    assert!(output.status.success(), "{}", common::stderr(&output));
    assert_eq!(
        common::stdout(&output),
        "WEB-1: change work item 270-1: date 2026-10-09\n"
    );
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn time_delete_needs_yes_without_a_terminal() {
    let server = MockServer::start().await;
    let output = run(&server, &["time", "delete", "WEB-1", "270-1"]).await;
    assert_eq!(output.status.code(), Some(2));
    assert!(common::stderr(&output).contains("refusing to delete work items 270-1 on WEB-1"));
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn time_delete_stops_at_the_first_failure() {
    let server = MockServer::start().await;
    Mock::given(method("DELETE"))
        .and(path("/api/issues/WEB-1/timeTracking/workItems/270-1"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path("/api/issues/WEB-1/timeTracking/workItems/270-2"))
        .respond_with(ResponseTemplate::new(404).set_body_json(
            json!({"error": "Not Found", "error_description": "Entity with id 270-2 not found"}),
        ))
        .mount(&server)
        .await;
    let output = run(
        &server,
        &[
            "time", "delete", "WEB-1", "270-1", "270-2", "270-3", "--yes",
        ],
    )
    .await;
    assert_eq!(output.status.code(), Some(1));
    let stderr = common::stderr(&output);
    assert!(stderr.contains("already deleted: 270-1"), "{stderr}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_cut_list_says_how_many_items_matched() {
    let server = MockServer::start().await;
    mount_me(&server).await;
    Mock::given(method("GET"))
        .and(path("/api/workItems"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            item("270-1", 1_791_331_200_000, 90, "1h 30m"),
            item("270-2", 1_791_590_400_000, 120, "2h"),
            item("270-3", 1_791_244_800_000, 60, "1h")
        ])))
        .mount(&server)
        .await;
    let output = run(&server, &["time", "list", "-n", "2"]).await;
    assert!(output.status.success(), "{}", common::stderr(&output));
    assert_eq!(common::stdout(&output).lines().count(), 2);
    let stderr = common::stderr(&output);
    assert!(
        stderr.contains("Showing 2 of 3 work items (4h 30m in all); --all lists every one."),
        "{stderr}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn loose_dates_are_sent_and_shown_normalised() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/workItems"))
        .and(query_param("startDate", "2026-10-01"))
        .and(query_param("endDate", "2026-10-09"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
        .expect(1)
        .mount(&server)
        .await;
    let listed = run(
        &server,
        &[
            "time",
            "list",
            "--author",
            "all",
            "--from",
            "2026-10-1",
            "--to",
            "2026-10-9",
        ],
    )
    .await;
    assert!(listed.status.success(), "{}", common::stderr(&listed));
    let logged = run(
        &server,
        &[
            "time",
            "log",
            "WEB-1",
            "1h",
            "--date",
            "2026-10-1",
            "--dry-run",
        ],
    )
    .await;
    assert_eq!(common::stdout(&logged), "WEB-1: log 1h on 2026-10-01\n");
    let edited = run(
        &server,
        &[
            "time",
            "edit",
            "WEB-1",
            "270-1",
            "--date",
            "2026-10-9",
            "--dry-run",
        ],
    )
    .await;
    assert_eq!(
        common::stdout(&edited),
        "WEB-1: change work item 270-1: date 2026-10-09\n"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn types_come_from_the_issues_current_project() {
    let server = MockServer::start().await;
    mount_types(&server, true).await;
    Mock::given(method("GET"))
        .and(path("/api/issues/OLD-12"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"project": {"shortName": "WEB"}})),
        )
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/issues/OLD-12/timeTracking/workItems"))
        .and(body_json(json!({
            "date": 1_791_590_400_000_i64,
            "duration": {"presentation": "1h"},
            "type": {"id": "85-1"}
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(item(
            "270-9",
            1_791_590_400_000,
            60,
            "1h",
        )))
        .expect(1)
        .mount(&server)
        .await;
    let output = run(
        &server,
        &[
            "time",
            "log",
            "OLD-12",
            "1h",
            "--date",
            "2026-10-10",
            "--type",
            "testing",
        ],
    )
    .await;
    assert!(output.status.success(), "{}", common::stderr(&output));
}

#[tokio::test(flavor = "multi_thread")]
async fn time_types_without_a_project_lists_the_instances_types() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/admin/timeTrackingSettings/workItemTypes"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {"id": "85-0", "name": "Development"}, {"id": "85-2", "name": "Documentation"}
        ])))
        .mount(&server)
        .await;
    let output = run(&server, &["time", "types"]).await;
    assert!(output.status.success(), "{}", common::stderr(&output));
    assert_eq!(common::stdout(&output), "Development\nDocumentation\n");
}

#[tokio::test(flavor = "multi_thread")]
async fn time_edit_type_sends_the_type_id() {
    let server = MockServer::start().await;
    mount_types(&server, true).await;
    Mock::given(method("POST"))
        .and(path("/api/issues/WEB-1/timeTracking/workItems/270-1"))
        .and(body_json(json!({"type": {"id": "85-1"}})))
        .respond_with(ResponseTemplate::new(200).set_body_json(item(
            "270-1",
            1_791_590_400_000,
            90,
            "1h 30m",
        )))
        .expect(1)
        .mount(&server)
        .await;
    let output = run(
        &server,
        &["time", "edit", "WEB-1", "270-1", "--type", "TESTING"],
    )
    .await;
    assert!(output.status.success(), "{}", common::stderr(&output));
}
