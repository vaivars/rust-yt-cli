use usage_rs::complete::{self, CompletionRequest, Completions, Split};
use usage_rs::spec::Candidate;
use yt::apps::cache::AppCache;
use yt::apps::openapi::AppApi;
use yt::apps::registry::{self, AppEntry};
use yt::config::file::ConfigStore;
use yt::config::profile::ProfileChoice;
use yt::dirs::Dirs;
use yt::env::Env;

use super::tree::{self, global_longs};
use crate::cli::Yt;

/// Adds the enabled apps after `yt app`, and an app's commands from its cached description.
/// Reads local files only: a Tab press never touches the network.
pub(crate) async fn answer(request: &CompletionRequest) -> String {
    let split = &request.split;
    let position = complete::walk(Yt::spec().root.cmd, split.argv());
    let answer = match position.external {
        // `external` counts from the first word after the program name.
        Some(index) => app_commands(split, index + 1),
        None => {
            let mut answer = crate::cli::completion::app()
                .complete_request(request)
                .await;
            let app_group = Yt::spec()
                .root
                .cmd
                .subcommands
                .iter()
                .find(|command| command.name == "app");
            if app_group.is_some_and(|group| std::ptr::eq(*group, position.cmd))
                && position.awaiting_value.is_none()
                && !split.prefix.starts_with('-')
            {
                let apps: Vec<Candidate> = enabled(split)
                    .into_iter()
                    .filter(|(app, _)| app.id.starts_with(&split.prefix))
                    .map(|(app, _)| Candidate::described(app.id, app.description))
                    .collect();
                if !apps.is_empty() {
                    answer.files = None;
                    answer.candidates.extend(apps);
                }
            }
            answer
        }
    };
    complete::render_request(&answer, request)
}

/// Completes the words after `yt app <APP>` against the app's own command line.
fn app_commands(split: &Split, app_word: usize) -> Completions<'static> {
    let none = Completions {
        candidates: Vec::new(),
        files: None,
    };
    let Some(word) = split.words.get(app_word) else {
        return none;
    };
    let Some(cache) = enabled(split)
        .into_iter()
        .find(|(app, _)| app.id == word)
        .map(|(_, cache)| cache)
    else {
        return none;
    };
    let Some(api) = cache
        .load_stale(word)
        .and_then(|text| AppApi::parse(word, &text, &global_longs()).ok())
    else {
        return none;
    };
    let sub = Split {
        words: split.words[app_word..].to_vec(),
        cword: split.cword.saturating_sub(app_word),
        prefix: split.prefix.clone(),
    };
    complete::complete(tree::build(&api).spec, &sub)
}

/// The apps the profile on this line uses, each with the profile's description cache.
fn enabled(split: &Split) -> Vec<(&'static AppEntry, AppCache)> {
    let env = Env::from_process();
    let Ok(dirs) = Dirs::from_env(&env) else {
        return Vec::new();
    };
    let Ok(config) = ConfigStore::new(dirs.config_file()).load() else {
        return Vec::new();
    };
    let repo = std::env::current_dir()
        .ok()
        .and_then(|cwd| yt::config::repo::find(&cwd).ok().flatten());
    let flag = profile_flag(&split.words).or_else(|| env.get("YT_PROFILE").map(str::to_string));
    let Ok(choice) = yt::config::profile::resolve(flag.as_deref(), &env, repo.as_ref(), &config)
    else {
        return Vec::new();
    };
    let Ok(instance) = choice.url() else {
        return Vec::new();
    };
    let cache = || {
        AppCache::for_profile(
            &dirs.cache,
            choice.label(),
            &instance,
            std::time::Duration::ZERO,
        )
    };
    registry::APPS
        .iter()
        .filter(|app| match &choice {
            ProfileChoice::Env { .. } => true,
            ProfileChoice::Named { profile, .. } => profile.apps.iter().any(|id| id == app.id),
        })
        .map(|app| (app, cache()))
        .collect()
}

fn profile_flag(words: &[String]) -> Option<String> {
    let mut words = words.iter();
    while let Some(word) = words.next() {
        if word == "--profile" {
            return words.next().cloned();
        }
        if let Some(name) = word.strip_prefix("--profile=") {
            return Some(name.to_string());
        }
    }
    None
}
