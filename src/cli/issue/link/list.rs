use serde_json::json;
use usage_rs::{Args, RunAsyncWith};
use yt::api::custom_fields::state_of;
use yt::output::{Format, Table};

use crate::cli::Ctx;

/// List an issue's links
#[derive(Args)]
#[usage(effect = "read")]
pub(crate) struct List {
    /// Issue ID or URL
    #[usage(arg, name = "ID")]
    id: String,
}

impl RunAsyncWith<&Ctx> for List {
    type Output = yt::Result<()>;

    async fn run_async_with(self, ctx: &Ctx) -> Self::Output {
        let client = ctx.client().await?;
        let instance = client.instance();
        let id = instance.issue_id(&self.id)?;
        let links = yt::api::links::list(client, &id).await?;
        let rows: Vec<serde_json::Value> = links
            .iter()
            .flat_map(|link| {
                link.issues.iter().map(move |issue| {
                    json!({
                        "type": link.word(),
                        "direction": if link.inward() { "inward" } else { "outward" },
                        "issue": {
                            "id": issue.id_readable,
                            "summary": issue.summary,
                            "state": state_of(&issue.custom_fields),
                            "url": instance.issue_url(&issue.id_readable).to_string(),
                        },
                    })
                })
            })
            .collect();
        match ctx.output.format() {
            Format::Table => {
                let mut table = Table::new(["TYPE", "ISSUE", "STATE", "SUMMARY"]);
                for row in &rows {
                    let text = |value: &serde_json::Value| value.as_str().unwrap_or("").to_string();
                    table.row([
                        text(&row["type"]),
                        text(&row["issue"]["id"]),
                        text(&row["issue"]["state"]),
                        text(&row["issue"]["summary"]),
                    ]);
                }
                ctx.output.table(&table)
            }
            Format::Json => ctx.output.json(&rows),
            Format::Ndjson => rows.iter().try_for_each(|row| ctx.output.ndjson(row)),
        }
    }
}
