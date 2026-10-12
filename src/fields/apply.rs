use serde_json::{Value, json};

use crate::api::{Client, commands, issues};
use crate::error::Result;
use crate::fields::plan::Operation;

/// Runs `operations` in order and stops at the first failure; the error then says what was already applied.
pub async fn execute(client: &Client, operations: &[Operation]) -> Result<()> {
    let mut applied: Vec<String> = Vec::new();
    for operation in operations {
        let result = match operation {
            Operation::Command { issues, command } => {
                commands::apply(client, issues, command).await
            }
            Operation::Update { issue, body, .. } => issues::update(client, issue, body).await,
        };
        if let Err(error) = result {
            return Err(if applied.is_empty() {
                error
            } else {
                error.wrap_err(format!(
                    "stopped after a failure; already applied: {}",
                    applied.join("; ")
                ))
            });
        }
        applied.push(operation.describe());
    }
    Ok(())
}

/// An operation and the problems YouTrack sees in it.
#[derive(Clone, Debug, PartialEq)]
pub struct Planned {
    pub operation: Operation,
    pub problems: Vec<String>,
}

impl Planned {
    pub fn json(&self) -> Value {
        match &self.operation {
            Operation::Command { issues, command } => json!({
                "kind": "command",
                "issues": issues,
                "command": command,
                "problems": self.problems,
            }),
            Operation::Update { issue, body, .. } => json!({
                "kind": "update",
                "issues": [issue],
                "body": body,
                "problems": self.problems,
            }),
        }
    }
}

/// Checks commands with `commands/assist` without applying anything; REST updates have nothing to check.
pub async fn check(client: &Client, operations: &[Operation]) -> Result<Vec<Planned>> {
    let mut planned = Vec::new();
    for operation in operations {
        let problems = match operation {
            Operation::Command { issues, command } => commands::assist(client, issues, command)
                .await?
                .into_iter()
                .filter(|parsed| parsed.error)
                .map(|parsed| parsed.description)
                .collect(),
            Operation::Update { .. } => Vec::new(),
        };
        planned.push(Planned {
            operation: operation.clone(),
            problems,
        });
    }
    Ok(planned)
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::api::test_client;

    fn operations() -> Vec<Operation> {
        vec![
            Operation::Update {
                issue: "WEB-1".into(),
                body: json!({"summary": "x"}),
                summary: "set the summary".into(),
            },
            Operation::Command {
                issues: vec!["WEB-1".into()],
                command: "Priority Major".into(),
            },
        ]
    }

    #[tokio::test]
    async fn operations_run_in_order() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/issues/WEB-1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/api/commands"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;
        execute(&test_client(&server.uri()), &operations())
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn a_failure_says_what_was_already_applied() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/issues/WEB-1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/api/commands"))
            .respond_with(
                ResponseTemplate::new(400)
                    .set_body_json(json!({"error": "bad", "error_description": "Unknown command"})),
            )
            .mount(&server)
            .await;
        let error = execute(&test_client(&server.uri()), &operations())
            .await
            .unwrap_err();
        let text = format!("{error}");
        assert!(
            text.contains("stopped after a failure; already applied: WEB-1: set the summary"),
            "{text}"
        );
        assert_eq!(crate::error::exit_code(&error), 1);
    }

    #[tokio::test]
    async fn a_dry_run_asks_youtrack_about_commands_and_changes_nothing() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/commands/assist"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({"commands": [
                    {"description": "Unknown value <b>Majr</b>", "error": true}
                ]})),
            )
            .expect(1)
            .mount(&server)
            .await;
        let client = test_client(&server.uri());
        let planned = check(&client, &operations()).await.unwrap();
        assert_eq!(planned[0].problems, Vec::<String>::new());
        assert_eq!(planned[1].problems, ["Unknown value Majr"]);
        assert!(!client.has_written());
        assert_eq!(
            planned[1].json(),
            json!({"kind": "command", "issues": ["WEB-1"], "command": "Priority Major", "problems": ["Unknown value Majr"]})
        );
    }
}
