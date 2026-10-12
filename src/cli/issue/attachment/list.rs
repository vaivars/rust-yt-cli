use usage_rs::{Args, RunAsyncWith};
use yt::api::Parent;

use crate::cli::Ctx;

/// List an issue's attachments
#[derive(Args)]
#[usage(effect = "read")]
pub(crate) struct List {
    /// Issue ID or URL
    #[usage(arg, name = "ID")]
    id: String,
}

impl RunAsyncWith<&Ctx> for List {
    type Output = yt::Result<()>;

    async fn run_async_with(self, ctx: &Ctx) -> Self::Output {
        let parent = Parent::Issue(ctx.instance()?.issue_id(&self.id)?);
        crate::cli::shared::attachment::list(ctx, parent).await
    }
}
