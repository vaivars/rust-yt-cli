use usage_rs::{Args, RunAsyncWith};
use yt::output::Format;

use crate::cli::Ctx;

/// Add issues to a sprint
#[derive(Args)]
#[usage(effect = "write")]
pub(crate) struct Add {
    /// Board name or ID
    #[usage(arg, name = "BOARD")]
    board: String,
    /// Sprint name or ID
    #[usage(arg, name = "SPRINT")]
    sprint: String,
    /// Issue IDs or URLs; `-` reads IDs from stdin
    #[usage(arg, name = "ID", required)]
    ids: Vec<String>,
}

/// Remove issues from a sprint
#[derive(Args)]
#[usage(effect = "write")]
pub(crate) struct Remove {
    /// Board name or ID
    #[usage(arg, name = "BOARD")]
    board: String,
    /// Sprint name or ID
    #[usage(arg, name = "SPRINT")]
    sprint: String,
    /// Issue IDs or URLs; `-` reads IDs from stdin
    #[usage(arg, name = "ID", required)]
    ids: Vec<String>,
}

impl RunAsyncWith<&Ctx> for Add {
    type Output = yt::Result<()>;

    async fn run_async_with(self, ctx: &Ctx) -> Self::Output {
        change(ctx, &self.board, &self.sprint, &self.ids, true).await
    }
}

impl RunAsyncWith<&Ctx> for Remove {
    type Output = yt::Result<()>;

    async fn run_async_with(self, ctx: &Ctx) -> Self::Output {
        change(ctx, &self.board, &self.sprint, &self.ids, false).await
    }
}

async fn change(ctx: &Ctx, board: &str, sprint: &str, ids: &[String], add: bool) -> yt::Result<()> {
    let ids = crate::cli::issue::ids::issue_ids(ctx, ids).await?;
    let client = ctx.client().await?;
    let board = crate::cli::board::find(client, board).await?;
    let sprints = yt::api::agiles::sprints(client, &board.id).await?;
    let sprint = yt::api::agiles::find_sprint(&sprints, sprint)?;
    let mut done: Vec<String> = Vec::new();
    for id in &ids {
        let result = if add {
            yt::api::agiles::add_issue(client, &board.id, &sprint.id, id).await
        } else {
            yt::api::agiles::remove_issue(client, &board.id, &sprint.id, id).await
        };
        if let Err(error) = result {
            let error = error.wrap_err(format!(
                "could not {} {id}",
                if add { "add" } else { "remove" }
            ));
            return Err(if done.is_empty() {
                error
            } else {
                error.wrap_err(format!(
                    "stopped after a failure; already {}: {}",
                    if add { "added" } else { "removed" },
                    done.join(", ")
                ))
            });
        }
        done.push(id.clone());
    }
    match ctx.output.format() {
        Format::Table => {
            let (verb, preposition) = if add {
                ("Added", "to")
            } else {
                ("Removed", "from")
            };
            ctx.note(&format!(
                "{verb} {} {preposition} {}",
                done.join(", "),
                sprint.name
            ));
            Ok(())
        }
        Format::Json => ctx.output.json(&done),
        Format::Ndjson => done.iter().try_for_each(|id| ctx.output.ndjson(id)),
    }
}
