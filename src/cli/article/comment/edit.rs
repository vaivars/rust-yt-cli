use usage_rs::{Args, RunAsyncWith};
use yt::api::Parent;

use crate::cli::Ctx;

/// Change a comment's text
#[derive(Args)]
#[usage(effect = "write")]
pub(crate) struct Edit {
    /// Article ID or URL
    #[usage(arg, name = "ID")]
    id: String,
    /// Comment ID, as `yt article comment list` shows it
    #[usage(arg, name = "COMMENT")]
    comment: String,
    /// New text (Markdown)
    #[usage(short, long, value_name = "TEXT")]
    body: Option<String>,
    /// Read the text from a file, or `-` for stdin
    #[usage(long, value_name = "PATH")]
    body_file: Option<String>,
}

impl RunAsyncWith<&Ctx> for Edit {
    type Output = yt::Result<()>;

    async fn run_async_with(self, ctx: &Ctx) -> Self::Output {
        let parent = Parent::Article(ctx.instance()?.article_id(&self.id)?);
        crate::cli::shared::comment::edit(ctx, parent, self.comment, self.body, self.body_file)
            .await
    }
}
