use usage_rs::{Args, RunAsyncWith};
use yt::api::Parent;

use crate::cli::Ctx;

/// Delete a comment
#[derive(Args)]
#[usage(effect = "destructive")]
pub(crate) struct Delete {
    /// Issue ID or URL
    #[usage(arg, name = "ID")]
    id: String,
    /// Comment ID, as `yt issue comment list` shows it
    #[usage(arg, name = "COMMENT")]
    comment: String,
}

impl RunAsyncWith<&Ctx> for Delete {
    type Output = yt::Result<()>;

    async fn run_async_with(self, ctx: &Ctx) -> Self::Output {
        let parent = Parent::Issue(ctx.instance()?.issue_id(&self.id)?);
        crate::cli::shared::comment::delete(ctx, parent, self.comment).await
    }
}
