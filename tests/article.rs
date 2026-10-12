mod common;

use common::TestEnv;
use serde_json::json;
use wiremock::matchers::{body_json, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

const TOKEN: &str = "perm-YXJ0aWNsZQ==.dGVzdA==.secret123456";

async fn run(server: &MockServer, args: &[&str]) -> std::process::Output {
    let url = server.uri();
    TestEnv::new()
        .run_async(
            args,
            &[("YOUTRACK_URL", url.as_str()), ("YOUTRACK_TOKEN", TOKEN)],
        )
        .await
}

fn row(id: &str, title: &str, parent: Option<&str>, children: &[&str]) -> serde_json::Value {
    json!({
        "idReadable": id,
        "summary": title,
        "project": {"shortName": "KB", "name": "Knowledge"},
        "parentArticle": parent.map(|parent| json!({"idReadable": parent})),
        "childArticles": children.iter().map(|child| json!({"idReadable": child})).collect::<Vec<_>>(),
        "updated": 1_791_590_400_000_i64
    })
}

async fn mount_list(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/api/articles"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            row("KB-A-2", "Install", Some("KB-A-1"), &[]),
            row("KB-A-1", "Guide", None, &["KB-A-2"]),
        ])))
        .mount(server)
        .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn article_list_shows_parents() {
    let server = MockServer::start().await;
    mount_list(&server).await;
    let output = run(&server, &["article", "ls"]).await;
    assert!(output.status.success(), "{}", common::stderr(&output));
    let stdout = common::stdout(&output);
    let lines: Vec<&str> = stdout.lines().collect();
    assert!(lines[0].starts_with("KB-A-2  KB-A-1"), "{stdout}");
    assert!(lines[0].ends_with("Install"), "{stdout}");
    assert!(lines[1].starts_with("KB-A-1 "), "{stdout}");
}

#[tokio::test(flavor = "multi_thread")]
async fn article_list_tree_indents_children() {
    let server = MockServer::start().await;
    mount_list(&server).await;
    let output = run(&server, &["article", "list", "--tree"]).await;
    assert!(output.status.success(), "{}", common::stderr(&output));
    let stdout = common::stdout(&output);
    let lines: Vec<&str> = stdout.lines().collect();
    assert!(lines[0].starts_with("KB-A-1 "), "{stdout}");
    assert!(lines[1].starts_with("  KB-A-2"), "{stdout}");
}

#[tokio::test(flavor = "multi_thread")]
async fn tree_cannot_be_limited() {
    let server = MockServer::start().await;
    let output = run(&server, &["article", "list", "--tree", "-n", "5"]).await;
    assert_eq!(output.status.code(), Some(2));
    assert!(
        common::stderr(&output).contains("--tree"),
        "{}",
        common::stderr(&output)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn article_list_json_is_flat_with_parent_and_children() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/articles"))
        .and(query_param("query", "project: {KB}"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([row(
            "KB-A-1",
            "Guide",
            None,
            &["KB-A-2"]
        )])))
        .mount(&server)
        .await;
    let output = run(
        &server,
        &["article", "list", "-p", "KB", "--format", "json"],
    )
    .await;
    assert!(output.status.success(), "{}", common::stderr(&output));
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        value,
        json!([{"id": "KB-A-1", "title": "Guide", "project": "KB", "parent": null, "children": ["KB-A-2"], "updated": "2026-10-10T00:00:00Z", "url": format!("{}/articles/KB-A-1", server.uri())}])
    );
}

async fn mount_article(server: &MockServer) {
    let mut article = row("KB-A-1", "Guide", None, &["KB-A-2"]);
    article["content"] = json!("# Guide\n\nRead me.");
    article["reporter"] = json!({"login": "jane", "fullName": "Jane"});
    article["created"] = json!(1_791_590_400_000_i64);
    Mock::given(method("GET"))
        .and(path("/api/articles/KB-A-1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(article))
        .mount(server)
        .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn article_show_prints_details_then_the_content() {
    let server = MockServer::start().await;
    mount_article(&server).await;
    let url = format!("{}/articles/KB-A-1", server.uri());
    let output = run(&server, &["article", "show", &url]).await;
    assert!(output.status.success(), "{}", common::stderr(&output));
    let stdout = common::stdout(&output);
    assert!(stdout.starts_with("KB-A-1  Guide\n"), "{stdout}");
    assert!(stdout.contains("Children:  KB-A-2"), "{stdout}");
    assert!(stdout.contains("Author:    jane"), "{stdout}");
    assert!(stdout.trim_end().ends_with("Read me."), "{stdout}");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_issue_id_is_not_an_article_id() {
    let server = MockServer::start().await;
    let output = run(&server, &["article", "show", "KB-1"]).await;
    assert_eq!(output.status.code(), Some(2));
    assert!(common::stderr(&output).contains("not an article ID"));
}

#[tokio::test(flavor = "multi_thread")]
async fn article_create_prints_the_url() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/admin/projects"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {"shortName": "KB", "name": "Knowledge", "leader": null, "description": null, "archived": false}
        ])))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/articles"))
        .and(body_json(
            json!({"project": {"shortName": "KB"}, "summary": "Guide", "content": "Read me."}),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"idReadable": "KB-A-1"})))
        .expect(1)
        .mount(&server)
        .await;
    let output = run(
        &server,
        &[
            "article", "create", "-p", "KB", "-s", "Guide", "-b", "Read me.",
        ],
    )
    .await;
    assert!(output.status.success(), "{}", common::stderr(&output));
    assert_eq!(
        common::stdout(&output),
        format!("{}/articles/KB-A-1\n", server.uri())
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_child_article_takes_its_parents_project() {
    let server = MockServer::start().await;
    mount_article(&server).await;
    Mock::given(method("POST"))
        .and(path("/api/articles"))
        .and(body_json(json!({"project": {"shortName": "KB"}, "summary": "Install", "parentArticle": {"idReadable": "KB-A-1"}})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"idReadable": "KB-A-2"})))
        .expect(1)
        .mount(&server)
        .await;
    let output = run(
        &server,
        &["article", "create", "-s", "Install", "--parent", "KB-A-1"],
    )
    .await;
    assert!(output.status.success(), "{}", common::stderr(&output));
}

#[tokio::test(flavor = "multi_thread")]
async fn article_create_without_a_terminal_needs_the_title() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/admin/projects"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {"shortName": "KB", "name": "Knowledge", "leader": null, "description": null, "archived": false}
        ])))
        .mount(&server)
        .await;
    let output = run(&server, &["article", "create", "-p", "KB"]).await;
    assert_eq!(output.status.code(), Some(2));
    assert!(
        common::stderr(&output).contains("-s"),
        "{}",
        common::stderr(&output)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn article_edit_sends_only_the_changes() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/articles/KB-A-2"))
        .and(body_json(
            json!({"summary": "Installing", "parentArticle": null}),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .expect(1)
        .mount(&server)
        .await;
    let output = run(
        &server,
        &[
            "article",
            "edit",
            "KB-A-2",
            "-s",
            "Installing",
            "--parent",
            "",
        ],
    )
    .await;
    assert!(output.status.success(), "{}", common::stderr(&output));
    assert!(common::stderr(&output).contains("Updated KB-A-2"));
}

#[tokio::test(flavor = "multi_thread")]
async fn article_edit_without_changes_or_a_terminal_is_refused() {
    let server = MockServer::start().await;
    let output = run(&server, &["article", "edit", "KB-A-2"]).await;
    assert_eq!(output.status.code(), Some(2));
    assert!(common::stderr(&output).contains("nothing to change"));
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn article_edit_refuses_a_blank_title() {
    let server = MockServer::start().await;
    let output = run(&server, &["article", "edit", "KB-A-2", "-s", " "]).await;
    assert_eq!(output.status.code(), Some(2));
    assert!(common::stderr(&output).contains("title cannot be empty"));
}

#[tokio::test(flavor = "multi_thread")]
async fn article_delete_names_the_children_and_needs_yes() {
    let server = MockServer::start().await;
    mount_article(&server).await;
    mount_tree_article(&server, "KB-A-2", &[]).await;
    let output = run(&server, &["article", "delete", "KB-A-1"]).await;
    assert_eq!(output.status.code(), Some(2));
    assert!(
        common::stderr(&output)
            .contains("refusing to delete KB-A-1 (also deletes child articles: KB-A-2)"),
        "{}",
        common::stderr(&output)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn article_delete_with_yes_deletes() {
    let server = MockServer::start().await;
    mount_article(&server).await;
    mount_tree_article(&server, "KB-A-2", &[]).await;
    Mock::given(method("DELETE"))
        .and(path("/api/articles/KB-A-1"))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&server)
        .await;
    let output = run(&server, &["article", "delete", "KB-A-1", "--yes"]).await;
    assert!(output.status.success(), "{}", common::stderr(&output));
    assert!(common::stderr(&output).contains("Deleted KB-A-1"));
}

#[tokio::test(flavor = "multi_thread")]
async fn article_comments_go_to_the_article() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/articles/KB-A-1/comments"))
        .and(body_json(json!({"text": "Thanks"})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "190-0", "text": "Thanks", "author": {"login": "jane", "fullName": "Jane"}, "created": 1, "updated": null
        })))
        .expect(1)
        .mount(&server)
        .await;
    let output = run(
        &server,
        &["article", "comment", "add", "KB-A-1", "-b", "Thanks"],
    )
    .await;
    assert!(output.status.success(), "{}", common::stderr(&output));
    assert_eq!(
        common::stdout(&output),
        format!(
            "{}/articles/KB-A-1#focus=Comments-190-0.0-0\n",
            server.uri()
        )
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn article_attachments_are_listed_from_the_article() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/articles/KB-A-1/attachments"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {"id": "8-1", "name": "diagram.png", "size": 10, "mimeType": "image/png", "author": {"login": "jane", "fullName": "Jane"}, "created": 1, "url": "/api/files/8-1"}
        ])))
        .mount(&server)
        .await;
    let output = run(
        &server,
        &["article", "attachment", "ls", "KB-A-1", "--format", "json"],
    )
    .await;
    assert!(output.status.success(), "{}", common::stderr(&output));
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value[0]["name"], "diagram.png");
}

async fn mount_tree_article(server: &MockServer, id: &str, children: &[&str]) {
    let mut article = row(id, id, None, children);
    article["content"] = json!("");
    article["reporter"] = json!(null);
    article["created"] = json!(0);
    Mock::given(method("GET"))
        .and(path(format!("/api/articles/{id}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(article))
        .mount(server)
        .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn deleting_a_parent_with_its_child_sends_one_delete() {
    let server = MockServer::start().await;
    mount_tree_article(&server, "KB-A-1", &["KB-A-2"]).await;
    mount_tree_article(&server, "KB-A-2", &[]).await;
    Mock::given(method("DELETE"))
        .and(path("/api/articles/KB-A-1"))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path("/api/articles/KB-A-2"))
        .respond_with(ResponseTemplate::new(404))
        .expect(0)
        .mount(&server)
        .await;
    let output = run(
        &server,
        &["article", "delete", "KB-A-1", "KB-A-2", "KB-A-1", "--yes"],
    )
    .await;
    assert!(output.status.success(), "{}", common::stderr(&output));
    assert!(common::stderr(&output).contains("Deleted KB-A-1, KB-A-2"));
}

#[tokio::test(flavor = "multi_thread")]
async fn the_delete_confirmation_names_every_descendant() {
    let server = MockServer::start().await;
    mount_tree_article(&server, "KB-A-1", &["KB-A-2"]).await;
    mount_tree_article(&server, "KB-A-2", &["KB-A-3"]).await;
    mount_tree_article(&server, "KB-A-3", &[]).await;
    let output = run(&server, &["article", "delete", "KB-A-1"]).await;
    assert_eq!(output.status.code(), Some(2));
    assert!(
        common::stderr(&output)
            .contains("refusing to delete KB-A-1 (also deletes child articles: KB-A-2, KB-A-3)"),
        "{}",
        common::stderr(&output)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn article_create_checks_the_project_before_writing() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/admin/projects"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {"shortName": "KB", "name": "Knowledge", "leader": null, "description": null, "archived": false}
        ])))
        .mount(&server)
        .await;
    let output = run(
        &server,
        &[
            "article", "create", "-p", "KBB", "-s", "Guide", "-b", "Text",
        ],
    )
    .await;
    assert_eq!(output.status.code(), Some(2));
    assert!(
        common::stderr(&output).contains("did you mean 'KB'"),
        "{}",
        common::stderr(&output)
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
async fn comment_and_attachment_errors_name_the_right_command() {
    let server = MockServer::start().await;
    for parent in ["issues/WEB-1", "articles/KB-A-1"] {
        Mock::given(method("GET"))
            .and(path(format!("/api/{parent}/attachments")))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
            .mount(&server)
            .await;
    }
    let issue = run(
        &server,
        &["issue", "comment", "delete", "WEB-1", "../x", "--yes"],
    )
    .await;
    assert!(
        common::stderr(&issue).contains("`yt issue comment list` shows them"),
        "{}",
        common::stderr(&issue)
    );
    let article = run(
        &server,
        &["article", "comment", "delete", "KB-A-1", "../x", "--yes"],
    )
    .await;
    assert!(
        common::stderr(&article).contains("`yt article comment list` shows them"),
        "{}",
        common::stderr(&article)
    );
    let issue = run(
        &server,
        &["issue", "attachment", "download", "WEB-1", "x.bin"],
    )
    .await;
    assert!(
        common::stderr(&issue).contains("the issue has none"),
        "{}",
        common::stderr(&issue)
    );
    let article = run(
        &server,
        &["article", "attachment", "download", "KB-A-1", "x.bin"],
    )
    .await;
    assert!(
        common::stderr(&article).contains("the article has none"),
        "{}",
        common::stderr(&article)
    );
}

#[test]
fn article_edit_without_changes_is_a_usage_error_even_when_signed_out() {
    let env = TestEnv::new();
    env.write_config("[profiles.acme]\nurl = \"https://acme.youtrack.cloud\"\nauth = \"oauth\"\n");
    let output = env.run(&["article", "edit", "KB-A-2"]);
    assert_eq!(output.status.code(), Some(2), "{}", common::stderr(&output));
    assert!(common::stderr(&output).contains("nothing to change"));
}

#[tokio::test(flavor = "multi_thread")]
async fn latvian_titles_and_content_round_trip() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/articles"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            row("KB-A-1", "Ēdienkarte", None, &["KB-A-2"]),
            row("KB-A-2", "Zupas un ķiploki", Some("KB-A-1"), &[]),
        ])))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/admin/projects"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {"shortName": "KB", "name": "Zināšanas", "leader": null, "description": null, "archived": false}
        ])))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/articles"))
        .and(body_json(json!({"project": {"shortName": "KB"}, "summary": "Ēdienkarte", "content": "Gaļa — ļoti garšīga"})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"idReadable": "KB-A-3"})))
        .expect(1)
        .mount(&server)
        .await;
    let tree = run(&server, &["article", "list", "--tree"]).await;
    let stdout = common::stdout(&tree);
    assert!(
        stdout
            .lines()
            .nth(1)
            .is_some_and(|line| line.starts_with("  KB-A-2") && line.ends_with("Zupas un ķiploki")),
        "{stdout}"
    );
    let created = run(
        &server,
        &[
            "article",
            "create",
            "-p",
            "kb",
            "-s",
            "Ēdienkarte",
            "-b",
            "Gaļa — ļoti garšīga",
        ],
    )
    .await;
    assert!(created.status.success(), "{}", common::stderr(&created));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_failed_delete_names_what_was_already_deleted() {
    let server = MockServer::start().await;
    mount_tree_article(&server, "KB-A-1", &[]).await;
    mount_tree_article(&server, "KB-A-5", &[]).await;
    Mock::given(method("DELETE"))
        .and(path("/api/articles/KB-A-1"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path("/api/articles/KB-A-5"))
        .respond_with(
            ResponseTemplate::new(400)
                .set_body_json(json!({"error": "Bad", "error_description": "Locked"})),
        )
        .mount(&server)
        .await;
    let output = run(&server, &["article", "delete", "KB-A-1", "KB-A-5", "--yes"]).await;
    assert_eq!(output.status.code(), Some(1));
    assert!(
        common::stderr(&output).contains("already deleted: KB-A-1"),
        "{}",
        common::stderr(&output)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn article_create_without_a_project_or_terminal_asks_for_p() {
    let server = MockServer::start().await;
    let output = run(&server, &["article", "create", "-s", "Guide", "-b", "Text"]).await;
    assert_eq!(output.status.code(), Some(2));
    assert!(
        common::stderr(&output).contains("-p"),
        "{}",
        common::stderr(&output)
    );
}
