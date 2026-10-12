use usage_rs::{Args, RunAsyncWith};

use crate::cli::Ctx;

/// Show a sprint's issues by column
#[derive(Args)]
#[usage(effect = "read")]
pub(crate) struct Show {
    /// Board name or ID
    #[usage(arg, name = "BOARD")]
    board: String,
    /// Sprint name or ID
    #[usage(arg, name = "SPRINT")]
    sprint: String,
    /// Also list the issues of resolved columns
    #[usage(long)]
    resolved: bool,
}

impl RunAsyncWith<&Ctx> for Show {
    type Output = yt::Result<()>;

    async fn run_async_with(self, ctx: &Ctx) -> Self::Output {
        let client = ctx.client().await?;
        let board = crate::cli::board::find(client, &self.board).await?;
        let sprint = crate::cli::board::sprint(client, &board, Some(&self.sprint)).await?;
        crate::cli::board::view::show_sprint(ctx, client, &board, sprint.as_ref(), self.resolved)
            .await
    }
}

/// Show the current sprint's issues by column
#[derive(Args)]
#[usage(effect = "read")]
pub(crate) struct Current {
    /// Board name or ID
    #[usage(arg, name = "BOARD")]
    board: String,
    /// Also list the issues of resolved columns
    #[usage(long)]
    resolved: bool,
}

impl RunAsyncWith<&Ctx> for Current {
    type Output = yt::Result<()>;

    async fn run_async_with(self, ctx: &Ctx) -> Self::Output {
        let client = ctx.client().await?;
        let board = crate::cli::board::find(client, &self.board).await?;
        let sprint = crate::cli::board::sprint(client, &board, None).await?;
        crate::cli::board::view::show_sprint(ctx, client, &board, sprint.as_ref(), self.resolved)
            .await
    }
}
