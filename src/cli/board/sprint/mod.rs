mod change;
mod create;
mod list;
mod show;

use usage_rs::{Args, Subcommands};

/// Work with a board's sprints
#[derive(Args)]
#[usage(run_async_with)]
pub(crate) struct SprintGroup {
    #[usage(subcommand)]
    command: SprintCommand,
}

#[derive(Subcommands)]
#[usage(run_async_with)]
pub(crate) enum SprintCommand {
    /// List a board's sprints
    #[usage(visible_alias = "ls")]
    List(list::List),
    /// Show the current sprint's issues by column
    Current(show::Current),
    /// Show a sprint's issues by column
    Show(show::Show),
    /// Create a sprint on a board
    Create(create::Create),
    /// Add issues to a sprint
    Add(change::Add),
    /// Remove issues from a sprint
    Remove(change::Remove),
}
