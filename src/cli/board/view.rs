use serde_json::json;
use yt::api::Client;
use yt::api::agiles::{Board, Sprint};
use yt::output::Format;

use super::{board_json, sprint_json};
use crate::cli::Ctx;

/// A board summary, then `sprint`'s issues grouped by column; resolved columns list their issues only with `resolved`.
pub(crate) async fn show_sprint(
    ctx: &Ctx,
    client: &Client,
    board: &Board,
    sprint: Option<&Sprint>,
    resolved: bool,
) -> yt::Result<()> {
    let instance = client.instance();
    let issues = match sprint {
        Some(sprint) => yt::api::agiles::sprint_issues(client, &board.id, &sprint.id).await?,
        None => Vec::new(),
    };
    let groups = yt::api::agiles::group_by_column(board, &issues);
    if ctx.output.format() != Format::Table {
        let mut value = board_json(instance, board);
        value["sprint"] = sprint.map(sprint_json).into();
        value["columns"] = groups
            .iter()
            .map(|group| {
                json!({
                    "name": group.name,
                    "resolved": group.resolved,
                    "issues": group.issues.iter().map(|issue| json!({
                        "id": issue.id_readable,
                        "summary": issue.summary,
                        "url": instance.issue_url(&issue.id_readable).to_string(),
                    })).collect::<Vec<_>>(),
                })
            })
            .collect::<Vec<_>>()
            .into();
        return if ctx.output.format() == Format::Json {
            ctx.output.json(&value)
        } else {
            ctx.output.ndjson(&value)
        };
    }
    let mut rows: Vec<(String, String)> = Vec::new();
    let projects: Vec<&str> = board
        .projects
        .iter()
        .map(|project| project.short_name.as_str())
        .collect();
    rows.push(("Projects".into(), projects.join(", ")));
    if let Some(settings) = board
        .column_settings
        .as_ref()
        .filter(|settings| !settings.columns.is_empty())
    {
        let columns: Vec<&str> = settings
            .columns
            .iter()
            .map(|column| column.presentation.as_str())
            .collect();
        let field = settings
            .field
            .as_ref()
            .map(|field| format!("{}: ", field.name))
            .unwrap_or_default();
        rows.push(("Columns".into(), format!("{field}{}", columns.join(", "))));
    }
    if let Some(field) = board
        .swimlane_settings
        .as_ref()
        .filter(|settings| settings.enabled)
        .and_then(|settings| settings.field.as_ref())
    {
        rows.push(("Swimlanes".into(), field.name.clone()));
    }
    rows.push((
        "Sprints".into(),
        if board.sprints_enabled() { "on" } else { "off" }.into(),
    ));
    if let Some(sprint) = sprint {
        let mut text = sprint.name.clone();
        if let Some(dates) = yt::api::agiles::sprint_dates(sprint) {
            text.push_str(&format!(" ({dates})"));
        }
        rows.push(("Sprint".into(), text));
        if let Some(goal) = sprint.goal.as_deref().filter(|goal| !goal.is_empty()) {
            rows.push(("Goal".into(), goal.to_string()));
        }
    }
    rows.push(("URL".into(), instance.board_url(&board.id).to_string()));
    let mut text = crate::cli::details(&board.name, rows, None);
    let width = groups
        .iter()
        .filter(|group| resolved || !group.resolved)
        .flat_map(|group| group.issues.iter())
        .map(|issue| issue.id_readable.chars().count())
        .max()
        .unwrap_or(0);
    for group in &groups {
        let name = group.name.as_deref().unwrap_or("Other");
        text.push_str("\n\n");
        if group.resolved && !resolved {
            text.push_str(&format!("{name} ({}, resolved)", group.issues.len()));
            continue;
        }
        text.push_str(&format!("{name} ({})", group.issues.len()));
        for issue in &group.issues {
            text.push_str(&format!(
                "\n{:<width$}  {}",
                issue.id_readable,
                issue.summary.as_deref().unwrap_or("")
            ));
        }
    }
    if sprint.is_none() {
        ctx.note("The board has no current sprint.");
    }
    ctx.output.write(&text)
}
