use usage_rs::{Args, RunAsyncWith};
use yt::output::Format;

use crate::cli::Ctx;

/// Delete work items
#[derive(Args)]
#[usage(effect = "destructive")]
pub(crate) struct Delete {
    /// Issue ID or URL
    #[usage(arg, name = "ID")]
    id: String,
    /// Work item IDs, as `yt time list` shows them
    #[usage(arg, name = "ITEM", required)]
    items: Vec<String>,
}

impl RunAsyncWith<&Ctx> for Delete {
    type Output = yt::Result<()>;

    async fn run_async_with(self, ctx: &Ctx) -> Self::Output {
        let id = ctx.instance()?.issue_id(&self.id)?;
        for item in &self.items {
            yt::api::work_items::check_item(item)?;
        }
        ctx.ui
            .confirm_destructive(&format!(
                "delete work items {} on {id}",
                self.items.join(", ")
            ))
            .await?;
        let client = ctx.client().await?;
        let mut deleted: Vec<String> = Vec::new();
        for item in &self.items {
            if let Err(error) = yt::api::work_items::delete(client, &id, item).await {
                return Err(if deleted.is_empty() {
                    error
                } else {
                    error.wrap_err(format!(
                        "stopped after a failure; already deleted: {}",
                        deleted.join(", ")
                    ))
                });
            }
            deleted.push(item.clone());
        }
        match ctx.output.format() {
            Format::Table => {
                ctx.note(&format!(
                    "Deleted work items {} on {id}",
                    deleted.join(", ")
                ));
                Ok(())
            }
            Format::Json => ctx.output.json(&deleted),
            Format::Ndjson => deleted.iter().try_for_each(|item| ctx.output.ndjson(item)),
        }
    }
}
