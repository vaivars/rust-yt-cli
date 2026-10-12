use usage_rs::{Args, RunAsyncWith};

use crate::cli::Ctx;

/// Show a board and its current sprint's issues by column
#[derive(Args)]
#[usage(effect = "read")]
pub(crate) struct Show {
    /// Board name or ID
    #[usage(arg, name = "BOARD")]
    board: String,
    /// Also list the issues of resolved columns
    #[usage(long)]
    resolved: bool,
    /// Open the board in the browser instead of printing it
    #[usage(short, long)]
    web: bool,
}

impl RunAsyncWith<&Ctx> for Show {
    type Output = yt::Result<()>;

    async fn run_async_with(self, ctx: &Ctx) -> Self::Output {
        let client = ctx.client().await?;
        let board = super::find(client, &self.board).await?;
        if self.web {
            let url = client.instance().board_url(&board.id);
            if !yt::browser::open(&ctx.env, url.as_str()) {
                ctx.output.write(url.as_str())?;
            }
            return Ok(());
        }
        let sprint = super::sprint(client, &board, None).await?;
        super::view::show_sprint(ctx, client, &board, sprint.as_ref(), self.resolved).await
    }
}
