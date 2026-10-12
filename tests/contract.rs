//! Runs yt against a real instance. Needs YT_CONTRACT_URL, YT_CONTRACT_TOKEN and YT_CONTRACT_PROJECT; the
//! project needs YouTrack's default fields and the versions 1.0 and 1.1. Every test deletes what it creates.

mod common;

use std::process::Output;

use common::TestEnv;
use serde_json::Value;

struct Contract {
    env: TestEnv,
    url: String,
    token: String,
    project: String,
}

#[allow(
    clippy::disallowed_methods,
    reason = "contract tests read their target from the environment"
)]
fn contract() -> Option<Contract> {
    let get = |key: &str| std::env::var(key).ok().filter(|value| !value.is_empty());
    match (
        get("YT_CONTRACT_URL"),
        get("YT_CONTRACT_TOKEN"),
        get("YT_CONTRACT_PROJECT"),
    ) {
        (Some(url), Some(token), Some(project)) => Some(Contract {
            env: TestEnv::new(),
            url,
            token,
            project,
        }),
        _ => {
            #[expect(clippy::disallowed_macros, reason = "test output")]
            {
                eprintln!(
                    "skipped: set YT_CONTRACT_URL, YT_CONTRACT_TOKEN and YT_CONTRACT_PROJECT"
                );
            }
            None
        }
    }
}

impl Contract {
    fn yt(&self, args: &[&str]) -> Output {
        self.env.run_with(
            args,
            &[
                ("YOUTRACK_URL", self.url.as_str()),
                ("YOUTRACK_TOKEN", self.token.as_str()),
            ],
        )
    }

    fn ok(&self, args: &[&str]) -> String {
        let output = self.yt(args);
        assert!(
            output.status.success(),
            "yt {args:?}: {}",
            common::stderr(&output)
        );
        common::stdout(&output)
    }

    fn json(&self, args: &[&str]) -> Value {
        let mut args = args.to_vec();
        args.extend(["--format", "json"]);
        serde_json::from_str(&self.ok(&args)).unwrap()
    }

    /// Creates an issue that is deleted when the guard drops.
    fn issue(&self, test: &str) -> Issue<'_> {
        let summary = format!("yt contract {test}");
        let created = self.json(&["issue", "create", "-p", &self.project, "-s", &summary]);
        Issue {
            contract: self,
            id: created["id"].as_str().unwrap().to_string(),
        }
    }

    fn fields(&self, id: &str) -> Value {
        self.json(&["issue", "show", id])["fields"].clone()
    }
}

struct Issue<'a> {
    contract: &'a Contract,
    id: String,
}

impl Drop for Issue<'_> {
    fn drop(&mut self) {
        let _ = self.contract.yt(&["issue", "delete", &self.id, "--yes"]);
    }
}

#[test]
fn field_changes_round_trip() {
    let Some(contract) = contract() else { return };
    let issue = contract.issue("fields");
    let id = issue.id.as_str();
    contract.ok(&[
        "issue",
        "edit",
        id,
        "-f",
        "Priority=Major",
        "-f",
        "State=In Progress",
        "-f",
        "Assignee=me",
        "-f",
        "Fix versions=1.0",
        "-f",
        "Fix versions=1.1",
    ]);
    let fields = contract.fields(id);
    assert_eq!(fields["Priority"], "Major");
    assert_eq!(fields["State"], "In Progress");
    assert!(fields["Assignee"].is_string());
    assert_eq!(fields["Fix versions"], serde_json::json!(["1.0", "1.1"]));
    contract.ok(&["issue", "edit", id, "-f", "Fix versions-=1.0"]);
    assert_eq!(
        contract.fields(id)["Fix versions"],
        serde_json::json!(["1.1"])
    );
    contract.ok(&["issue", "edit", id, "-f", "Fix versions+=1.0"]);
    assert_eq!(
        contract.fields(id)["Fix versions"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    contract.ok(&[
        "issue",
        "edit",
        id,
        "-f",
        "Assignee=",
        "-f",
        "Fix versions=",
    ]);
    let fields = contract.fields(id);
    assert!(fields["Assignee"].is_null());
    assert!(fields["Fix versions"].is_null());
}

#[test]
fn summary_description_and_commands_apply() {
    let Some(contract) = contract() else { return };
    let issue = contract.issue("text");
    contract.ok(&[
        "issue",
        "edit",
        &issue.id,
        "-s",
        "yt contract renamed",
        "-b",
        "Two words",
    ]);
    contract.ok(&["issue", "command", "Priority Critical", &issue.id]);
    let shown = contract.json(&["issue", "show", &issue.id]);
    assert_eq!(shown["summary"], "yt contract renamed");
    assert_eq!(shown["description"], "Two words");
    assert_eq!(shown["fields"]["Priority"], "Critical");
}

#[test]
fn tags_are_added_created_and_removed() {
    let Some(contract) = contract() else { return };
    let issue = contract.issue("tags");
    let tag = "yt contract tag";
    let _cleanup = TagCleanup {
        contract: &contract,
        name: tag,
    };
    contract.ok(&["issue", "tag", "add", &issue.id, tag, "--no-validate"]);
    assert!(
        contract.json(&["issue", "show", &issue.id])["tags"]
            .as_array()
            .unwrap()
            .contains(&Value::from(tag))
    );
    contract.ok(&["issue", "tag", "remove", &issue.id, tag]);
    assert!(
        contract.json(&["issue", "show", &issue.id])["tags"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

/// Deletes the tag a test created, matched by its exact name, even when the test fails.
struct TagCleanup<'a> {
    contract: &'a Contract,
    name: &'a str,
}

impl Drop for TagCleanup<'_> {
    fn drop(&mut self) {
        let listed = self.contract.yt(&["api", "tags?fields=id,name&$top=1000"]);
        let Ok(tags) = serde_json::from_slice::<Value>(&listed.stdout) else {
            return;
        };
        for tag in tags.as_array().into_iter().flatten() {
            if tag["name"] == self.name
                && let Some(id) = tag["id"].as_str()
            {
                let _ = self
                    .contract
                    .yt(&["api", "-X", "DELETE", &format!("tags/{id}"), "--yes"]);
            }
        }
    }
}

#[test]
fn links_are_added_and_removed() {
    let Some(contract) = contract() else { return };
    let a = contract.issue("link source");
    let b = contract.issue("link target");
    contract.ok(&["issue", "link", "add", &a.id, "relates to", &b.id]);
    let links = contract.json(&["issue", "link", "list", &a.id]);
    assert_eq!(links[0]["issue"]["id"], b.id.as_str());
    contract.ok(&["issue", "link", "remove", &a.id, "relates to", &b.id]);
    assert!(
        contract
            .json(&["issue", "link", "list", &a.id])
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn comments_are_added_edited_and_deleted() {
    let Some(contract) = contract() else { return };
    let issue = contract.issue("comments");
    let added = contract.json(&["issue", "comment", "add", &issue.id, "-b", "First"]);
    let comment = added["id"].as_str().unwrap().to_string();
    contract.ok(&[
        "issue", "comment", "edit", &issue.id, &comment, "-b", "Edited",
    ]);
    assert_eq!(
        contract.json(&["issue", "comment", "list", &issue.id])[0]["text"],
        "Edited"
    );
    contract.ok(&["issue", "comment", "delete", &issue.id, &comment, "--yes"]);
    assert!(
        contract
            .json(&["issue", "comment", "list", &issue.id])
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn attachments_round_trip() {
    let Some(contract) = contract() else { return };
    let issue = contract.issue("attachments");
    let bytes: Vec<u8> = (0..=255).collect();
    std::fs::write(contract.env.path().join("bytes.bin"), &bytes).unwrap();
    contract.ok(&["issue", "attachment", "upload", &issue.id, "bytes.bin"]);
    std::fs::remove_file(contract.env.path().join("bytes.bin")).unwrap();
    contract.ok(&["issue", "attachment", "download", &issue.id, "bytes.bin"]);
    assert_eq!(
        std::fs::read(contract.env.path().join("bytes.bin")).unwrap(),
        bytes
    );
    contract.ok(&[
        "issue",
        "attachment",
        "delete",
        &issue.id,
        "bytes.bin",
        "--yes",
    ]);
    assert!(
        contract
            .json(&["issue", "attachment", "list", &issue.id])
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn a_new_issue_gets_its_fields_and_state() {
    let Some(contract) = contract() else { return };
    let created = contract.json(&[
        "issue",
        "create",
        "-p",
        &contract.project,
        "-s",
        "yt contract create with fields",
        "-f",
        "Priority=Major",
        "-f",
        "State=In Progress",
        "-f",
        "Fix versions=1.0",
    ]);
    let issue = Issue {
        contract: &contract,
        id: created["id"].as_str().unwrap().to_string(),
    };
    let fields = contract.fields(&issue.id);
    assert_eq!(fields["Priority"], "Major");
    assert_eq!(fields["State"], "In Progress");
    assert_eq!(fields["Fix versions"], serde_json::json!(["1.0"]));
}

#[test]
fn delete_removes_the_issue() {
    let Some(contract) = contract() else { return };
    let created = contract.json(&[
        "issue",
        "create",
        "-p",
        &contract.project,
        "-s",
        "yt contract delete",
    ]);
    let id = created["id"].as_str().unwrap();
    contract.ok(&["issue", "delete", id, "--yes"]);
    assert!(!contract.yt(&["issue", "show", id]).status.success());
}

#[test]
fn projects_are_listed_shown_and_described() {
    let Some(contract) = contract() else { return };
    let listed = contract.json(&["project", "list", "--all"]);
    assert!(
        listed
            .as_array()
            .unwrap()
            .iter()
            .any(|project| project["short_name"] == contract.project.as_str())
    );
    let shown = contract.json(&["project", "show", &contract.project.to_lowercase()]);
    assert_eq!(shown["short_name"], contract.project.as_str());
    assert_eq!(shown["time_tracking"], true);
    let fields = contract.json(&["project", "fields", &contract.project]);
    assert!(
        fields
            .as_array()
            .unwrap()
            .iter()
            .any(|field| field["name"] == "Priority")
    );
}

#[test]
fn time_is_logged_listed_edited_and_deleted() {
    let Some(contract) = contract() else { return };
    let issue = contract.issue("time");
    let id = issue.id.as_str();
    let types = contract.json(&["time", "types", "-p", &contract.project]);
    let first = types[0]["name"].as_str().unwrap().to_string();
    let logged = contract.json(&[
        "time",
        "log",
        id,
        "1h 30m",
        "--date",
        "2026-01-05",
        "--type",
        &first,
        "-b",
        "contract",
    ]);
    assert_eq!(logged["duration"]["minutes"], 90);
    assert_eq!(logged["date"], "2026-01-05");
    assert_eq!(logged["type"], first.as_str());
    let item = logged["id"].as_str().unwrap().to_string();
    contract.ok(&["time", "edit", id, &item, "--duration", "2h"]);
    let listed = contract.json(&["time", "list", id, "--author", "all"]);
    assert_eq!(listed.as_array().unwrap().len(), 1);
    assert_eq!(listed[0]["duration"]["minutes"], 120);
    contract.ok(&["time", "delete", id, &item, "--yes"]);
    let listed = contract.json(&["time", "list", id, "--author", "all"]);
    assert!(listed.as_array().unwrap().is_empty());
}

/// Deletes an article the test created, even when the test fails.
struct ArticleCleanup<'a> {
    contract: &'a Contract,
    id: String,
}

impl Drop for ArticleCleanup<'_> {
    fn drop(&mut self) {
        let _ = self.contract.yt(&["article", "delete", &self.id, "--yes"]);
    }
}

#[test]
fn articles_comments_and_attachments_round_trip() {
    let Some(contract) = contract() else { return };
    let parent = contract.json(&[
        "article",
        "create",
        "-p",
        &contract.project,
        "-s",
        "yt contract parent",
        "-b",
        "Parent",
    ]);
    let parent = ArticleCleanup {
        contract: &contract,
        id: parent["id"].as_str().unwrap().to_string(),
    };
    let child = contract.json(&[
        "article",
        "create",
        "-s",
        "yt contract child",
        "--parent",
        &parent.id,
    ]);
    let child = ArticleCleanup {
        contract: &contract,
        id: child["id"].as_str().unwrap().to_string(),
    };
    assert_eq!(
        contract.json(&["article", "show", &child.id])["parent"],
        parent.id.as_str()
    );
    contract.ok(&["article", "edit", &child.id, "--parent", ""]);
    assert!(contract.json(&["article", "show", &child.id])["parent"].is_null());
    let comment = contract.json(&["article", "comment", "add", &child.id, "-b", "Hi"]);
    assert!(comment["url"].as_str().unwrap().contains("/articles/"));
    let file = contract.env.path().join("note.txt");
    std::fs::write(&file, "note").unwrap();
    contract.ok(&[
        "article",
        "attachment",
        "upload",
        &child.id,
        file.to_str().unwrap(),
    ]);
    let attachments = contract.json(&["article", "attachment", "list", &child.id]);
    assert_eq!(attachments[0]["name"], "note.txt");
    let listed = contract.json(&["article", "list", "-p", &contract.project, "--all"]);
    assert!(
        listed
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["id"] == child.id.as_str())
    );
}

/// Deletes a board the test created, even when the test fails.
struct BoardCleanup<'a> {
    contract: &'a Contract,
    id: String,
}

impl Drop for BoardCleanup<'_> {
    fn drop(&mut self) {
        let _ = self.contract.yt(&[
            "api",
            "-X",
            "DELETE",
            &format!("agiles/{}", self.id),
            "--yes",
        ]);
    }
}

#[test]
fn boards_and_sprints_round_trip() {
    let Some(contract) = contract() else { return };
    let projects = contract.json(&[
        "api",
        &format!("admin/projects/{}?fields=id", contract.project),
    ]);
    let body = serde_json::json!({
        "name": "yt contract board",
        "projects": [{"id": projects["id"]}],
        "sprintsSettings": {"disableSprints": false}
    })
    .to_string();
    let output = contract
        .env
        .command()
        .args(["api", "-X", "POST", "agiles?fields=id", "--input", "-"])
        .env("YOUTRACK_URL", &contract.url)
        .env("YOUTRACK_TOKEN", &contract.token)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            use std::io::Write as _;
            child.stdin.take().unwrap().write_all(body.as_bytes())?;
            child.wait_with_output()
        })
        .unwrap();
    assert!(output.status.success());
    let created: Value = serde_json::from_slice(&output.stdout).unwrap();
    let board = BoardCleanup {
        contract: &contract,
        id: created["id"].as_str().unwrap().to_string(),
    };
    let issue = contract.issue("sprint");
    contract.ok(&[
        "board",
        "sprint",
        "create",
        &board.id,
        "yt contract sprint",
        "--goal",
        "probe",
    ]);
    contract.ok(&[
        "board",
        "sprint",
        "add",
        &board.id,
        "yt contract sprint",
        &issue.id,
    ]);
    let shown = contract.json(&["board", "sprint", "show", &board.id, "yt contract sprint"]);
    let ids: Vec<&str> = shown["columns"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|column| column["issues"].as_array().unwrap().iter())
        .map(|issue| issue["id"].as_str().unwrap())
        .collect();
    assert!(ids.contains(&issue.id.as_str()), "{shown}");
    contract.ok(&[
        "board",
        "sprint",
        "remove",
        &board.id,
        "yt contract sprint",
        &issue.id,
    ]);
    let shown = contract.json(&["board", "sprint", "show", &board.id, "yt contract sprint"]);
    assert!(
        shown["columns"]
            .as_array()
            .unwrap()
            .iter()
            .all(|column| column["issues"].as_array().unwrap().is_empty()),
        "{shown}"
    );
    let listed = contract.json(&["board", "list"]);
    assert!(
        listed
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["id"] == board.id.as_str())
    );
}
