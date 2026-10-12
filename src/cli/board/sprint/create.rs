use usage_rs::{Args, RunAsyncWith};
use yt::YtError;
use yt::output::Format;

use crate::cli::Ctx;
use crate::cli::board::sprint_json;

/// Create a sprint on a board
#[derive(Args)]
#[usage(effect = "write")]
pub(crate) struct Create {
    /// Board name or ID
    #[usage(arg, name = "BOARD")]
    board: String,
    /// Sprint name
    #[usage(arg, name = "NAME")]
    name: String,
    /// First day, YYYY-MM-DD
    #[usage(long, value_name = "DATE")]
    start: Option<String>,
    /// Last day, YYYY-MM-DD
    #[usage(long, value_name = "DATE")]
    finish: Option<String>,
    /// What the sprint is for
    #[usage(long, value_name = "TEXT")]
    goal: Option<String>,
}

impl RunAsyncWith<&Ctx> for Create {
    type Output = yt::Result<()>;

    async fn run_async_with(self, ctx: &Ctx) -> Self::Output {
        let start = self
            .start
            .as_deref()
            .map(|value| day("--start", value))
            .transpose()?;
        let finish = self
            .finish
            .as_deref()
            .map(|value| day("--finish", value))
            .transpose()?;
        let client = ctx.client().await?;
        let board = crate::cli::board::find(client, &self.board).await?;
        if !board.sprints_enabled() {
            return Err(YtError::Usage {
                message: format!("sprints are turned off on board {}", board.name),
            }
            .into());
        }
        let body = yt::api::agiles::sprint_body(&self.name, self.goal.as_deref(), start, finish);
        let sprint = yt::api::agiles::create_sprint(client, &board.id, &body).await?;
        match ctx.output.format() {
            Format::Table => {
                ctx.note(&format!("Created sprint {} on {}", sprint.name, board.name));
                Ok(())
            }
            Format::Json => ctx.output.json(&sprint_json(&sprint)),
            Format::Ndjson => ctx.output.ndjson(&sprint_json(&sprint)),
        }
    }
}

fn day(flag: &str, value: &str) -> yt::Result<i64> {
    yt::time::day_millis(value).ok_or_else(|| {
        YtError::Usage {
            message: format!("{flag}: '{value}' is not a date (YYYY-MM-DD)"),
        }
        .into()
    })
}
