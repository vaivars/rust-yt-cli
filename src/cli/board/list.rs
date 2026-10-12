use usage_rs::{Args, RunAsyncWith};
use yt::output::{Format, Table};

use super::board_json;
use crate::cli::Ctx;

/// List agile boards
#[derive(Args)]
#[usage(effect = "read")]
pub(crate) struct List {}

impl RunAsyncWith<&Ctx> for List {
    type Output = yt::Result<()>;

    async fn run_async_with(self, ctx: &Ctx) -> Self::Output {
        let client = ctx.client().await?;
        let instance = client.instance();
        let boards = yt::api::agiles::boards(client).await?;
        if boards.is_empty() {
            ctx.note("No boards found.");
        }
        match ctx.output.format() {
            Format::Table => {
                let mut table =
                    Table::new(["NAME", "PROJECTS", "SPRINTS", "CURRENT SPRINT", "OWNER"]);
                for board in &boards {
                    let projects: Vec<&str> = board
                        .projects
                        .iter()
                        .map(|project| project.short_name.as_str())
                        .collect();
                    table.row([
                        board.name.clone(),
                        projects.join(", "),
                        if board.sprints_enabled() { "on" } else { "off" }.to_string(),
                        board
                            .current_sprint
                            .as_ref()
                            .map(|sprint| sprint.name.clone())
                            .unwrap_or_default(),
                        board
                            .owner
                            .as_ref()
                            .map(|owner| owner.login.clone())
                            .unwrap_or_default(),
                    ]);
                }
                ctx.output.table(&table)
            }
            Format::Json => ctx.output.json(
                &boards
                    .iter()
                    .map(|board| board_json(instance, board))
                    .collect::<Vec<_>>(),
            ),
            Format::Ndjson => boards
                .iter()
                .try_for_each(|board| ctx.output.ndjson(&board_json(instance, board))),
        }
    }
}
