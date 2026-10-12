use usage_rs::{Args, RunAsyncWith};
use yt::output::Format;

use super::ids;
use crate::cli::Ctx;

/// Delete issues
#[derive(Args)]
#[usage(effect = "destructive")]
pub(crate) struct Delete {
    /// Issue IDs or URLs; `-` reads IDs from stdin
    #[usage(arg, name = "ID", required)]
    ids: Vec<String>,
}

impl RunAsyncWith<&Ctx> for Delete {
    type Output = yt::Result<()>;

    async fn run_async_with(self, ctx: &Ctx) -> Self::Output {
        let ids = ids::issue_ids(ctx, &self.ids).await?;
        ctx.ui
            .confirm_destructive(&format!("delete {}", ids.join(", ")))
            .await?;
        let client = ctx.client().await?;
        let mut deleted: Vec<String> = Vec::new();
        for id in &ids {
            if let Err(error) = yt::api::issues::delete(client, id).await {
                return Err(if deleted.is_empty() {
                    error
                } else {
                    error.wrap_err(format!(
                        "stopped after a failure; already deleted: {}",
                        deleted.join(", ")
                    ))
                });
            }
            deleted.push(id.clone());
        }
        match ctx.output.format() {
            Format::Table => {
                ctx.note(&format!("Deleted {}", deleted.join(", ")));
                Ok(())
            }
            Format::Json => ctx.output.json(&deleted),
            Format::Ndjson => deleted.iter().try_for_each(|id| ctx.output.ndjson(id)),
        }
    }
}
