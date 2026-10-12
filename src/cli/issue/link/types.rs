use serde_json::json;
use usage_rs::{Args, RunAsyncWith};
use yt::output::{Format, Table};

use crate::cli::Ctx;

/// List the instance's link types
#[derive(Args)]
#[usage(effect = "read")]
pub(crate) struct Types;

impl RunAsyncWith<&Ctx> for Types {
    type Output = yt::Result<()>;

    async fn run_async_with(self, ctx: &Ctx) -> Self::Output {
        let types = yt::api::links::types(ctx.client().await?).await?;
        let rows: Vec<serde_json::Value> = types
            .iter()
            .map(|link_type| {
                json!({
                    "name": link_type.name,
                    "outward": link_type.source_to_target,
                    "inward": link_type.target_to_source,
                    "directed": link_type.directed,
                })
            })
            .collect();
        match ctx.output.format() {
            Format::Table => {
                let mut table = Table::new(["NAME", "OUTWARD", "INWARD"]);
                for link_type in &types {
                    table.row([
                        link_type.name.clone(),
                        link_type.source_to_target.clone().unwrap_or_default(),
                        link_type.target_to_source.clone().unwrap_or_default(),
                    ]);
                }
                ctx.output.table(&table)
            }
            Format::Json => ctx.output.json(&rows),
            Format::Ndjson => rows.iter().try_for_each(|row| ctx.output.ndjson(row)),
        }
    }
}
