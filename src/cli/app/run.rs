use std::ffi::OsString;

use reqwest::Method;
use yt::YtError;
use yt::api::Client;
use yt::apps::Stale;
use yt::apps::openapi::{AppApi, Operation, Scope};
use yt::apps::registry::{self, AppEntry};
use yt::apps::request::{self, Request};
use yt::config::profile::ProfileChoice;

use super::add::unknown;
use super::tree::{self, Parsed, global_longs};
use crate::cli::Ctx;

/// `yt app <APP> <COMMAND> …`: `words` starts with the app's id.
pub(crate) async fn run(ctx: &Ctx, words: Vec<OsString>) -> yt::Result<()> {
    let Some((name, rest)) = words.split_first() else {
        return Err(unknown(""));
    };
    let app = enabled(ctx, &name.to_string_lossy())?;
    let client = ctx.client().await?;
    let cache = ctx.app_cache()?;
    let taken = global_longs();
    let api = yt::apps::load(client, &cache, app, false, &taken).await?;
    let Some((operation, request)) = prepare(ctx, client, &api, rest).await? else {
        return Ok(());
    };
    let error = match send(ctx, client, &request).await {
        Ok(()) => return Ok(()),
        Err(error) => error,
    };
    let Some(stale) = yt::apps::stale(&error) else {
        return Err(error);
    };
    let fresh = yt::apps::load(client, &cache, app, true, &taken).await?;
    // A read can go to whichever endpoint is still on; a write is never sent twice.
    if request.method == Method::GET
        && let Ok(Some((retry, again))) = prepare(ctx, client, &fresh, rest).await
        && retry.path != operation.path
    {
        return send(ctx, client, &again).await;
    }
    Err(explain(error, stale, &operation, app))
}

fn enabled(ctx: &Ctx, name: &str) -> yt::Result<&'static AppEntry> {
    let app = registry::find(name).ok_or_else(|| unknown(name))?;
    match ctx.profile()? {
        ProfileChoice::Env { .. } => Ok(app),
        ProfileChoice::Named { profile, .. } if profile.apps.iter().any(|id| id == app.id) => {
            Ok(app)
        }
        ProfileChoice::Named { name, .. } => Err(YtError::AppNotEnabled {
            id: app.id.to_string(),
            profile: name,
        }
        .into()),
    }
}

/// The request the words ask for, or `None` once help has been printed.
async fn prepare(
    ctx: &Ctx,
    client: &Client,
    api: &AppApi,
    words: &[OsString],
) -> yt::Result<Option<(Operation, Request)>> {
    let call = match tree::parse(api, words)? {
        Parsed::Help(page) => {
            ctx.output.write(page.trim_end())?;
            return Ok(None);
        }
        Parsed::Call(call) => call,
    };
    let operation = api.operations[call.operation].clone();
    let mut inputs = call.inputs;
    if let Some(source) = &call.input {
        inputs.input = Some(crate::cli::api::read_body(source).await?);
    }
    let request = request::build(&operation, &inputs, client.instance())?;
    Ok(Some((operation, request)))
}

async fn send(ctx: &Ctx, client: &Client, request: &Request) -> yt::Result<()> {
    match client
        .raw(
            request.method.clone(),
            &request.path_and_query,
            request.body.as_ref(),
        )
        .await?
    {
        Some(text) => ctx.output.raw_json(&text),
        None => Ok(()),
    }
}

fn explain(
    error: miette::Report,
    stale: Stale,
    operation: &Operation,
    app: &AppEntry,
) -> miette::Report {
    let id = app.id;
    let refreshed = format!("yt refreshed the app's commands; see `yt app {id} --help`");
    let help = match (stale, operation.scope) {
        (Stale::Disabled, _) => {
            format!("an administrator disabled this endpoint of the {id} app; {refreshed}")
        }
        (Stale::NoHandler, Scope::Issue) => {
            format!("the {id} app is not attached to this issue's project")
        }
        (Stale::NoHandler, Scope::Article) => {
            format!("the {id} app is not attached to this article's project")
        }
        (Stale::NoHandler, Scope::Project) => {
            format!("the {id} app is not attached to this project")
        }
        (Stale::NoHandler, _) => format!("the {id} app no longer has this endpoint; {refreshed}"),
    };
    match error.downcast::<YtError>() {
        Ok(YtError::Api {
            method,
            path,
            status,
            code,
            message,
            ..
        }) => YtError::Api {
            method,
            path,
            status,
            code,
            message,
            help: Some(help),
        }
        .into(),
        Ok(other) => other.into(),
        Err(error) => error,
    }
}
