mod common;

use common::TestEnv;
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const TOKEN: &str = "perm-YXR0YWNo.dGVzdA==.secret123456";

async fn run(env: &TestEnv, server: &MockServer, args: &[&str]) -> std::process::Output {
    let url = server.uri();
    env.run_async(
        args,
        &[("YOUTRACK_URL", url.as_str()), ("YOUTRACK_TOKEN", TOKEN)],
    )
    .await
}

async fn mount_list(server: &MockServer, attachments: serde_json::Value) {
    Mock::given(method("GET"))
        .and(path("/api/issues/WEB-1/attachments"))
        .respond_with(ResponseTemplate::new(200).set_body_json(attachments))
        .mount(server)
        .await;
}

fn data_bin() -> serde_json::Value {
    json!([{"id": "1-1", "name": "data.bin", "size": 7, "mimeType": "application/octet-stream",
            "author": {"login": "jane"}, "created": 0, "url": "/api/files/1-1?sign=x"}])
}

const BYTES: [u8; 7] = [0, 159, 146, 150, 255, 10, 13];

async fn mount_file(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/api/files/1-1"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(BYTES.to_vec()))
        .mount(server)
        .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn attachment_list_shows_names_and_sizes() {
    let server = MockServer::start().await;
    mount_list(&server, data_bin()).await;
    let output = run(
        &TestEnv::new(),
        &server,
        &["issue", "attachment", "ls", "WEB-1"],
    )
    .await;
    assert!(output.status.success(), "{}", common::stderr(&output));
    let stdout = common::stdout(&output);
    assert!(
        stdout.contains("data.bin") && stdout.contains("jane"),
        "{stdout}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn upload_sends_the_files_and_names_them() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/issues/WEB-1/attachments"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!([{"id": "1-2", "name": "log.txt", "size": 3}])),
        )
        .expect(1)
        .mount(&server)
        .await;
    let env = TestEnv::new();
    std::fs::write(env.path().join("log.txt"), "abc").unwrap();
    let output = run(
        &env,
        &server,
        &["issue", "attachment", "upload", "WEB-1", "log.txt"],
    )
    .await;
    assert!(output.status.success(), "{}", common::stderr(&output));
    assert!(common::stderr(&output).contains("Uploaded log.txt to WEB-1"));
}

#[tokio::test(flavor = "multi_thread")]
async fn download_writes_the_file_byte_for_byte() {
    let server = MockServer::start().await;
    mount_list(&server, data_bin()).await;
    mount_file(&server).await;
    let env = TestEnv::new();
    let output = run(
        &env,
        &server,
        &["issue", "attachment", "download", "WEB-1", "data.bin"],
    )
    .await;
    assert!(output.status.success(), "{}", common::stderr(&output));
    assert_eq!(std::fs::read(env.path().join("data.bin")).unwrap(), BYTES);
    assert!(common::stderr(&output).contains("Saved"));
}

#[tokio::test(flavor = "multi_thread")]
async fn download_does_not_overwrite_without_yes() {
    let server = MockServer::start().await;
    mount_list(&server, data_bin()).await;
    mount_file(&server).await;
    let env = TestEnv::new();
    std::fs::write(env.path().join("data.bin"), "keep").unwrap();
    let refused = run(
        &env,
        &server,
        &["issue", "attachment", "download", "WEB-1", "data.bin"],
    )
    .await;
    assert_eq!(refused.status.code(), Some(2));
    assert_eq!(
        std::fs::read_to_string(env.path().join("data.bin")).unwrap(),
        "keep"
    );
    let output = run(
        &env,
        &server,
        &[
            "issue",
            "attachment",
            "download",
            "WEB-1",
            "data.bin",
            "--yes",
        ],
    )
    .await;
    assert!(output.status.success(), "{}", common::stderr(&output));
    assert_eq!(std::fs::read(env.path().join("data.bin")).unwrap(), BYTES);
}

#[tokio::test(flavor = "multi_thread")]
async fn download_to_stdout_writes_raw_bytes() {
    let server = MockServer::start().await;
    mount_list(&server, data_bin()).await;
    mount_file(&server).await;
    let output = run(
        &TestEnv::new(),
        &server,
        &[
            "issue",
            "attachment",
            "download",
            "WEB-1",
            "data.bin",
            "-o",
            "-",
            "--format",
            "json",
        ],
    )
    .await;
    assert!(output.status.success(), "{}", common::stderr(&output));
    assert_eq!(output.stdout, BYTES);
}

#[tokio::test(flavor = "multi_thread")]
async fn delete_needs_yes_and_finds_by_name() {
    let server = MockServer::start().await;
    mount_list(&server, data_bin()).await;
    Mock::given(method("DELETE"))
        .and(path("/api/issues/WEB-1/attachments/1-1"))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&server)
        .await;
    let env = TestEnv::new();
    let refused = run(
        &env,
        &server,
        &["issue", "attachment", "delete", "WEB-1", "data.bin"],
    )
    .await;
    assert_eq!(refused.status.code(), Some(2));
    let output = run(
        &env,
        &server,
        &[
            "issue",
            "attachment",
            "delete",
            "WEB-1",
            "data.bin",
            "--yes",
        ],
    )
    .await;
    assert!(output.status.success(), "{}", common::stderr(&output));
}

#[tokio::test(flavor = "multi_thread")]
async fn download_into_a_directory_keeps_the_attachment_name() {
    let server = MockServer::start().await;
    mount_list(&server, data_bin()).await;
    mount_file(&server).await;
    let env = TestEnv::new();
    std::fs::create_dir(env.path().join("out")).unwrap();
    let output = run(
        &env,
        &server,
        &[
            "issue",
            "attachment",
            "download",
            "WEB-1",
            "data.bin",
            "-o",
            "out",
        ],
    )
    .await;
    assert!(output.status.success(), "{}", common::stderr(&output));
    assert_eq!(
        std::fs::read(env.path().join("out").join("data.bin")).unwrap(),
        BYTES
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_failed_save_names_the_path() {
    let server = MockServer::start().await;
    mount_list(&server, data_bin()).await;
    mount_file(&server).await;
    let output = run(
        &TestEnv::new(),
        &server,
        &[
            "issue",
            "attachment",
            "download",
            "WEB-1",
            "data.bin",
            "-o",
            "missing/x.bin",
        ],
    )
    .await;
    assert!(!output.status.success());
    assert!(
        common::stderr(&output).contains("missing/x.bin"),
        "{}",
        common::stderr(&output)
    );
}

fn named(name: &str) -> serde_json::Value {
    json!([{"id": "1-1", "name": name, "size": 7, "mimeType": "text/plain",
            "author": {"login": "jane"}, "created": 0, "url": "/api/files/1-1?sign=x"}])
}

#[tokio::test(flavor = "multi_thread")]
async fn a_hidden_attachment_name_needs_an_explicit_path() {
    let server = MockServer::start().await;
    mount_list(&server, named(".yt.toml")).await;
    mount_file(&server).await;
    let env = TestEnv::new();
    let download = ["issue", "attachment", "download", "WEB-1", ".yt.toml"];
    let refused = run(&env, &server, &[&download[..], &["--yes"]].concat()).await;
    assert_eq!(
        refused.status.code(),
        Some(2),
        "{}",
        common::stderr(&refused)
    );
    assert!(common::stderr(&refused).contains("-o PATH"));
    assert!(!env.path().join(".yt.toml").exists());
    let output = run(
        &env,
        &server,
        &[&download[..], &["-o", "saved.toml"]].concat(),
    )
    .await;
    assert!(output.status.success(), "{}", common::stderr(&output));
    assert_eq!(std::fs::read(env.path().join("saved.toml")).unwrap(), BYTES);
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn download_never_writes_through_a_symbolic_link() {
    let server = MockServer::start().await;
    mount_list(&server, data_bin()).await;
    mount_file(&server).await;
    let env = TestEnv::new();
    std::fs::write(env.path().join("target.txt"), "keep").unwrap();
    std::os::unix::fs::symlink(env.path().join("target.txt"), env.path().join("data.bin")).unwrap();
    std::os::unix::fs::symlink(
        env.path().join("created.txt"),
        env.path().join("dangling.bin"),
    )
    .unwrap();
    let download = [
        "issue",
        "attachment",
        "download",
        "WEB-1",
        "data.bin",
        "--yes",
    ];
    let through_link = run(&env, &server, &download).await;
    assert_eq!(
        through_link.status.code(),
        Some(2),
        "{}",
        common::stderr(&through_link)
    );
    assert_eq!(
        std::fs::read_to_string(env.path().join("target.txt")).unwrap(),
        "keep"
    );
    let dangling = run(
        &env,
        &server,
        &[&download[..], &["-o", "dangling.bin"]].concat(),
    )
    .await;
    assert_eq!(
        dangling.status.code(),
        Some(2),
        "{}",
        common::stderr(&dangling)
    );
    assert!(!env.path().join("created.txt").exists());
}
