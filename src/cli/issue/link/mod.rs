mod change;
mod list;
mod types;

use usage_rs::{Args, Subcommands};

/// Work with issue links
#[derive(Args)]
#[usage(run_async_with)]
pub(crate) struct LinkGroup {
    #[usage(subcommand)]
    command: LinkCommand,
}

#[derive(Subcommands)]
#[usage(run_async_with)]
pub(crate) enum LinkCommand {
    /// List an issue's links
    #[usage(visible_alias = "ls")]
    List(list::List),
    /// Link an issue to another
    Add(change::AddLink),
    /// Remove a link between two issues
    Remove(change::RemoveLink),
    /// List the instance's link types
    Types(types::Types),
}
