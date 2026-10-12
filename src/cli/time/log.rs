use serde_json::json;
use usage_rs::{Args, RunAsyncWith};
use yt::api::work_items::WorkItemChange;
use yt::output::Format;

use super::{day, duration, work_item_json, work_type};
use crate::cli::Ctx;
use crate::cli::issue::text;

/// Log time spent on an issue
#[derive(Args)]
#[usage(effect = "write")]
pub(crate) struct Log {
    /// Issue ID or URL
    #[usage(arg, name = "ID")]
    id: String,
    /// Time spent, e.g. `1h 30m` or `2d`
    #[usage(arg, name = "DURATION")]
    duration: String,
    /// Day the work was done, YYYY-MM-DD (default: today)
    #[usage(long, value_name = "DATE")]
    date: Option<String>,
    /// Work item type, e.g. `Development`
    #[usage(long = "type", value_name = "TYPE")]
    work_type: Option<String>,
    /// What was done (Markdown)
    #[usage(short, long, value_name = "TEXT")]
    body: Option<String>,
    /// Read the text from a file, or `-` for stdin
    #[usage(long, value_name = "PATH")]
    body_file: Option<String>,
    /// Show what would be logged without logging it
    #[usage(long)]
    dry_run: bool,
}

impl RunAsyncWith<&Ctx> for Log {
    type Output = yt::Result<()>;

    async fn run_async_with(self, ctx: &Ctx) -> Self::Output {
        let duration = duration(&self.duration)?;
        let date = day("--date", &self.date.clone().unwrap_or_else(yt::time::today))?;
        let day_text = yt::time::date(date);
        let text = text::main_text(self.body, self.body_file.as_deref())
            .await?
            .map(|text| text.trim_end().to_string())
            .filter(|text| !text.is_empty());
        let client = ctx.client().await?;
        let id = client.instance().issue_id(&self.id)?;
        let work_type = match &self.work_type {
            Some(name) => Some(work_type(client, &id, name).await?),
            None => None,
        };
        let change = WorkItemChange {
            date: Some(date),
            duration: Some(duration.clone()),
            type_id: work_type.as_ref().map(|work_type| work_type.id.clone()),
            text,
        };
        if self.dry_run {
            if ctx.output.format() != Format::Table {
                return ctx.output.json(&json!({
                    "issue": id,
                    "date": day_text,
                    "duration": duration,
                    "type": work_type.as_ref().map(|work_type| work_type.name.as_str()),
                    "text": change.text,
                }));
            }
            let mut line = format!("{id}: log {duration} on {day_text}");
            if let Some(work_type) = &work_type {
                line.push_str(&format!(" ({})", work_type.name));
            }
            if let Some(text) = &change.text {
                line.push_str(&format!(" \"{}\"", text.lines().next().unwrap_or("")));
            }
            return ctx.output.write(&line);
        }
        let item = yt::api::work_items::create(client, &id, &change).await?;
        match ctx.output.format() {
            Format::Table => {
                ctx.note(&format!("Logged {} on {id}", item.presentation()));
                Ok(())
            }
            Format::Json => ctx.output.json(&work_item_json(&item)),
            Format::Ndjson => ctx.output.ndjson(&work_item_json(&item)),
        }
    }
}
