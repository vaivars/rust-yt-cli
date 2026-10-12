mod delete;
mod edit;
mod list;
mod log;
mod types;

use serde_json::{Value, json};
use usage_rs::{Args, Subcommands};
use yt::YtError;
use yt::api::Client;
use yt::api::projects::WorkItemType;
use yt::api::work_items::WorkItem;

/// Log and list spent time
#[derive(Args)]
#[usage(run_async_with)]
pub(crate) struct TimeGroup {
    #[usage(subcommand)]
    command: TimeCommand,
}

#[derive(Subcommands)]
#[usage(run_async_with)]
pub(crate) enum TimeCommand {
    /// Log time spent on an issue
    Log(log::Log),
    /// List work items, newest first
    #[usage(visible_alias = "ls")]
    List(list::List),
    /// Change a work item
    Edit(edit::Edit),
    /// Delete work items
    Delete(delete::Delete),
    /// List work item types
    Types(types::Types),
}

pub(crate) fn work_item_json(item: &WorkItem) -> Value {
    json!({
        "id": item.id,
        "issue": item.issue.as_ref().map(|issue| issue.id_readable.as_str()),
        "date": yt::time::date(item.date),
        "duration": item.duration.as_ref().map(|duration| json!({
            "minutes": duration.minutes,
            "presentation": duration.presentation,
        })),
        "type": item.work_type.as_ref().map(|work_type| work_type.name.as_str()),
        "author": item.author.as_ref().map(|author| author.login.as_str()),
        "text": item.text,
    })
}

/// A duration argument as YouTrack's presentation text (`1h 30m`).
pub(crate) fn duration(value: &str) -> yt::Result<String> {
    if let Some(period) = yt::fields::period(value) {
        return Ok(period);
    }
    let trimmed = value.trim();
    let hint = if !trimmed.is_empty() && trimmed.chars().all(|c| c.is_ascii_digit()) {
        format!("; write {trimmed}m for minutes")
    } else {
        String::new()
    };
    Err(YtError::Usage {
        message: format!("'{value}' is not a duration like 1h 30m or 1w 2d 3h 30m{hint}"),
    }
    .into())
}

/// A `YYYY-MM-DD` argument as midnight UTC in milliseconds.
pub(crate) fn day(flag: &str, value: &str) -> yt::Result<i64> {
    yt::time::day_millis(value).ok_or_else(|| {
        YtError::Usage {
            message: format!("{flag}: '{value}' is not a date (YYYY-MM-DD)"),
        }
        .into()
    })
}

/// The work item type named `name` in the issue's project.
pub(crate) async fn work_type(
    client: &Client,
    issue: &str,
    name: &str,
) -> yt::Result<WorkItemType> {
    let project = yt::api::issues::project(client, issue).await?;
    let types = yt::api::work_items::types(client, Some(&project)).await?;
    Ok(yt::api::work_items::resolve_type(&types, name)?.clone())
}
