use usage_rs::{Args, RunAsyncWith};
use yt::output::{Format, Table};

use super::project_json;
use crate::cli::Ctx;

/// List projects
#[derive(Args)]
#[usage(effect = "read")]
pub(crate) struct List {
    /// How many projects to show
    #[usage(short = 'n', long, default = "50")]
    limit: usize,
    /// Show every project
    #[usage(long, conflicts("--limit"))]
    all: bool,
}

impl RunAsyncWith<&Ctx> for List {
    type Output = yt::Result<()>;

    async fn run_async_with(self, ctx: &Ctx) -> Self::Output {
        crate::cli::check_limit(self.limit)?;
        let client = ctx.client().await?;
        let instance = client.instance();
        let mut projects = yt::api::projects::list(client).await?;
        if !self.all {
            projects.truncate(self.limit);
        }
        if projects.is_empty() {
            ctx.note("No projects found.");
        }
        match ctx.output.format() {
            Format::Table => {
                let mut table = Table::new(["SHORT", "NAME", "LEADER"]);
                for project in &projects {
                    let name = if project.archived {
                        format!("{} (archived)", project.name)
                    } else {
                        project.name.clone()
                    };
                    table.row([
                        project.short_name.clone(),
                        name,
                        project
                            .leader
                            .as_ref()
                            .map(|leader| leader.login.clone())
                            .unwrap_or_default(),
                    ]);
                }
                ctx.output.table(&table)
            }
            Format::Json => ctx.output.json(
                &projects
                    .iter()
                    .map(|project| project_json(instance, project))
                    .collect::<Vec<_>>(),
            ),
            Format::Ndjson => projects
                .iter()
                .try_for_each(|project| ctx.output.ndjson(&project_json(instance, project))),
        }
    }
}
