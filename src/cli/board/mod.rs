mod list;
mod show;
mod sprint;
pub(crate) mod view;

use serde_json::{Value, json};
use usage_rs::{Args, Subcommands};
use yt::api::Client;
use yt::api::agiles::{Board, Sprint};
use yt::instance::InstanceUrl;

/// Work with agile boards and sprints
#[derive(Args)]
#[usage(run_async_with)]
pub(crate) struct BoardGroup {
    #[usage(subcommand)]
    command: BoardCommand,
}

#[derive(Subcommands)]
#[usage(run_async_with)]
pub(crate) enum BoardCommand {
    /// List agile boards
    #[usage(visible_alias = "ls")]
    List(list::List),
    /// Show a board and its current sprint's issues by column
    Show(show::Show),
    /// Work with a board's sprints
    Sprint(sprint::SprintGroup),
}

pub(crate) fn board_json(instance: &InstanceUrl, board: &Board) -> Value {
    json!({
        "id": board.id,
        "name": board.name,
        "projects": board.projects.iter().map(|project| project.short_name.as_str()).collect::<Vec<_>>(),
        "owner": board.owner.as_ref().map(|owner| owner.login.as_str()),
        "sprints": board.sprints_enabled(),
        "current_sprint": board.current_sprint.as_ref().map(|sprint| sprint.name.as_str()),
        "url": instance.board_url(&board.id).to_string(),
    })
}

pub(crate) fn sprint_json(sprint: &Sprint) -> Value {
    json!({
        "id": sprint.id,
        "name": sprint.name,
        "goal": sprint.goal,
        "start": sprint.start.map(yt::time::date),
        "finish": sprint.finish.map(yt::time::date),
        "archived": sprint.archived,
    })
}

/// The board named or numbered `name_or_id`.
pub(crate) async fn find(client: &Client, name_or_id: &str) -> yt::Result<Board> {
    let boards = yt::api::agiles::boards(client).await?;
    Ok(yt::api::agiles::find_board(&boards, name_or_id)?.clone())
}

/// The sprint of `board` named or numbered `name_or_id`, or the current one.
pub(crate) async fn sprint(
    client: &Client,
    board: &Board,
    name_or_id: Option<&str>,
) -> yt::Result<Option<Sprint>> {
    match (name_or_id, &board.current_sprint) {
        (Some(name_or_id), _) => {
            let sprints = yt::api::agiles::sprints(client, &board.id).await?;
            Ok(Some(
                yt::api::agiles::find_sprint(&sprints, name_or_id)?.clone(),
            ))
        }
        (None, Some(current)) => Ok(Some(
            yt::api::agiles::sprint(client, &board.id, &current.id).await?,
        )),
        (None, None) => Ok(None),
    }
}
