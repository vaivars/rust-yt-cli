use usage_rs::{Args, RunAsyncWith};
use yt::YtError;
use yt::fields::{change, plan};

use super::write::{print_dry_run, report_updated};
use super::{ids, text};
use crate::cli::Ctx;

/// Change issues
#[derive(Args)]
#[usage(effect = "write")]
pub(crate) struct Edit {
    /// Issue IDs or URLs; `-` reads IDs from stdin
    #[usage(arg, name = "ID", required)]
    ids: Vec<String>,
    /// New summary (one issue)
    #[usage(short, long, value_name = "TEXT")]
    summary: Option<String>,
    /// New description (one issue)
    #[usage(short, long, value_name = "TEXT")]
    body: Option<String>,
    /// Read the description from a file, or `-` for stdin
    #[usage(long, value_name = "PATH")]
    body_file: Option<String>,
    /// Custom field: NAME=VALUE, NAME+=VALUE, NAME-=VALUE, or NAME= to clear (repeatable)
    #[usage(short = 'f', long = "field", value_name = "NAME=VALUE", var)]
    fields: Vec<String>,
    /// Show the changes without applying them
    #[usage(long)]
    dry_run: bool,
    /// Skip client-side field validation
    #[usage(long)]
    no_validate: bool,
}

fn usage(message: &str) -> miette::Report {
    YtError::Usage {
        message: message.into(),
    }
    .into()
}

impl RunAsyncWith<&Ctx> for Edit {
    type Output = yt::Result<()>;

    async fn run_async_with(self, ctx: &Ctx) -> Self::Output {
        if self.ids.iter().any(|id| id == "-") && self.body_file.as_deref() == Some("-") {
            return Err(usage(
                "only one argument can read stdin: IDs (-) or --body-file -",
            ));
        }
        let summary = self.summary.as_deref().map(str::trim);
        if summary == Some("") {
            return Err(usage("the summary cannot be empty"));
        }
        let ids = ids::issue_ids(ctx, &self.ids).await?;
        let description = text::main_text(self.body, self.body_file.as_deref()).await?;
        if summary.is_none() && description.is_none() && self.fields.is_empty() {
            if self.dry_run {
                return Err(usage(
                    "--dry-run shows a change; pass -s, -b, --body-file or -f",
                ));
            }
            if ids.len() == 1 && ctx.ui.is_interactive() {
                return edit_in_editor(ctx, &ids[0]).await;
            }
            return Err(usage("nothing to change: pass -s, -b, --body-file or -f"));
        }
        if ids.len() > 1 && (summary.is_some() || description.is_some()) {
            return Err(usage(
                "-s, -b and --body-file change one issue; give a single ID",
            ));
        }
        let changes = change::parse(&self.fields)?;
        let client = ctx.client().await?;
        let cache = ctx.schema_cache()?;
        let mut operations = Vec::new();
        for (project, group) in yt::api::issues::by_project(&ids) {
            let resolved = if changes.is_empty() {
                Vec::new()
            } else {
                yt::fields::resolve_for(client, &cache, &project, &changes, !self.no_validate)
                    .await?
                    .1
            };
            operations.extend(plan::edit_operations(
                &group,
                &resolved,
                summary,
                description.as_deref(),
            )?);
        }
        if self.dry_run {
            return print_dry_run(ctx, client, &operations).await;
        }
        yt::fields::apply::execute(client, &operations).await?;
        report_updated(ctx, client, &ids).await
    }
}

/// One issue in `$EDITOR`: the summary on the first line, then the description.
async fn edit_in_editor(ctx: &Ctx, id: &str) -> yt::Result<()> {
    let client = ctx.client().await?;
    let issue = yt::api::issues::get(client, id).await?;
    let summary = issue.summary.clone().unwrap_or_default();
    let description = issue.description.clone().unwrap_or_default();
    let description = description.trim_end();
    let buffer = yt::editor::edit(
        &ctx.env,
        &ctx.dirs.state,
        &yt::editor::join_issue(&summary, description),
    )
    .await?;
    let (new_summary, new_description) = yt::editor::split_issue(&buffer);
    if new_summary.is_empty() {
        return Err(usage("the summary (the first line) cannot be empty"));
    }
    if new_summary == summary && new_description == description {
        ctx.note("No changes");
        return Ok(());
    }
    let operations = plan::edit_operations(
        &[id.to_string()],
        &[],
        (new_summary != summary).then_some(new_summary.as_str()),
        (new_description != description).then_some(new_description.as_str()),
    )?;
    yt::fields::apply::execute(client, &operations).await?;
    report_updated(ctx, client, &[id.to_string()]).await
}
