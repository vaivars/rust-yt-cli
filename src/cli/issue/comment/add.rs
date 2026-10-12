use usage_rs::{Args, RunAsyncWith};
use yt::api::Parent;

use crate::cli::Ctx;

/// Add a comment
#[derive(Args)]
#[usage(effect = "write")]
pub(crate) struct Add {
    /// Issue ID or URL
    #[usage(arg, name = "ID")]
    id: String,
    /// Comment text (Markdown)
    #[usage(short, long, value_name = "TEXT")]
    body: Option<String>,
    /// Read the text from a file, or `-` for stdin
    #[usage(long, value_name = "PATH")]
    body_file: Option<String>,
}

impl RunAsyncWith<&Ctx> for Add {
    type Output = yt::Result<()>;

    async fn run_async_with(self, ctx: &Ctx) -> Self::Output {
        let parent = Parent::Issue(ctx.instance()?.issue_id(&self.id)?);
        crate::cli::shared::comment::add(ctx, parent, self.body, self.body_file).await
    }
}
