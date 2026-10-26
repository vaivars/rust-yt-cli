mod common;

use std::io::Write as _;
use std::process::Stdio;

use common::TestEnv;
use serde_json::{Value, json};
use wiremock::matchers::{any, body_json, method, path, path_regex, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

const TOKEN: &str = "perm-YXBw.dGVzdA==.secret123456";
const META: &str = "/api/extensionEndpoints/hidden-fields/meta/openapi.json";
const GLOBAL: &str = "/api/extensionEndpoints/hidden-fields/global/issue";
const ATTACHED: &str = "/api/issues/WEB-1/extensionEndpoints/hidden-fields/api/issue";
const DESCRIPTION: &str = include_str!("fixtures/apps/hidden-fields.openapi.json");
const NO_HANDLER: &str = r#"{"error":"Not Found","error_description":"HTTP handler not found by path hidden-fields/meta/openapi.json"}"#;

async fn silent_server() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(any())
        .respond_with(ResponseTemplate::new(500))
        .expect(0)
        .mount(&server)
        .await;
    server
}

/// A profile `acme` on the mock server, signed in through `YOUTRACK_TOKEN`.
fn env_for(server: &MockServer, apps: &str) -> TestEnv {
    let env = TestEnv::new();
    env.write_config(&format!(
        "# my yt config\n[profiles.acme] # work\nurl = \"{}\"\nauth = \"token\"\n{apps}",
        server.uri()
    ));
    env
}

fn enabled(server: &MockServer) -> TestEnv {
    env_for(server, "apps = [\"hidden-fields\"]\n")
}

async fn run(env: &TestEnv, args: &[&str]) -> std::process::Output {
    env.run_async(args, &[("YOUTRACK_TOKEN", TOKEN)]).await
}

fn attached_only() -> String {
    let mut document: Value = serde_json::from_str(DESCRIPTION).unwrap();
    document["paths"].as_object_mut().unwrap().remove(GLOBAL);
    document.to_string()
}

async fn describe(server: &MockServer, document: &str) {
    Mock::given(method("GET"))
        .and(path(META))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw(document.to_string(), "application/json"),
        )
        .mount(server)
        .await;
}

fn issue_body() -> Value {
    json!({"idReadable": "WEB-1", "customFields": [{"value": "EXT-42", "name": "HF External ID", "$type": "SimpleIssueCustomField"}], "$type": "Issue"})
}

fn cached(env: &TestEnv) -> bool {
    fn walk(dir: &std::path::Path) -> bool {
        std::fs::read_dir(dir)
            .into_iter()
            .flatten()
            .flatten()
            .any(|entry| {
                let path = entry.path();
                path.ends_with("apps/hidden-fields.json") || (path.is_dir() && walk(&path))
            })
    }
    walk(&env.path().join("cache"))
}

#[tokio::test(flavor = "multi_thread")]
async fn root_typos_keep_their_error_and_send_nothing() {
    let server = silent_server().await;
    let url = server.uri();
    let vars = [("YOUTRACK_URL", url.as_str()), ("YOUTRACK_TOKEN", TOKEN)];
    let env = TestEnv::new();
    let cases: [(&[&str], &str); 2] = [
        (
            &["isue"],
            "error: unrecognized subcommand 'isue'\n\n  tip: a similar subcommand exists: 'issue'\n\nUsage: yt [FLAGS] <SUBCOMMAND>\n\nFor more information, try '--help'.\n",
        ),
        (
            &["hidden-fields", "get-issue", "WEB-1"],
            "error: unrecognized subcommand 'hidden-fields'\n\nUsage: yt [FLAGS] <SUBCOMMAND>\n\nFor more information, try '--help'.\n",
        ),
    ];
    for (args, expected) in cases {
        let output = env.run_async(args, &vars).await;
        assert_eq!(output.status.code(), Some(2), "{args:?}");
        assert_eq!(common::stderr(&output), expected, "{args:?}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn without_apps_nothing_else_is_requested() {
    let server = MockServer::start().await;
    Mock::given(path_regex("extensionEndpoints"))
        .respond_with(ResponseTemplate::new(500))
        .expect(0)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/issues/WEB-1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"idReadable": "WEB-1"})))
        .expect(1)
        .mount(&server)
        .await;
    let env = env_for(&server, "");
    let output = run(&env, &["api", "issues/WEB-1"]).await;
    assert!(output.status.success(), "{}", common::stderr(&output));
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn words_that_are_not_enabled_apps_are_usage_errors_without_requests() {
    let server = silent_server().await;
    let env = env_for(&server, "");
    let cases: [(&[&str], &str); 4] = [
        (&["app", "hiden-fields"], "did you mean 'hidden-fields'?"),
        (&["app", "lst"], "did you mean 'list'?"),
        (&["app", "nope", "x"], "run `yt app list`"),
        (
            &["app", "hidden-fields", "get-issue", "WEB-1"],
            "run `yt app add hidden-fields --profile acme`",
        ),
    ];
    for (args, expected) in cases {
        let output = run(&env, args).await;
        assert_eq!(output.status.code(), Some(2), "{args:?}");
        let stderr = common::stderr(&output);
        assert!(stderr.contains(expected), "{args:?}: {stderr}");
    }
    let output = run(&env, &["app", "list"]).await;
    assert!(output.status.success(), "{}", common::stderr(&output));
    assert!(common::stdout(&output).contains("hidden-fields"));
}

#[tokio::test(flavor = "multi_thread")]
async fn add_checks_the_app_and_enables_it() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(META))
        .respond_with(ResponseTemplate::new(200).set_body_raw(DESCRIPTION, "application/json"))
        .expect(1)
        .mount(&server)
        .await;
    let env = env_for(&server, "");
    let output = run(&env, &["app", "add", "hidden-fields"]).await;
    assert!(output.status.success(), "{}", common::stderr(&output));
    assert!(
        common::stderr(&output)
            .contains("Enabled hidden-fields 0.0.1 for profile acme: get-issue, update-issue"),
        "{}",
        common::stderr(&output)
    );
    let config = env.read_config();
    assert!(config.contains("apps = [\"hidden-fields\"]"), "{config}");
    assert!(
        config.contains("# my yt config") && config.contains("# work"),
        "{config}"
    );
    let output = run(&env, &["app", "list", "--format", "json"]).await;
    let list: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(list[0]["id"], "hidden-fields");
    assert_eq!(list[0]["enabled"], true);
}

#[tokio::test(flavor = "multi_thread")]
async fn add_leaves_the_config_alone_when_the_app_is_not_installed() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(META))
        .respond_with(ResponseTemplate::new(404).set_body_raw(NO_HANDLER, "application/json"))
        .mount(&server)
        .await;
    let env = env_for(&server, "");
    let before = env.read_config();
    let output = run(&env, &["app", "add", "hidden-fields"]).await;
    assert_eq!(output.status.code(), Some(1));
    assert!(
        common::stderr(&output).contains("is not installed"),
        "{}",
        common::stderr(&output)
    );
    assert_eq!(env.read_config(), before);
    let output = run(&env, &["app", "add", "nope"]).await;
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn remove_disables_the_app_and_drops_its_cache() {
    let server = MockServer::start().await;
    describe(&server, DESCRIPTION).await;
    let env = env_for(&server, "");
    assert!(
        run(&env, &["app", "add", "hidden-fields"])
            .await
            .status
            .success()
    );
    assert!(cached(&env));
    let output = run(&env, &["app", "remove", "hidden-fields"]).await;
    assert!(output.status.success(), "{}", common::stderr(&output));
    assert!(!env.read_config().contains("apps"), "{}", env.read_config());
    assert!(!cached(&env));
}

#[tokio::test(flavor = "multi_thread")]
async fn get_issue_calls_the_app_wide_endpoint_and_caches_the_description() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(META))
        .respond_with(ResponseTemplate::new(200).set_body_raw(DESCRIPTION, "application/json"))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(GLOBAL))
        .and(query_param("issueId", "WEB-1"))
        .and(query_param("fields", "HF External ID"))
        .respond_with(ResponseTemplate::new(200).set_body_json(issue_body()))
        .expect(2)
        .mount(&server)
        .await;
    let env = enabled(&server);
    let issue_url = format!("{}/issue/WEB-1/some-title", server.uri());
    for issue in ["WEB-1", issue_url.as_str()] {
        let output = run(
            &env,
            &[
                "app",
                "hidden-fields",
                "get-issue",
                issue,
                "--fields",
                "HF External ID",
                "--format",
                "json",
                "-q",
            ],
        )
        .await;
        assert!(output.status.success(), "{}", common::stderr(&output));
        let value: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(value, issue_body());
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn with_only_per_project_endpoints_the_command_uses_them() {
    let server = MockServer::start().await;
    describe(&server, &attached_only()).await;
    Mock::given(method("GET"))
        .and(path(ATTACHED))
        .respond_with(ResponseTemplate::new(200).set_body_json(issue_body()))
        .expect(1)
        .mount(&server)
        .await;
    let output = run(
        &enabled(&server),
        &["app", "hidden-fields", "get-issue", "WEB-1"],
    )
    .await;
    assert!(output.status.success(), "{}", common::stderr(&output));
}

#[tokio::test(flavor = "multi_thread")]
async fn update_issue_sends_the_body_once() {
    let server = MockServer::start().await;
    describe(&server, DESCRIPTION).await;
    let cases = [
        (
            vec!["--field", "Estimate", "--value", r#"{"presentation":"1d"}"#],
            json!({"field": "Estimate", "value": {"presentation": "1d"}}),
        ),
        (
            vec!["--field", "Priority", "--value", "Major"],
            json!({"field": "Priority", "value": "Major"}),
        ),
        (
            vec!["--field", "Estimate", "--value", "null"],
            json!({"field": "Estimate", "value": null}),
        ),
    ];
    let env = enabled(&server);
    for (flags, body) in cases {
        let mock = Mock::given(method("POST"))
            .and(path(GLOBAL))
            .and(query_param("issueId", "WEB-1"))
            .and(body_json(&body))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"idReadable": "WEB-1", "status": "updated"})),
            )
            .expect(1)
            .mount_as_scoped(&server)
            .await;
        let mut args = vec!["app", "hidden-fields", "update-issue", "WEB-1"];
        args.extend(flags);
        let output = run(&env, &args).await;
        assert!(
            output.status.success(),
            "{body}: {}",
            common::stderr(&output)
        );
        drop(mock);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn update_issue_reads_the_body_from_stdin() {
    let server = MockServer::start().await;
    describe(&server, DESCRIPTION).await;
    Mock::given(method("POST"))
        .and(path(GLOBAL))
        .and(body_json(
            json!({"field": "LLM Session", "value": {"text": "where I stopped"}}),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"status": "updated"})))
        .expect(1)
        .mount(&server)
        .await;
    let env = enabled(&server);
    let url = server.uri();
    let mut child = env
        .command()
        .args([
            "app",
            "hidden-fields",
            "update-issue",
            "WEB-1",
            "--input",
            "-",
        ])
        .env("YOUTRACK_TOKEN", TOKEN)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(br#"{"field": "LLM Session", "value": {"text": "where I stopped"}}"#)
        .unwrap();
    let output = tokio::task::spawn_blocking(move || child.wait_with_output().unwrap())
        .await
        .unwrap();
    assert!(
        output.status.success(),
        "{url}: {}",
        common::stderr(&output)
    );
}

/// The first description lists both scopes; after a refresh, only the per-project one.
async fn disabled_after_first_description(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path(META))
        .respond_with(ResponseTemplate::new(200).set_body_raw(DESCRIPTION, "application/json"))
        .up_to_n_times(1)
        .with_priority(1)
        .expect(1)
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path(META))
        .respond_with(ResponseTemplate::new(200).set_body_raw(attached_only(), "application/json"))
        .expect(1)
        .mount(server)
        .await;
    Mock::given(path(GLOBAL))
        .respond_with(ResponseTemplate::new(403).set_body_raw(
            r#"{"error":"disabled","error_description":"The app-wide REST endpoints are disabled in the app settings"}"#,
            "application/json",
        ))
        .expect(1)
        .mount(server)
        .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_read_from_a_disabled_endpoint_moves_to_one_still_on() {
    let server = MockServer::start().await;
    disabled_after_first_description(&server).await;
    Mock::given(method("GET"))
        .and(path(ATTACHED))
        .respond_with(ResponseTemplate::new(200).set_body_json(issue_body()))
        .expect(1)
        .mount(&server)
        .await;
    let output = run(
        &enabled(&server),
        &["app", "hidden-fields", "get-issue", "WEB-1"],
    )
    .await;
    assert!(output.status.success(), "{}", common::stderr(&output));
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value, issue_body());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_read_moved_to_a_project_without_the_app_says_so() {
    let server = MockServer::start().await;
    disabled_after_first_description(&server).await;
    Mock::given(method("GET"))
        .and(path(ATTACHED))
        .respond_with(ResponseTemplate::new(404).set_body_raw(
            r#"{"error":"Not Found","error_description":"HTTP handler not found by path hidden-fields/api/issue"}"#,
            "application/json",
        ))
        .expect(1)
        .mount(&server)
        .await;
    let output = run(
        &enabled(&server),
        &["app", "hidden-fields", "get-issue", "WEB-1"],
    )
    .await;
    assert_eq!(output.status.code(), Some(1));
    let stderr = common::stderr(&output);
    assert!(
        stderr.contains("the hidden-fields app is not attached to this issue's project"),
        "{stderr}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_write_to_a_disabled_endpoint_is_never_sent_again() {
    let server = MockServer::start().await;
    disabled_after_first_description(&server).await;
    Mock::given(path(ATTACHED))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&server)
        .await;
    let output = run(
        &enabled(&server),
        &[
            "app",
            "hidden-fields",
            "update-issue",
            "WEB-1",
            "--field",
            "X",
            "--value",
            "1",
        ],
    )
    .await;
    assert_eq!(output.status.code(), Some(1));
    let stderr = common::stderr(&output);
    assert!(
        stderr.contains("disabled this endpoint") && stderr.contains("yt app hidden-fields --help"),
        "{stderr}"
    );
}

fn without_updates() -> String {
    let mut document: Value = serde_json::from_str(DESCRIPTION).unwrap();
    for item in document["paths"].as_object_mut().unwrap().values_mut() {
        item.as_object_mut().unwrap().remove("post");
    }
    document.to_string()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_command_the_cached_description_lacks_is_looked_up_once_more() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(META))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw(without_updates(), "application/json"),
        )
        .up_to_n_times(1)
        .with_priority(1)
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(META))
        .respond_with(ResponseTemplate::new(200).set_body_raw(DESCRIPTION, "application/json"))
        .expect(2)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path(GLOBAL))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"status": "updated"})))
        .expect(1)
        .mount(&server)
        .await;
    let env = env_for(&server, "");
    let output = run(&env, &["app", "add", "hidden-fields"]).await;
    assert!(
        common::stderr(&output).contains(": get-issue\n"),
        "{}",
        common::stderr(&output)
    );
    let output = run(
        &env,
        &[
            "app",
            "hidden-fields",
            "update-issue",
            "WEB-1",
            "--field",
            "X",
            "--value",
            "1",
        ],
    )
    .await;
    assert!(output.status.success(), "{}", common::stderr(&output));
    let output = run(&env, &["app", "hidden-fields", "get-isue", "WEB-1"]).await;
    assert_eq!(output.status.code(), Some(2));
    assert!(
        common::stderr(&output).contains("unrecognized subcommand 'get-isue'"),
        "{}",
        common::stderr(&output)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_project_without_the_app_is_not_asked_twice() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(META))
        .respond_with(ResponseTemplate::new(200).set_body_raw(attached_only(), "application/json"))
        .expect(2)
        .mount(&server)
        .await;
    Mock::given(path(ATTACHED))
        .respond_with(ResponseTemplate::new(404).set_body_raw(
            r#"{"error":"Not Found","error_description":"HTTP handler not found by path hidden-fields/api/issue"}"#,
            "application/json",
        ))
        .expect(1)
        .mount(&server)
        .await;
    let output = run(
        &enabled(&server),
        &["app", "hidden-fields", "get-issue", "WEB-1"],
    )
    .await;
    assert_eq!(output.status.code(), Some(1));
    let stderr = common::stderr(&output);
    assert!(
        stderr.contains("the hidden-fields app is not attached to this issue's project"),
        "{stderr}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn an_uninstalled_app_fails_alone() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(META))
        .respond_with(ResponseTemplate::new(200).set_body_raw(DESCRIPTION, "application/json"))
        .up_to_n_times(1)
        .with_priority(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(META))
        .respond_with(ResponseTemplate::new(404).set_body_raw(NO_HANDLER, "application/json"))
        .mount(&server)
        .await;
    Mock::given(path(GLOBAL))
        .respond_with(ResponseTemplate::new(404).set_body_raw(
            r#"{"error":"Not Found","error_description":"HTTP handler not found by path hidden-fields/global/issue"}"#,
            "application/json",
        ))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/issues/WEB-1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"idReadable": "WEB-1"})))
        .mount(&server)
        .await;
    let env = enabled(&server);
    let output = run(&env, &["app", "hidden-fields", "get-issue", "WEB-1"]).await;
    assert_eq!(output.status.code(), Some(1));
    assert!(
        common::stderr(&output).contains("is not installed"),
        "{}",
        common::stderr(&output)
    );
    assert!(!cached(&env));
    let output = run(&env, &["api", "issues/WEB-1"]).await;
    assert!(output.status.success(), "{}", common::stderr(&output));
}

#[tokio::test(flavor = "multi_thread")]
async fn help_lists_the_apps_commands() {
    let server = MockServer::start().await;
    describe(&server, DESCRIPTION).await;
    let env = enabled(&server);
    let output = run(&env, &["app", "hidden-fields", "--help"]).await;
    assert!(output.status.success(), "{}", common::stderr(&output));
    let page = common::stdout(&output);
    assert!(
        page.contains("get-issue") && page.contains("update-issue"),
        "{page}"
    );
    let output = run(&env, &["app", "hidden-fields", "get-issue", "--help"]).await;
    assert!(output.status.success(), "{}", common::stderr(&output));
    assert!(common::stdout(&output).contains("Issue ID (WEB-42) or issue URL"));
    let output = run(&env, &["app", "hidden-fields", "get-issue"]).await;
    assert_eq!(output.status.code(), Some(2));
    assert!(
        common::stderr(&output).contains("<ISSUE>"),
        "{}",
        common::stderr(&output)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn help_is_coloured_as_yt_decides() {
    let server = MockServer::start().await;
    describe(&server, DESCRIPTION).await;
    let env = enabled(&server);
    for (flags, coloured) in [(&[][..], true), (&["--no-color"][..], false)] {
        let mut command = env.command();
        command
            .args(["app", "hidden-fields", "get-issue", "--help"])
            .args(flags)
            .env("YOUTRACK_TOKEN", TOKEN)
            .env_remove("NO_COLOR")
            .env("CLICOLOR_FORCE", "1");
        let output = tokio::task::spawn_blocking(move || command.output().unwrap())
            .await
            .unwrap();
        assert!(output.status.success(), "{}", common::stderr(&output));
        let page = common::stdout(&output);
        assert_eq!(page.contains("\u{1b}["), coloured, "{flags:?}: {page:?}");
        assert!(!page.contains('\u{fffd}'), "{flags:?}: {page:?}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn commands_complete_from_the_cache_without_the_server() {
    let server = MockServer::start().await;
    describe(&server, DESCRIPTION).await;
    let env = env_for(&server, "");
    assert!(
        run(&env, &["app", "add", "hidden-fields"])
            .await
            .status
            .success()
    );
    drop(server);
    let complete = |line: &str| {
        common::stdout(&env.run(&["__complete_word__", "--shell", "fish", "--line", line]))
    };
    let text = complete("yt app ");
    for word in ["list", "add", "remove", "hidden-fields"] {
        assert!(text.contains(word), "{text}");
    }
    let text = complete("yt app hidden-fields ");
    assert!(
        text.contains("get-issue") && text.contains("update-issue"),
        "{text}"
    );
    let text = complete("yt app hidden-fields get-issue --f");
    assert!(
        text.contains("--fields") && text.contains("--full"),
        "{text}"
    );
    let text = complete("yt app hidden-fields get-issue WEB-1 --profile ");
    assert!(text.contains("acme"), "{text}");
}
