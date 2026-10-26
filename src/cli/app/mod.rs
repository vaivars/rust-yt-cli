mod add;
pub(crate) mod complete;
mod list;
mod run;
mod tree;

use std::ffi::{OsStr, OsString};

use usage_rs::spec::CommandArgs;
use usage_rs::{Args, RunAsyncWith, Subcommands};
use yt::YtError;

use crate::cli::{Ctx, Globals, Yt};

/// Run commands of YouTrack apps that describe their API
#[derive(Args)]
#[usage(
    run_async_with,
    after_long_help = "An app enabled with `yt app add` runs as `yt app <APP> <COMMAND>`; `yt app <APP> --help` lists its commands."
)]
pub(crate) struct AppGroup {
    #[usage(subcommand)]
    command: AppCommand,
}

// Dispatched by hand: the derive's `external = …` function cannot receive `&Ctx`.
#[derive(Subcommands)]
pub(crate) enum AppCommand {
    /// List the apps yt can use and which are enabled
    #[usage(visible_alias = "ls")]
    List(list::List),
    /// Check that an app is installed and use it with the profile
    Add(add::Add),
    /// Stop using an app with the profile
    Remove(add::Remove),
    #[usage(external_subcommand)]
    Run(Vec<OsString>),
}

impl RunAsyncWith<&Ctx> for AppCommand {
    type Output = yt::Result<()>;

    async fn run_async_with(self, ctx: &Ctx) -> Self::Output {
        match self {
            Self::List(command) => command.run_async_with(ctx).await,
            Self::Add(command) => command.run_async_with(ctx).await,
            Self::Remove(command) => command.run_async_with(ctx).await,
            Self::Run(words) => run::run(ctx, words).await,
        }
    }
}

/// usage-rs leaves every word after `yt app <APP>` unparsed, global flags included. This parses
/// the command line again with those moved in front of the app, so they reach `Ctx` and logging.
pub(crate) fn lift_globals(cli: Yt, argv: &[OsString]) -> yt::Result<Yt> {
    let crate::cli::Command::App(AppGroup {
        command: AppCommand::Run(words),
    }) = &cli.command
    else {
        return Ok(cli);
    };
    let Some((app, rest)) = words.split_first() else {
        return Ok(cli);
    };
    let (lifted, kept) = split_globals(rest);
    let Some(start) = argv.len().checked_sub(words.len()) else {
        return Ok(cli);
    };
    if lifted.is_empty() || argv[start..] != words[..] {
        return Ok(cli);
    }
    let mut reordered: Vec<&OsStr> = argv[..start].iter().map(OsString::as_os_str).collect();
    reordered.extend(lifted);
    reordered.push(app);
    reordered.extend(kept);
    Yt::parse_from(&reordered).map_err(|error| {
        YtError::CommandLine(usage_rs::render_failure(Yt::spec(), &reordered, &error)).into()
    })
}

/// The global flags among `words` (with their values) and the other words, in order.
fn split_globals(words: &[OsString]) -> (Vec<&OsStr>, Vec<&OsStr>) {
    let flags = <Globals as CommandArgs>::COMMAND.flags;
    let mut lifted = Vec::new();
    let mut kept = Vec::new();
    let mut words = words.iter().map(OsString::as_os_str);
    while let Some(word) = words.next() {
        let text = word.to_str().unwrap_or_default();
        if text == "--" {
            kept.push(word);
            kept.extend(words.by_ref());
            break;
        }
        if let Some(long) = text.strip_prefix("--") {
            let (name, attached) = match long.split_once('=') {
                Some((name, _)) => (name, true),
                None => (long, false),
            };
            match flags.iter().find(|flag| flag.longs.contains(&name)) {
                Some(flag) => {
                    lifted.push(word);
                    if flag.takes_value
                        && !attached
                        && let Some(value) = words.next()
                    {
                        lifted.push(value);
                    }
                }
                None => kept.push(word),
            }
        } else if let Some(shorts) = text.strip_prefix('-')
            && !shorts.is_empty()
            && shorts.bytes().all(|short| {
                flags
                    .iter()
                    .any(|flag| !flag.takes_value && flag.shorts.contains(&short))
            })
        {
            lifted.push(word);
        } else {
            kept.push(word);
        }
    }
    (lifted, kept)
}

#[cfg(test)]
mod tests {
    use yt::output::Format;

    use super::*;

    fn argv(words: &[&str]) -> Vec<OsString> {
        words.iter().map(OsString::from).collect()
    }

    fn lifted(words: &[&str]) -> Yt {
        let argv = argv(words);
        let refs: Vec<&OsStr> = argv.iter().map(OsString::as_os_str).collect();
        let cli = Yt::parse_from(&refs).unwrap();
        lift_globals(cli, &argv).unwrap()
    }

    fn run_words(cli: &Yt) -> Vec<String> {
        match &cli.command {
            crate::cli::Command::App(AppGroup {
                command: AppCommand::Run(words),
            }) => words
                .iter()
                .map(|word| word.to_string_lossy().into_owned())
                .collect(),
            _ => panic!("not an app command"),
        }
    }

    #[test]
    fn global_flags_after_the_app_reach_the_globals() {
        let cli = lifted(&[
            "--profile",
            "acme",
            "app",
            "hidden-fields",
            "get-issue",
            "WEB-1",
            "--format",
            "json",
            "-vq",
            "--fields",
            "A",
            "--no-color",
        ]);
        assert_eq!(cli.globals.profile.as_deref(), Some("acme"));
        assert_eq!(cli.globals.format, Some(Format::Json));
        assert_eq!(cli.globals.verbose, 1);
        assert!(cli.globals.quiet && cli.globals.no_color);
        assert_eq!(
            run_words(&cli),
            ["hidden-fields", "get-issue", "WEB-1", "--fields", "A"]
        );
    }

    #[test]
    fn words_after_a_double_dash_are_left_alone() {
        let cli = lifted(&["app", "hidden-fields", "x", "--", "--format", "json"]);
        assert_eq!(cli.globals.format, None);
        assert_eq!(
            run_words(&cli),
            ["hidden-fields", "x", "--", "--format", "json"]
        );
    }

    #[test]
    fn a_bad_global_value_is_a_usage_error() {
        let argv = argv(&["app", "hidden-fields", "x", "--format", "xml"]);
        let refs: Vec<&OsStr> = argv.iter().map(OsString::as_os_str).collect();
        let Err(error) = lift_globals(Yt::parse_from(&refs).unwrap(), &argv) else {
            panic!("--format xml was accepted");
        };
        assert_eq!(yt::error::exit_code(&error), 2);
    }

    #[test]
    fn built_in_app_commands_are_not_external() {
        for name in ["list", "ls", "add", "remove"] {
            let argv = argv(&["app", name, "hidden-fields"]);
            let refs: Vec<&OsStr> = argv.iter().map(OsString::as_os_str).collect();
            let parsed = Yt::parse_from(&refs);
            assert!(
                !matches!(
                    parsed,
                    Ok(Yt {
                        command: crate::cli::Command::App(AppGroup {
                            command: AppCommand::Run(_)
                        }),
                        ..
                    })
                ),
                "{name}"
            );
        }
        for name in yt::apps::registry::RESERVED {
            assert!(yt::apps::registry::find(name).is_none());
        }
    }
}
