use serde::Deserialize;
use serde_json::{Value, json};

use crate::api::{Client, Projection};
use crate::error::Result;

/// How YouTrack reads one command, from `commands/assist`.
#[derive(Clone, Debug, PartialEq)]
pub struct ParsedCommand {
    pub description: String,
    pub error: bool,
}

#[derive(Deserialize)]
struct Assist {
    #[serde(default)]
    commands: Vec<RawCommand>,
}

#[derive(Deserialize)]
struct RawCommand {
    #[serde(default)]
    description: String,
    #[serde(default)]
    error: bool,
}

impl Projection for Assist {
    const FIELDS: &'static str = "commands(description,error)";
}

fn body(issues: &[String], command: &str) -> Value {
    json!({
        "query": command,
        "issues": issues.iter().map(|id| json!({"idReadable": id})).collect::<Vec<_>>(),
    })
}

/// Applies `command` to every issue in one request.
pub async fn apply(client: &Client, issues: &[String], command: &str) -> Result<()> {
    client.post_unit("commands", &body(issues, command)).await
}

/// How YouTrack would read `command`, without applying it.
pub async fn assist(
    client: &Client,
    issues: &[String],
    command: &str,
) -> Result<Vec<ParsedCommand>> {
    let mut request = body(issues, command);
    request["caret"] = json!(command.chars().count());
    let assist: Assist = client.post_query("commands/assist", &request).await?;
    Ok(assist
        .commands
        .into_iter()
        .map(|raw| ParsedCommand {
            description: strip_html(&raw.description),
            error: raw.error,
        })
        .collect())
}

/// Removes the markup YouTrack puts in command descriptions (`Set State <span class="bold">Fixed</span>`).
pub fn strip_html(text: &str) -> String {
    let mut plain = String::with_capacity(text.len());
    let mut in_tag = false;
    for c in text.chars() {
        match c {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            _ if !in_tag => plain.push(c),
            _ => {}
        }
    }
    plain
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use wiremock::matchers::{body_json, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::api::test_client;

    #[tokio::test]
    async fn apply_sends_the_command_for_every_issue() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/commands"))
            .and(body_json(json!({
                "query": "State In Progress",
                "issues": [{"idReadable": "WEB-1"}, {"idReadable": "WEB-2"}]
            })))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;
        apply(
            &test_client(&server.uri()),
            &["WEB-1".into(), "WEB-2".into()],
            "State In Progress",
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn assist_reports_how_the_command_reads() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/commands/assist"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"commands": [
                {"description": "Set State <span class=\"bold\">In Progress</span>", "error": false},
                {"description": "Unknown command: Stat", "error": true}
            ]})))
            .mount(&server)
            .await;
        let parsed = assist(
            &test_client(&server.uri()),
            &["WEB-1".into()],
            "State In Progress",
        )
        .await
        .unwrap();
        assert_eq!(
            parsed,
            vec![
                ParsedCommand {
                    description: "Set State In Progress".into(),
                    error: false
                },
                ParsedCommand {
                    description: "Unknown command: Stat".into(),
                    error: true
                },
            ]
        );
    }

    #[test]
    fn markup_and_entities_are_removed() {
        assert_eq!(strip_html("a <b>&lt;b&gt;</b> &amp; c"), "a <b> & c");
    }
}
