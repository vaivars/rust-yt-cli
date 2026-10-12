mod get;
mod list;
mod set;

use usage_rs::{Args, Subcommands};

/// Show and change settings
#[derive(Args)]
#[usage(run_async_with)]
pub(crate) struct Config {
    #[usage(subcommand)]
    command: ConfigCommand,
}

#[derive(Subcommands)]
#[usage(run_async_with)]
pub(crate) enum ConfigCommand {
    /// List every setting with its value
    #[usage(visible_alias = "ls")]
    List(list::List),
    /// Show one setting's value and where it came from
    Get(get::Get),
    /// Change a setting in the global config file
    Set(set::Set),
}
