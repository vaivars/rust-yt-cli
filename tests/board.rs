mod common;

use common::TestEnv;
use serde_json::json;
use wiremock::matchers::{body_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const TOKEN: &str = "perm-Ym9hcmQ=.dGVzdA==.secret123456";

async fn run(server: &MockServer, args: &[&str]) -> std::process::Output {
    let url = server.uri();
    TestEnv::new()
        .run_async(
            args,
            &[("YOUTRACK_URL", url.as_str()), ("YOUTRACK_TOKEN", TOKEN)],
        )
        .await
}

fn board(id: &str, name: &str, sprints_off: bool) -> serde_json::Value {
    json!({
        "id": id,
        "name": name,
        "owner": {"login": "jane", "fullName": "Jane"},
        "projects": [{"shortName": "WEB", "name": "Web"}],
        "sprintsSettings": {"disableSprints": sprints_off, "isExplicit": false},
        "currentSprint": {"id": "111-0", "name": "First sprint"},
        "columnSettings": {"field": {"name": "Stage"}, "columns": [
            {"presentation": "Procesā", "isResolved": false, "fieldValues": [{"name": "Procesā"}]},
            {"presentation": "Darīts", "isResolved": true, "fieldValues": [{"name": "Darīts"}]}
        ]},
        "swimlaneSettings": null
    })
}

fn issue(id: &str, summary: &str, stage: &str) -> serde_json::Value {
    json!({
        "idReadable": id, "summary": summary, "project": {"shortName": "WEB", "name": "Web"},
        "created": 0, "updated": 0, "resolved": null,
        "customFields": [{
            "name": "Stage", "$type": "SingleEnumIssueCustomField",
            "projectCustomField": {"field": {"fieldType": {"id": "enum[1]"}}},
            "value": {"name": stage, "$type": "EnumBundleElement"}
        }]
    })
}

async fn mount_board(server: &MockServer, sprints_off: bool) {
    Mock::given(method("GET"))
        .and(path("/api/agiles"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            board("110-0", "Web Kanban Board", sprints_off),
            board("110-1", "Mājaslapa Kanban Board", sprints_off)
        ])))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/agiles/110-0/sprints"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {"id": "111-0", "name": "First sprint", "goal": null, "start": null, "finish": null, "archived": false}
        ])))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/agiles/110-0/sprints/111-0"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({"id": "111-0", "name": "First sprint", "goal": null, "start": null, "finish": null, "archived": false}),
        ))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/agiles/110-0/sprints/111-0/issues"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            issue("WEB-1", "Login page", "Procesā"),
            issue("WEB-2", "Signup form", "Darīts"),
            issue("WEB-3", "Password reset", "Ideja")
        ])))
        .mount(server)
        .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn board_list_shows_sprints_and_projects() {
    let server = MockServer::start().await;
    mount_board(&server, true).await;
    let output = run(&server, &["board", "ls"]).await;
    assert!(output.status.success(), "{}", common::stderr(&output));
    let stdout = common::stdout(&output);
    assert!(
        stdout.starts_with("Web Kanban Board        WEB  off  First sprint  jane\n"),
        "{stdout}"
    );
    assert!(stdout.contains("Mājaslapa Kanban Board"), "{stdout}");
}

#[tokio::test(flavor = "multi_thread")]
async fn board_show_groups_the_current_sprint_by_column() {
    let server = MockServer::start().await;
    mount_board(&server, true).await;
    let output = run(&server, &["board", "show", "web kanban board"]).await;
    assert!(output.status.success(), "{}", common::stderr(&output));
    let stdout = common::stdout(&output);
    assert!(stdout.starts_with("Web Kanban Board\n"), "{stdout}");
    assert!(
        stdout.contains("Columns:   Stage: Procesā, Darīts"),
        "{stdout}"
    );
    assert!(
        stdout.contains("\nProcesā (1)\nWEB-1  Login page\n"),
        "{stdout}"
    );
    assert!(stdout.contains("\nDarīts (1, resolved)\n"), "{stdout}");
    assert!(!stdout.contains("Signup form"), "{stdout}");
    assert!(
        stdout.contains("\nOther (1)\nWEB-3  Password reset"),
        "{stdout}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn resolved_columns_are_listed_on_request() {
    let server = MockServer::start().await;
    mount_board(&server, true).await;
    let output = run(&server, &["board", "show", "110-0", "--resolved"]).await;
    assert!(output.status.success(), "{}", common::stderr(&output));
    assert!(common::stdout(&output).contains("WEB-2  Signup form"));
}

#[tokio::test(flavor = "multi_thread")]
async fn board_show_json_has_every_issue() {
    let server = MockServer::start().await;
    mount_board(&server, true).await;
    let output = run(&server, &["board", "show", "110-0", "--format", "json"]).await;
    assert!(output.status.success(), "{}", common::stderr(&output));
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["sprint"]["name"], "First sprint");
    assert_eq!(value["columns"][1]["name"], "Darīts");
    assert_eq!(value["columns"][1]["resolved"], true);
    assert_eq!(value["columns"][1]["issues"][0]["id"], "WEB-2");
    assert!(value["columns"][2]["name"].is_null());
}

#[tokio::test(flavor = "multi_thread")]
async fn sprint_list_marks_the_current_sprint_and_says_when_sprints_are_off() {
    let server = MockServer::start().await;
    mount_board(&server, true).await;
    let output = run(&server, &["board", "sprint", "ls", "110-0"]).await;
    assert!(output.status.success(), "{}", common::stderr(&output));
    assert!(common::stdout(&output).starts_with("* First sprint"));
    assert!(common::stderr(&output).contains("Sprints are turned off"));
}

#[tokio::test(flavor = "multi_thread")]
async fn sprint_current_shows_the_column_view() {
    let server = MockServer::start().await;
    mount_board(&server, true).await;
    let output = run(&server, &["board", "sprint", "current", "110-0"]).await;
    assert!(output.status.success(), "{}", common::stderr(&output));
    assert!(common::stdout(&output).contains("WEB-1  Login page"));
}

#[tokio::test(flavor = "multi_thread")]
async fn sprint_show_finds_the_sprint_by_name() {
    let server = MockServer::start().await;
    mount_board(&server, true).await;
    let output = run(
        &server,
        &["board", "sprint", "show", "110-0", "first SPRINT"],
    )
    .await;
    assert!(output.status.success(), "{}", common::stderr(&output));
    assert!(common::stdout(&output).contains("Sprint:    First sprint"));
}

#[tokio::test(flavor = "multi_thread")]
async fn sprint_create_is_refused_when_sprints_are_off() {
    let server = MockServer::start().await;
    mount_board(&server, true).await;
    let output = run(&server, &["board", "sprint", "create", "110-0", "Sprint 2"]).await;
    assert_eq!(output.status.code(), Some(2));
    assert!(common::stderr(&output).contains("sprints are turned off on board Web Kanban Board"));
}

#[tokio::test(flavor = "multi_thread")]
async fn sprint_create_sends_the_dates_and_goal() {
    let server = MockServer::start().await;
    mount_board(&server, false).await;
    Mock::given(method("POST"))
        .and(path("/api/agiles/110-0/sprints"))
        .and(body_json(json!({"name": "Sprint 2", "goal": "Ship", "start": 1_791_590_400_000_i64, "finish": 1_792_195_200_000_i64})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "111-2", "name": "Sprint 2", "goal": "Ship", "start": 1_791_590_400_000_i64, "finish": 1_792_195_200_000_i64, "archived": false
        })))
        .expect(1)
        .mount(&server)
        .await;
    let output = run(
        &server,
        &[
            "board",
            "sprint",
            "create",
            "110-0",
            "Sprint 2",
            "--start",
            "2026-10-10",
            "--finish",
            "2026-10-17",
            "--goal",
            "Ship",
        ],
    )
    .await;
    assert!(output.status.success(), "{}", common::stderr(&output));
    assert!(common::stderr(&output).contains("Created sprint Sprint 2 on Web Kanban Board"));
}

#[tokio::test(flavor = "multi_thread")]
async fn sprint_add_and_remove_send_one_request_per_issue() {
    let server = MockServer::start().await;
    mount_board(&server, false).await;
    for id in ["WEB-1", "WEB-2"] {
        Mock::given(method("POST"))
            .and(path("/api/agiles/110-0/sprints/111-0/issues"))
            .and(body_json(json!({"idReadable": id})))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
            .expect(1)
            .mount(&server)
            .await;
    }
    Mock::given(method("GET"))
        .and(path("/api/issues/WEB-1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "2-1"})))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path("/api/agiles/110-0/sprints/111-0/issues/2-1"))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&server)
        .await;
    let added = run(
        &server,
        &[
            "board",
            "sprint",
            "add",
            "110-0",
            "First sprint",
            "WEB-1",
            "WEB-2",
        ],
    )
    .await;
    assert!(added.status.success(), "{}", common::stderr(&added));
    assert!(common::stderr(&added).contains("Added WEB-1, WEB-2 to First sprint"));
    let removed = run(
        &server,
        &[
            "board",
            "sprint",
            "remove",
            "110-0",
            "First sprint",
            "WEB-1",
        ],
    )
    .await;
    assert!(removed.status.success(), "{}", common::stderr(&removed));
    assert!(common::stderr(&removed).contains("Removed WEB-1 from First sprint"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_board_without_columns_has_no_columns_row() {
    let server = MockServer::start().await;
    let mut bare = board("110-4", "Bare", false);
    bare["columnSettings"] = json!({"field": null, "columns": []});
    bare["currentSprint"] = json!(null);
    Mock::given(method("GET"))
        .and(path("/api/agiles"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([bare])))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/agiles/110-4/sprints"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
        .mount(&server)
        .await;
    let output = run(&server, &["board", "show", "Bare"]).await;
    assert!(output.status.success(), "{}", common::stderr(&output));
    assert!(
        !common::stdout(&output).contains("Columns:"),
        "{}",
        common::stdout(&output)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_failed_remove_names_the_issue_typed() {
    let server = MockServer::start().await;
    mount_board(&server, false).await;
    for (issue, entity) in [("WEB-1", "2-1"), ("WEB-3", "2-19")] {
        Mock::given(method("GET"))
            .and(path(format!("/api/issues/{issue}")))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": entity})))
            .expect(1)
            .mount(&server)
            .await;
    }
    Mock::given(method("DELETE"))
        .and(path("/api/agiles/110-0/sprints/111-0/issues/2-1"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path("/api/agiles/110-0/sprints/111-0/issues/2-19"))
        .respond_with(ResponseTemplate::new(404).set_body_json(
            json!({"error": "Not Found", "error_description": "Entity with id 2-19 not found"}),
        ))
        .mount(&server)
        .await;
    let output = run(
        &server,
        &[
            "board",
            "sprint",
            "remove",
            "110-0",
            "First sprint",
            "WEB-1",
            "WEB-3",
        ],
    )
    .await;
    assert_eq!(output.status.code(), Some(1));
    let stderr = common::stderr(&output);
    assert!(stderr.contains("already removed: WEB-1"), "{stderr}");
    assert!(stderr.contains("could not remove WEB-3"), "{stderr}");
}

async fn mount_board_with_issues(server: &MockServer, issues: serde_json::Value) {
    Mock::given(method("GET"))
        .and(path("/api/agiles"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!([board("110-0", "Web", true)])),
        )
        .mount(server)
        .await;
    let sprint = json!({"id": "111-0", "name": "First sprint", "goal": null, "start": null, "finish": null, "archived": false});
    Mock::given(method("GET"))
        .and(path("/api/agiles/110-0/sprints"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([sprint.clone()])))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/agiles/110-0/sprints/111-0"))
        .respond_with(ResponseTemplate::new(200).set_body_json(sprint))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/agiles/110-0/sprints/111-0/issues"))
        .respond_with(ResponseTemplate::new(200).set_body_json(issues))
        .mount(server)
        .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn hidden_resolved_issues_do_not_widen_the_id_column() {
    let server = MockServer::start().await;
    mount_board_with_issues(
        &server,
        json!([
            issue("WEB-1", "Login page", "Procesā"),
            issue("WEB-1234", "Old work", "Darīts")
        ]),
    )
    .await;
    let output = run(&server, &["board", "show", "110-0"]).await;
    assert!(output.status.success(), "{}", common::stderr(&output));
    assert!(
        common::stdout(&output).contains("\nWEB-1  Login page\n"),
        "{}",
        common::stdout(&output)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_current_sprint_is_read_directly() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/agiles"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!([board("110-0", "Web", false)])),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/agiles/110-0/sprints"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
        .expect(0)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/agiles/110-0/sprints/111-0"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({"id": "111-0", "name": "First sprint", "goal": null, "start": null, "finish": null, "archived": false}),
        ))
        .expect(2)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/agiles/110-0/sprints/111-0/issues"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
        .mount(&server)
        .await;
    for args in [["board", "show", "110-0"], ["board", "sprint", "current"]] {
        let mut args = args.to_vec();
        if args[1] == "sprint" {
            args.push("110-0");
        }
        let output = run(&server, &args).await;
        assert!(output.status.success(), "{}", common::stderr(&output));
        assert!(common::stdout(&output).contains("First sprint"));
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_sprint_with_only_a_start_shows_it() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/agiles"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!([board("110-0", "Web", false)])),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/agiles/110-0/sprints"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {"id": "111-5", "name": "Next", "goal": null, "start": 1_791_763_200_000_i64, "finish": null, "archived": false}
        ])))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/agiles/110-0/sprints/111-5/issues"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
        .mount(&server)
        .await;
    let output = run(&server, &["board", "sprint", "show", "110-0", "Next"]).await;
    assert!(output.status.success(), "{}", common::stderr(&output));
    assert!(
        common::stdout(&output).contains("Sprint:    Next (from 2026-10-12)"),
        "{}",
        common::stdout(&output)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn an_empty_sprint_list_says_so() {
    let server = MockServer::start().await;
    let mut bare = board("110-4", "Bare", false);
    bare["currentSprint"] = json!(null);
    Mock::given(method("GET"))
        .and(path("/api/agiles"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([bare])))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/agiles/110-4/sprints"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
        .mount(&server)
        .await;
    let output = run(&server, &["board", "sprint", "list", "Bare"]).await;
    assert!(output.status.success(), "{}", common::stderr(&output));
    assert!(common::stderr(&output).contains("No sprints found."));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_board_without_a_current_sprint_says_so() {
    let server = MockServer::start().await;
    let mut bare = board("110-4", "Bare", false);
    bare["currentSprint"] = json!(null);
    Mock::given(method("GET"))
        .and(path("/api/agiles"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([bare])))
        .mount(&server)
        .await;
    let table = run(&server, &["board", "show", "Bare"]).await;
    assert!(table.status.success(), "{}", common::stderr(&table));
    assert!(common::stderr(&table).contains("The board has no current sprint."));
    let json = run(&server, &["board", "show", "Bare", "--format", "json"]).await;
    let value: serde_json::Value = serde_json::from_slice(&json.stdout).unwrap();
    assert!(value["sprint"].is_null(), "{value}");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unknown_sprint_suggests_the_closest() {
    let server = MockServer::start().await;
    mount_board(&server, true).await;
    let output = run(
        &server,
        &["board", "sprint", "show", "110-0", "First sprnt"],
    )
    .await;
    assert_eq!(output.status.code(), Some(2));
    assert!(
        common::stderr(&output).contains("did you mean 'First sprint'"),
        "{}",
        common::stderr(&output)
    );
}
