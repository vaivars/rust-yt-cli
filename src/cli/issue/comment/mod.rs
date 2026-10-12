mod add;
mod delete;
mod edit;
mod list;

use usage_rs::{Args, Subcommands};

/// Work with issue comments
#[derive(Args)]
#[usage(run_async_with)]
pub(crate) struct CommentGroup {
    #[usage(subcommand)]
    command: CommentCommand,
}

#[derive(Subcommands)]
#[usage(run_async_with)]
pub(crate) enum CommentCommand {
    /// List an issue's comments
    #[usage(visible_alias = "ls")]
    List(list::List),
    /// Add a comment
    Add(add::Add),
    /// Change a comment's text
    Edit(edit::Edit),
    /// Delete a comment
    Delete(delete::Delete),
}
