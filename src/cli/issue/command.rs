use usage_rs::{Args, RunAsyncWith};
use yt::fields::plan::Operation;

use super::ids;
use super::write::{print_dry_run, report_updated};
use crate::cli::Ctx;

/// Apply a YouTrack command to issues
#[derive(Args)]
#[usage(effect = "write")]
pub(crate) struct Apply {
    /// Command text, e.g. `State Fixed` or `for me`
    #[usage(arg, name = "COMMAND")]
    command: String,
    /// Issue IDs or URLs; `-` reads IDs from stdin
    #[usage(arg, name = "ID", required)]
    ids: Vec<String>,
    /// Show how YouTrack reads the command without applying it
    #[usage(long)]
    dry_run: bool,
}

impl RunAsyncWith<&Ctx> for Apply {
    type Output = yt::Result<()>;

    async fn run_async_with(self, ctx: &Ctx) -> Self::Output {
        let ids = ids::issue_ids(ctx, &self.ids).await?;
        let client = ctx.client().await?;
        let operations = vec![Operation::Command {
            issues: ids.clone(),
            command: self.command,
        }];
        if self.dry_run {
            return print_dry_run(ctx, client, &operations).await;
        }
        yt::fields::apply::execute(client, &operations).await?;
        report_updated(ctx, client, &ids).await
    }
}
