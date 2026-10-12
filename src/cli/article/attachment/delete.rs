use usage_rs::{Args, RunAsyncWith};
use yt::api::Parent;

use crate::cli::Ctx;

/// Delete an attachment
#[derive(Args)]
#[usage(effect = "destructive")]
pub(crate) struct Delete {
    /// Article ID or URL
    #[usage(arg, name = "ID")]
    id: String,
    /// Attachment name or ID
    #[usage(arg, name = "NAME")]
    name: String,
}

impl RunAsyncWith<&Ctx> for Delete {
    type Output = yt::Result<()>;

    async fn run_async_with(self, ctx: &Ctx) -> Self::Output {
        let parent = Parent::Article(ctx.instance()?.article_id(&self.id)?);
        crate::cli::shared::attachment::delete(ctx, parent, self.name).await
    }
}
