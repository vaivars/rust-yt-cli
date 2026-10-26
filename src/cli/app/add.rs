use usage_rs::{Args, RunAsyncWith};
use yt::YtError;
use yt::apps::registry::{self, AppEntry};
use yt::config::profile::ProfileChoice;

use super::tree::global_longs;
use crate::cli::Ctx;

/// Check that an app is installed and use it with the profile
#[derive(Args)]
#[usage(effect = "write")]
pub(crate) struct Add {
    /// App ID, as `yt app list` shows it
    #[usage(arg, name = "APP")]
    id: String,
}

impl RunAsyncWith<&Ctx> for Add {
    type Output = yt::Result<()>;

    async fn run_async_with(self, ctx: &Ctx) -> Self::Output {
        let app = known(&self.id)?;
        let profile = ctx.profile()?;
        let client = ctx.client().await?;
        let api = yt::apps::load(client, &ctx.app_cache()?, app, true, &global_longs())
            .await?
            .api;
        let commands: Vec<&str> = api
            .operations
            .iter()
            .map(|operation| operation.command.as_str())
            .collect();
        let what = format!("{} {}", app.id, api.version);
        let available = if commands.is_empty() {
            "every endpoint is disabled in the app's settings".to_string()
        } else {
            commands.join(", ")
        };
        match &profile {
            ProfileChoice::Named { name, profile } => {
                if !profile.apps.iter().any(|enabled| enabled == app.id) {
                    let mut apps = profile.apps.clone();
                    apps.push(app.id.to_string());
                    ctx.config_store().set_apps(name, &apps)?;
                }
                ctx.note(&format!("Enabled {what} for profile {name}: {available}"));
            }
            ProfileChoice::Env { url, .. } => {
                ctx.note(&format!("{what} is available on {url}: {available}"));
            }
        }
        Ok(())
    }
}

/// Stop using an app with the profile
#[derive(Args)]
#[usage(effect = "write")]
pub(crate) struct Remove {
    /// App ID, as `yt app list` shows it
    #[usage(arg, name = "APP")]
    id: String,
}

impl RunAsyncWith<&Ctx> for Remove {
    type Output = yt::Result<()>;

    async fn run_async_with(self, ctx: &Ctx) -> Self::Output {
        let ProfileChoice::Named { name, profile } = ctx.profile()? else {
            return Err(YtError::Usage {
                message:
                    "a profile from YOUTRACK_URL does not save apps, so there is nothing to remove"
                        .into(),
            }
            .into());
        };
        if !profile.apps.iter().any(|enabled| enabled == &self.id) {
            known(&self.id)?;
            ctx.note(&format!("{} is not enabled for profile {name}", self.id));
            return Ok(());
        }
        let apps: Vec<String> = profile
            .apps
            .iter()
            .filter(|enabled| **enabled != self.id)
            .cloned()
            .collect();
        ctx.config_store().set_apps(&name, &apps)?;
        ctx.app_cache()?.forget(&self.id);
        ctx.note(&format!("Removed {} from profile {name}", self.id));
        Ok(())
    }
}

pub(super) fn known(id: &str) -> yt::Result<&'static AppEntry> {
    registry::find(id).ok_or_else(|| unknown(id))
}

/// `yt app` has its own commands and the registry's apps; anything else gets a suggestion.
pub(super) fn unknown(name: &str) -> miette::Report {
    let candidates = ["list", "add", "remove"].into_iter().chain(registry::ids());
    let help = match yt::fields::closest(name, candidates) {
        Some(candidate) => format!("did you mean '{candidate}'?"),
        None => "run `yt app list` to see the apps yt can use".into(),
    };
    YtError::UnknownApp {
        name: name.to_string(),
        help: Some(help),
    }
    .into()
}
