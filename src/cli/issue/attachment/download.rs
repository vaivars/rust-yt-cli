use usage_rs::{Args, RunAsyncWith};
use yt::api::Parent;

use crate::cli::Ctx;

/// Download an attachment
#[derive(Args)]
#[usage(effect = "write")]
pub(crate) struct Download {
    /// Issue ID or URL
    #[usage(arg, name = "ID")]
    id: String,
    /// Attachment name or ID
    #[usage(arg, name = "NAME")]
    name: String,
    /// Where to save it; `-` writes to stdout (default: ./NAME)
    #[usage(short, long, value_name = "PATH")]
    output: Option<String>,
}

impl RunAsyncWith<&Ctx> for Download {
    type Output = yt::Result<()>;

    async fn run_async_with(self, ctx: &Ctx) -> Self::Output {
        let parent = Parent::Issue(ctx.instance()?.issue_id(&self.id)?);
        crate::cli::shared::attachment::download(ctx, parent, self.name, self.output).await
    }
}
