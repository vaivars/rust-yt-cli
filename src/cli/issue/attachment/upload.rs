use std::path::PathBuf;

use usage_rs::{Args, RunAsyncWith};
use yt::api::Parent;

use crate::cli::Ctx;

/// Upload files to an issue
#[derive(Args)]
#[usage(effect = "write")]
pub(crate) struct Upload {
    /// Issue ID or URL
    #[usage(arg, name = "ID")]
    id: String,
    /// Files to upload
    #[usage(arg, name = "FILE", required)]
    files: Vec<PathBuf>,
}

impl RunAsyncWith<&Ctx> for Upload {
    type Output = yt::Result<()>;

    async fn run_async_with(self, ctx: &Ctx) -> Self::Output {
        let parent = Parent::Issue(ctx.instance()?.issue_id(&self.id)?);
        crate::cli::shared::attachment::upload(ctx, parent, self.files).await
    }
}
