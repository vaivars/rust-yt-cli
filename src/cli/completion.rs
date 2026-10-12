use std::ffi::OsString;
use std::process::ExitCode;

use usage_rs::{Args, RunAsyncWith, ValueEnum};

use super::{Ctx, Yt};

#[derive(Clone, Copy, Debug, ValueEnum)]
pub(crate) enum Shell {
    Bash,
    Zsh,
    Fish,
}

/// Print a shell completion script
#[derive(Args)]
#[usage(effect = "read")]
pub(crate) struct Completion {
    /// Shell to generate the script for
    #[usage(value_enum)]
    shell: Shell,
}

impl RunAsyncWith<&Ctx> for Completion {
    type Output = yt::Result<()>;

    async fn run_async_with(self, ctx: &Ctx) -> Self::Output {
        let shell = match self.shell {
            Shell::Bash => usage_rs::complete::Shell::Bash,
            Shell::Zsh => usage_rs::complete::Shell::Zsh,
            Shell::Fish => usage_rs::complete::Shell::Fish,
        };
        ctx.output.write(app().completion_script(shell).trim_end())
    }
}

static COMPLETIONS: [usage_rs::complete::CompletionOverlay<'static>; 1] =
    [usage_rs::complete::CompletionOverlay::async_any(
        "PROFILE",
        complete_profiles,
    )];

pub(crate) fn app() -> usage_rs::complete::App<'static> {
    Yt::app().completion_app().completions(&COMPLETIONS)
}

/// Reads only the local config: a Tab press never touches the network or the keyring.
fn complete_profiles(
    _ctx: usage_rs::spec::CompleteCtx<'_>,
) -> usage_rs::complete::CompletionFuture<'_> {
    Box::pin(async move {
        profile_names()
            .into_iter()
            .map(usage_rs::spec::Candidate::new)
            .collect()
    })
}

fn profile_names() -> Vec<String> {
    let env = yt::env::Env::from_process();
    let Ok(dirs) = yt::dirs::Dirs::from_env(&env) else {
        return Vec::new();
    };
    yt::config::file::ConfigStore::new(dirs.config_file())
        .load()
        .map(|config| config.profiles.into_keys().collect())
        .unwrap_or_default()
}

/// Answers the hidden request a completion script sends when the user presses Tab.
pub(crate) fn answer(argv: &[OsString]) -> ExitCode {
    let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    else {
        return ExitCode::FAILURE;
    };
    if let Some(answer) = runtime.block_on(app().completion_request(argv)) {
        print!("{answer}");
    }
    ExitCode::SUCCESS
}
