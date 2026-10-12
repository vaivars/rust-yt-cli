use usage_rs::{Args, RunAsyncWith};
use yt::output::{Format, Table};

use crate::cli::Ctx;
use crate::cli::board::sprint_json;

/// List a board's sprints
#[derive(Args)]
#[usage(effect = "read")]
pub(crate) struct List {
    /// Board name or ID
    #[usage(arg, name = "BOARD")]
    board: String,
}

impl RunAsyncWith<&Ctx> for List {
    type Output = yt::Result<()>;

    async fn run_async_with(self, ctx: &Ctx) -> Self::Output {
        let client = ctx.client().await?;
        let board = crate::cli::board::find(client, &self.board).await?;
        let sprints = yt::api::agiles::sprints(client, &board.id).await?;
        if sprints.is_empty() {
            ctx.note("No sprints found.");
        }
        let current = board
            .current_sprint
            .as_ref()
            .map(|sprint| sprint.id.as_str());
        if !board.sprints_enabled() {
            ctx.note(&format!(
                "Sprints are turned off on board {}; its issues are in one sprint.",
                board.name
            ));
        }
        let json_of = |sprint: &yt::api::agiles::Sprint| {
            let mut value = sprint_json(sprint);
            value["current"] = (Some(sprint.id.as_str()) == current).into();
            value
        };
        match ctx.output.format() {
            Format::Table => {
                let mut table = Table::new(["NAME", "START", "FINISH", "ARCHIVED"]);
                for sprint in &sprints {
                    let marker = if Some(sprint.id.as_str()) == current {
                        "* "
                    } else {
                        "  "
                    };
                    table.row([
                        format!("{marker}{}", sprint.name),
                        sprint.start.map(yt::time::date).unwrap_or_default(),
                        sprint.finish.map(yt::time::date).unwrap_or_default(),
                        if sprint.archived { "yes" } else { "" }.to_string(),
                    ]);
                }
                ctx.output.table(&table)
            }
            Format::Json => ctx
                .output
                .json(&sprints.iter().map(json_of).collect::<Vec<_>>()),
            Format::Ndjson => sprints
                .iter()
                .try_for_each(|sprint| ctx.output.ndjson(&json_of(sprint))),
        }
    }
}
