mod fields;
mod list;
mod show;

use serde_json::{Value, json};
use usage_rs::{Args, Subcommands};
use yt::YtError;
use yt::api::Client;
use yt::api::projects::Project;
use yt::instance::InstanceUrl;
use yt::ui::Choice;

use crate::cli::Ctx;

/// Work with projects
#[derive(Args)]
#[usage(run_async_with)]
pub(crate) struct ProjectGroup {
    #[usage(subcommand)]
    command: ProjectCommand,
}

#[derive(Subcommands)]
#[usage(run_async_with)]
pub(crate) enum ProjectCommand {
    /// List projects
    #[usage(visible_alias = "ls")]
    List(list::List),
    /// Show a project
    Show(show::Show),
    /// Show a project's custom fields
    Fields(fields::Fields),
}

pub(crate) fn project_json(instance: &InstanceUrl, project: &Project) -> Value {
    json!({
        "short_name": project.short_name,
        "name": project.name,
        "leader": project.leader.as_ref().map(|leader| leader.login.as_str()),
        "archived": project.archived,
        "url": instance.project_url(&project.short_name).to_string(),
    })
}

pub(crate) async fn pick_project(ctx: &Ctx, client: &Client) -> yt::Result<String> {
    if !ctx.ui.is_interactive() {
        return Err(YtError::InputRequired {
            what: "the project".into(),
            flag: "-p".into(),
        }
        .into());
    }
    let choices = yt::api::projects::short_names(client)
        .await?
        .into_iter()
        .map(|name| Choice {
            value: name.clone(),
            label: name,
            description: None,
        })
        .collect();
    ctx.ui.select("Project", choices, "-p").await
}
