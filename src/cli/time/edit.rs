use usage_rs::{Args, RunAsyncWith};
use yt::YtError;
use yt::api::work_items::WorkItemChange;
use yt::output::Format;

use super::{day, duration, work_item_json, work_type};
use crate::cli::Ctx;
use crate::cli::issue::text;

/// Change a work item
#[derive(Args)]
#[usage(effect = "write")]
pub(crate) struct Edit {
    /// Issue ID or URL
    #[usage(arg, name = "ID")]
    id: String,
    /// Work item ID, as `yt time list` shows it
    #[usage(arg, name = "ITEM")]
    item: String,
    /// New time spent, e.g. `2h`
    #[usage(long, value_name = "DURATION")]
    duration: Option<String>,
    /// New day, YYYY-MM-DD
    #[usage(long, value_name = "DATE")]
    date: Option<String>,
    /// New work item type
    #[usage(long = "type", value_name = "TYPE")]
    work_type: Option<String>,
    /// New text (Markdown)
    #[usage(short, long, value_name = "TEXT")]
    body: Option<String>,
    /// Read the text from a file, or `-` for stdin
    #[usage(long, value_name = "PATH")]
    body_file: Option<String>,
    /// Show the change without applying it
    #[usage(long)]
    dry_run: bool,
}

impl RunAsyncWith<&Ctx> for Edit {
    type Output = yt::Result<()>;

    async fn run_async_with(self, ctx: &Ctx) -> Self::Output {
        yt::api::work_items::check_item(&self.item)?;
        let duration = self.duration.as_deref().map(duration).transpose()?;
        let date = match &self.date {
            Some(value) => Some(day("--date", value)?),
            None => None,
        };
        let text = text::main_text(self.body, self.body_file.as_deref())
            .await?
            .map(|text| text.trim_end().to_string());
        if duration.is_none() && date.is_none() && self.work_type.is_none() && text.is_none() {
            return Err(YtError::Usage {
                message: "nothing to change: pass --duration, --date, --type, -b or --body-file"
                    .into(),
            }
            .into());
        }
        let instance = ctx.instance()?;
        let id = instance.issue_id(&self.id)?;
        let mut parts = Vec::new();
        if let Some(duration) = &duration {
            parts.push(format!("duration {duration}"));
        }
        if let Some(date) = date {
            parts.push(format!("date {}", yt::time::date(date)));
        }
        let mut change = WorkItemChange {
            date,
            duration,
            type_id: None,
            text,
        };
        if let Some(name) = &self.work_type {
            let client = ctx.client().await?;
            let found = work_type(client, &id, name).await?;
            parts.push(format!("type {}", found.name));
            change.type_id = Some(found.id);
        }
        if change.text.is_some() {
            parts.push("text".into());
        }
        if self.dry_run {
            return ctx.output.write(&format!(
                "{id}: change work item {}: {}",
                self.item,
                parts.join(", ")
            ));
        }
        let client = ctx.client().await?;
        let item = yt::api::work_items::update(client, &id, &self.item, &change).await?;
        match ctx.output.format() {
            Format::Table => {
                ctx.note(&format!("Updated work item {} on {id}", item.id));
                Ok(())
            }
            Format::Json => ctx.output.json(&work_item_json(&item)),
            Format::Ndjson => ctx.output.ndjson(&work_item_json(&item)),
        }
    }
}
