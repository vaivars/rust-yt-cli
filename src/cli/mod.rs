mod api;
mod article;
mod auth;
mod board;
pub(crate) mod completion;
mod config;
mod ctx;
mod globals;
mod issue;
mod profile;
mod project;
mod shared;
mod time;

use std::process::ExitCode;

use usage_rs::{Cli, Subcommands};
use yt::env::Env;

pub(crate) use ctx::Ctx;
pub(crate) use globals::Globals;

/// Command line interface for JetBrains YouTrack
#[derive(Cli)]
#[usage(
    bin = "yt",
    version,
    completion,
    unknown_flags = "error",
    config = yt::config::settings::Settings,
    run_async_with
)]
pub(crate) struct Yt {
    #[usage(flatten)]
    pub(crate) globals: Globals,
    #[usage(subcommand)]
    pub(crate) command: Command,
}

#[derive(Subcommands)]
#[usage(run_async_with)]
pub(crate) enum Command {
    /// Sign in and manage credentials
    Auth(auth::Auth),
    /// Work with issues
    Issue(issue::Issue),
    /// Work with projects
    Project(project::ProjectGroup),
    /// Log and list spent time
    Time(time::TimeGroup),
    /// Work with knowledge base articles
    Article(article::ArticleGroup),
    /// Work with agile boards and sprints
    Board(board::BoardGroup),
    /// Manage profiles (a YouTrack instance and an account)
    Profile(profile::Profile),
    /// Show and change settings
    Config(config::Config),
    /// Send a request to the YouTrack REST API and print its JSON response
    Api(api::Api),
    /// Print a shell completion script
    Completion(completion::Completion),
}

pub(crate) fn run(cli: Yt) -> ExitCode {
    let env = Env::from_process();
    yt::logger::init(yt::logger::level(
        cli.globals.verbose,
        cli.globals.quiet,
        &env,
    ));
    let ctx = match Ctx::new(&cli.globals, env) {
        Ok(ctx) => ctx,
        Err(error) => return report(error),
    };
    configure_terminal(&cli.globals, &ctx);
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            return report(miette::miette!(
                "could not start the async runtime: {error}"
            ));
        }
    };
    let globals = cli.globals.clone();
    let mut result = runtime.block_on(cli.run_command_async_with(&ctx));
    match expired_sign_in(&result, ctx.has_written()) {
        Some(Expired::Rerun(profile)) if ctx.ui.is_interactive() => {
            result = match runtime.block_on(sign_in_again(&ctx, &profile)) {
                Ok(true) => match Ctx::new(&globals, Env::from_process()) {
                    Ok(fresh) => runtime.block_on(Yt::parse().run_command_async_with(&fresh)),
                    Err(error) => Err(error),
                },
                Ok(false) => result,
                Err(error) => Err(error),
            };
        }
        Some(Expired::AfterChanges(profile)) => ctx.note(&format!(
            "Changes were already sent before the sign-in to {profile} expired, so yt does not run the command again. Sign in with `yt auth login --profile {profile}` and check them before retrying."
        )),
        _ => {}
    }
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => report(error),
    }
}

#[derive(Debug, PartialEq)]
enum Expired {
    /// Nothing was changed yet: sign in again and run the command again.
    Rerun(String),
    /// Re-running could repeat changes the server already applied.
    AfterChanges(String),
}

fn expired_sign_in(result: &yt::Result<()>, written: bool) -> Option<Expired> {
    match result.as_ref().err()?.downcast_ref::<yt::YtError>() {
        Some(yt::YtError::SessionExpired { profile }) if written => {
            Some(Expired::AfterChanges(profile.clone()))
        }
        Some(yt::YtError::SessionExpired { profile }) => Some(Expired::Rerun(profile.clone())),
        _ => None,
    }
}

async fn sign_in_again(ctx: &Ctx, profile: &str) -> yt::Result<bool> {
    let question = format!("Your sign-in to {profile} has expired. Sign in again?");
    if !ctx.ui.confirm(&question, true, "--yes").await? {
        return Ok(false);
    }
    let existing = ctx.global_config()?.profiles.get(profile).cloned();
    let request = yt::auth::login::LoginRequest {
        url: existing.as_ref().map(|existing| existing.url.clone()),
        method: Some(yt::auth::login::Method::Browser),
        client_id: existing
            .as_ref()
            .and_then(|existing| existing.oauth_client_id.clone()),
        profile: Some(profile.to_string()),
        revocable: existing.as_ref().is_some_and(|existing| existing.revocable),
        ..yt::auth::login::LoginRequest::default()
    };
    let outcome = auth::sign_in(ctx, request).await?;
    ctx.note(&format!(
        "Signed in to {} ({}) as {}",
        outcome.profile, outcome.url, outcome.login
    ));
    Ok(true)
}

fn configure_terminal(globals: &Globals, ctx: &Ctx) {
    let setting = ctx
        .settings()
        .map(|resolved| resolved.settings.color.clone())
        .unwrap_or_default();
    let enabled = if globals.no_color {
        Some(false)
    } else {
        match setting.as_str() {
            "always" => Some(true),
            "never" => Some(false),
            _ if ctx.env.get("NO_COLOR").is_some() => Some(false),
            _ => None,
        }
    };
    if let Some(enabled) = enabled {
        console::set_colors_enabled(enabled);
        console::set_colors_enabled_stderr(enabled);
    }
    let stderr_color = console::colors_enabled_stderr();
    let _ = miette::set_hook(Box::new(move |_| {
        Box::new(miette::MietteHandlerOpts::new().color(stderr_color).build())
    }));
}

fn report(error: miette::Report) -> ExitCode {
    yt::ui::eprint_raw(&render(&error));
    ExitCode::from(yt::error::exit_code(&error))
}

/// A heading, aligned `Label:  value` rows and an optional body after a blank line, as `show` commands print.
pub(crate) fn details(heading: &str, rows: Vec<(String, String)>, body: Option<&str>) -> String {
    let width = rows
        .iter()
        .map(|(label, _)| label.chars().count())
        .max()
        .unwrap_or(0)
        + 1;
    let mut lines = vec![heading.to_string()];
    lines.extend(
        rows.into_iter()
            .map(|(label, value)| format!("{:<width$}  {value}", format!("{label}:"))),
    );
    if let Some(body) = body.map(str::trim_end).filter(|body| !body.is_empty()) {
        lines.push(String::new());
        lines.push(body.to_string());
    }
    lines.join("\n")
}

/// `-n 0` would ask for nothing.
pub(crate) fn check_limit(limit: usize) -> yt::Result<()> {
    if limit == 0 {
        return Err(yt::YtError::Usage {
            message: "--limit must be at least 1".into(),
        }
        .into());
    }
    Ok(())
}

fn render(error: &miette::Report) -> String {
    yt::redact::redact(&format!("{:?}", yt::error::printable(error)))
}

#[cfg(test)]
mod tests {
    use usage_rs::test::{self as harness, Page};
    use yt::output::Format;

    use super::*;

    #[test]
    fn details_align_labels_and_add_the_body() {
        let text = super::details(
            "TEST  Test project",
            vec![
                ("Leader".into(), "jane".into()),
                ("Time tracking".into(), "on".into()),
            ],
            Some("About it\n"),
        );
        assert_eq!(
            text,
            "TEST  Test project\nLeader:         jane\nTime tracking:  on\n\nAbout it"
        );
    }

    #[test]
    fn help_tree() {
        let mut settings = insta::Settings::clone_current();
        // Release PRs change the version; the snapshot must not.
        settings.add_filter(r"(?m)^yt \d+\.\d+\.\d+\S*$", "yt [version]");
        settings.bind(|| insta::assert_snapshot!(harness::help_tree(Yt::spec(), Page::Long)));
    }

    #[test]
    fn global_flags_parse_after_the_subcommand() {
        let words = harness::argv(["completion", "zsh", "--format", "json", "-vv"]);
        let cli = harness::parse(Yt::spec(), &words.words(), Yt::parse_from).unwrap();
        assert_eq!(cli.globals.format, Some(Format::Json));
        assert_eq!(cli.globals.verbose, 2);
    }

    #[test]
    fn reports_never_show_a_registered_secret() {
        yt::redact::register("perm-cmVwb3J0.dGVzdA==.report123456");
        let error = miette::miette!("server said: perm-cmVwb3J0.dGVzdA==.report123456");
        assert!(!render(&error).contains("report123456"));
    }

    #[test]
    fn an_expired_sign_in_reruns_the_command_only_before_any_change() {
        let expired: yt::Result<()> = Err(yt::YtError::SessionExpired {
            profile: "acme".into(),
        }
        .into());
        assert_eq!(
            expired_sign_in(&expired, false),
            Some(Expired::Rerun("acme".into()))
        );
        assert_eq!(
            expired_sign_in(&expired, true),
            Some(Expired::AfterChanges("acme".into()))
        );
        let other: yt::Result<()> = Err(yt::YtError::NoProfile.into());
        assert_eq!(expired_sign_in(&other, false), None);
        assert_eq!(expired_sign_in(&Ok(()), false), None);
    }
}
