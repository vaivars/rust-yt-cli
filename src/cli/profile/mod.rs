mod list;
mod remove;
mod use_profile;

use usage_rs::{Args, Subcommands};

/// Manage profiles (a YouTrack instance and an account)
#[derive(Args)]
#[usage(run_async_with)]
pub(crate) struct Profile {
    #[usage(subcommand)]
    command: ProfileCommand,
}

#[derive(Subcommands)]
#[usage(run_async_with)]
pub(crate) enum ProfileCommand {
    /// List profiles
    #[usage(visible_alias = "ls")]
    List(list::List),
    /// Make a profile the default
    Use(use_profile::UseProfile),
    /// Remove a profile and its stored credential
    Remove(remove::Remove),
}
