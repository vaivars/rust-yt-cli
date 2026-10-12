use serde::Serialize;
use usage_rs::{Args, RunAsyncWith};
use yt::config::profile::ProfileChoice;
use yt::output::{Format, Table};

use crate::cli::Ctx;

/// List profiles
#[derive(Args)]
#[usage(effect = "read")]
pub(crate) struct List;

#[derive(Serialize)]
struct ProfileJson<'a> {
    name: &'a str,
    url: &'a str,
    login: Option<&'a str>,
    auth: &'a str,
    project: Option<&'a str>,
    active: bool,
}

impl RunAsyncWith<&Ctx> for List {
    type Output = yt::Result<()>;

    async fn run_async_with(self, ctx: &Ctx) -> Self::Output {
        let config = ctx.global_config()?;
        let active = match ctx.profile() {
            Ok(ProfileChoice::Named { name, .. }) => Some(name),
            _ => None,
        };
        let rows: Vec<ProfileJson<'_>> = config
            .profiles
            .iter()
            .map(|(name, profile)| ProfileJson {
                name,
                url: &profile.url,
                login: profile.login.as_deref(),
                auth: profile.auth.as_str(),
                project: profile.project.as_deref(),
                active: active.as_deref() == Some(name.as_str()),
            })
            .collect();
        match ctx.output.format() {
            Format::Json => ctx.output.json(&rows),
            Format::Ndjson => rows.iter().try_for_each(|row| ctx.output.ndjson(row)),
            Format::Table => {
                let mut table = Table::new(["NAME", "URL", "LOGIN", "AUTH", "ACTIVE"]);
                for row in &rows {
                    table.row([
                        row.name,
                        row.url,
                        row.login.unwrap_or(""),
                        row.auth,
                        if row.active { "*" } else { "" },
                    ]);
                }
                ctx.output.table(&table)
            }
        }
    }
}
