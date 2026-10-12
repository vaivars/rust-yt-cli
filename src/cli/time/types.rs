use serde_json::json;
use usage_rs::{Args, RunAsyncWith};
use yt::output::{Format, Table};

use crate::cli::Ctx;

/// List work item types
#[derive(Args)]
#[usage(effect = "read")]
pub(crate) struct Types {
    /// Project short name (default: the repo or profile project; without one, the instance's types)
    #[usage(short, long, value_name = "PROJECT")]
    project: Option<String>,
}

impl RunAsyncWith<&Ctx> for Types {
    type Output = yt::Result<()>;

    async fn run_async_with(self, ctx: &Ctx) -> Self::Output {
        let client = ctx.client().await?;
        let project =
            yt::config::profile::project(self.project.as_deref(), ctx.repo()?, &ctx.profile()?);
        let types = yt::api::work_items::types(client, project.as_deref()).await?;
        let json_of = |id: &str, name: &str| json!({"id": id, "name": name});
        match ctx.output.format() {
            Format::Table => {
                let mut table = Table::new(["NAME"]);
                for work_type in &types {
                    table.row([work_type.name.clone()]);
                }
                ctx.output.table(&table)
            }
            Format::Json => ctx.output.json(
                &types
                    .iter()
                    .map(|work_type| json_of(&work_type.id, &work_type.name))
                    .collect::<Vec<_>>(),
            ),
            Format::Ndjson => types.iter().try_for_each(|work_type| {
                ctx.output.ndjson(&json_of(&work_type.id, &work_type.name))
            }),
        }
    }
}
