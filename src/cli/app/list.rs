use serde::Serialize;
use usage_rs::{Args, RunAsyncWith};
use yt::apps::registry::APPS;
use yt::config::profile::ProfileChoice;
use yt::output::{Format, Table};

use crate::cli::Ctx;

/// List the apps yt can use and which are enabled
#[derive(Args)]
#[usage(effect = "read")]
pub(crate) struct List;

#[derive(Serialize)]
struct AppJson {
    id: &'static str,
    name: &'static str,
    description: &'static str,
    homepage: &'static str,
    enabled: bool,
}

impl RunAsyncWith<&Ctx> for List {
    type Output = yt::Result<()>;

    async fn run_async_with(self, ctx: &Ctx) -> Self::Output {
        let enabled = |id: &str| match ctx.profile() {
            Ok(ProfileChoice::Named { profile, .. }) => profile.apps.iter().any(|app| app == id),
            Ok(ProfileChoice::Env { .. }) => true,
            Err(_) => false,
        };
        let rows: Vec<AppJson> = APPS
            .iter()
            .map(|app| AppJson {
                id: app.id,
                name: app.name,
                description: app.description,
                homepage: app.homepage,
                enabled: enabled(app.id),
            })
            .collect();
        match ctx.output.format() {
            Format::Json => ctx.output.json(&rows),
            Format::Ndjson => rows.iter().try_for_each(|row| ctx.output.ndjson(row)),
            Format::Table => {
                let mut table = Table::new(["ID", "ENABLED", "DESCRIPTION"]);
                for row in &rows {
                    table.row([row.id, if row.enabled { "*" } else { "" }, row.description]);
                }
                ctx.output.table(&table)
            }
        }
    }
}
