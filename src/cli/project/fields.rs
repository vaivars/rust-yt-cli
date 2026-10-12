use serde_json::json;
use usage_rs::{Args, RunAsyncWith};
use yt::fields::schema::SchemaField;
use yt::output::{Format, Table};

use crate::cli::Ctx;

/// Show a project's custom fields
#[derive(Args)]
#[usage(effect = "read")]
pub(crate) struct Fields {
    /// Project short name
    #[usage(arg, name = "PROJECT")]
    project: String,
}

impl RunAsyncWith<&Ctx> for Fields {
    type Output = yt::Result<()>;

    async fn run_async_with(self, ctx: &Ctx) -> Self::Output {
        let client = ctx.client().await?;
        let projects = yt::api::projects::list(client).await?;
        let project = yt::api::projects::find(&projects, &self.project)?;
        // Always fresh: this is where people look when a value seems to be missing.
        let schema = ctx
            .schema_cache()?
            .refresh(client, &project.short_name)
            .await?;
        let json_of = |field: &SchemaField| {
            json!({
                "name": field.name,
                "aliases": field.aliases,
                "type": field.type_id,
                "multi": field.multi,
                "required": required(field),
                "values": field.values,
            })
        };
        match ctx.output.format() {
            Format::Table => {
                let mut table = Table::new(["NAME", "TYPE", "MULTI", "REQUIRED", "VALUES"]);
                for field in &schema.fields {
                    table.row([
                        field.name.clone(),
                        field.type_id.clone(),
                        yes_no(field.multi),
                        yes_no(required(field)),
                        yt::fields::choices(&field.values),
                    ]);
                }
                ctx.output.table(&table)
            }
            Format::Json => ctx
                .output
                .json(&schema.fields.iter().map(json_of).collect::<Vec<_>>()),
            Format::Ndjson => schema
                .fields
                .iter()
                .try_for_each(|field| ctx.output.ndjson(&json_of(field))),
        }
    }
}

/// A value must be given on create: the field cannot be empty and has no default.
fn required(field: &SchemaField) -> bool {
    !field.can_be_empty && !field.has_default
}

fn yes_no(value: bool) -> String {
    if value { "yes" } else { "no" }.into()
}
