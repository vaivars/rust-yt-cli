use serde_json::json;
use yt::api::Client;
use yt::fields::apply::{self, Planned};
use yt::fields::plan::Operation;
use yt::output::Format;

use super::json::issue_json;
use crate::cli::Ctx;

/// Prints what `operations` would do, checking commands with YouTrack; fails when YouTrack sees a problem.
pub(crate) async fn print_dry_run(
    ctx: &Ctx,
    client: &Client,
    operations: &[Operation],
) -> yt::Result<()> {
    let planned = apply::check(client, operations).await?;
    print_planned(ctx, &[], &planned)
}

/// Prints `steps` (text and JSON) and then `planned`; fails when YouTrack saw a problem.
pub(crate) fn print_planned(
    ctx: &Ctx,
    steps: &[(String, serde_json::Value)],
    planned: &[Planned],
) -> yt::Result<()> {
    if ctx.output.format() == Format::Table {
        let mut lines: Vec<String> = steps.iter().map(|(text, _)| text.clone()).collect();
        for item in planned {
            lines.push(item.operation.describe());
            lines.extend(
                item.problems
                    .iter()
                    .map(|problem| format!("  problem: {problem}")),
            );
        }
        ctx.output.write(&lines.join("\n"))?;
    } else {
        ctx.output.json(&json!({
            "operations": steps
                .iter()
                .map(|(_, value)| value.clone())
                .chain(planned.iter().map(Planned::json))
                .collect::<Vec<_>>()
        }))?;
    }
    if planned.iter().any(|item| !item.problems.is_empty()) {
        return Err(miette::miette!(
            "YouTrack reported problems; nothing was changed"
        ));
    }
    Ok(())
}

/// Reports changed issues: a note on stderr in table mode, the issues themselves in JSON.
pub(crate) async fn report_updated(ctx: &Ctx, client: &Client, ids: &[String]) -> yt::Result<()> {
    let format = ctx.output.format();
    if format == Format::Table {
        ctx.note(&format!("Updated {}", ids.join(", ")));
        return Ok(());
    }
    let mut issues = Vec::new();
    for id in ids {
        issues.push(yt::api::issues::get(client, id).await?);
    }
    let values: Vec<_> = issues.iter().map(issue_json).collect();
    if format == Format::Json {
        ctx.output.json(&values)
    } else {
        values.iter().try_for_each(|value| ctx.output.ndjson(value))
    }
}

/// Reports one changed issue: a note on stderr in table mode, the issue object in JSON.
pub(crate) async fn report_issue(ctx: &Ctx, client: &Client, id: &str) -> yt::Result<()> {
    let format = ctx.output.format();
    if format == Format::Table {
        ctx.note(&format!("Updated {id}"));
        return Ok(());
    }
    let issue = yt::api::issues::get(client, id).await?;
    if format == Format::Json {
        ctx.output.json(&issue_json(&issue))
    } else {
        ctx.output.ndjson(&issue_json(&issue))
    }
}
