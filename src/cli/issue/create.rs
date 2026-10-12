use serde_json::{Value, json};
use usage_rs::{Args, RunAsyncWith};
use yt::YtError;
use yt::fields::change::{self, Change, Op};
use yt::fields::plan::{self, Operation};
use yt::fields::resolve::{self, Resolved, ResolvedOp};
use yt::fields::schema::{ProjectSchema, SchemaField};
use yt::output::Format;
use yt::ui::Choice;

use super::json::issue_json;
use super::text;
use crate::cli::Ctx;

/// Create an issue
#[derive(Args)]
#[usage(effect = "write")]
pub(crate) struct Create {
    /// Project short name
    #[usage(short, long, value_name = "PROJECT")]
    project: Option<String>,
    /// Summary
    #[usage(short, long, value_name = "TEXT")]
    summary: Option<String>,
    /// Description (Markdown)
    #[usage(short, long, value_name = "TEXT")]
    body: Option<String>,
    /// Read the description from a file, or `-` for stdin
    #[usage(long, value_name = "PATH")]
    body_file: Option<String>,
    /// Custom field: NAME=VALUE (repeatable)
    #[usage(short = 'f', long = "field", value_name = "NAME=VALUE", var)]
    fields: Vec<String>,
    /// Show what would be created without creating it
    #[usage(long)]
    dry_run: bool,
    /// Skip client-side field validation
    #[usage(long)]
    no_validate: bool,
    /// Open the new issue in the browser instead of printing its URL
    #[usage(short, long)]
    web: bool,
}

impl RunAsyncWith<&Ctx> for Create {
    type Output = yt::Result<()>;

    async fn run_async_with(self, ctx: &Ctx) -> Self::Output {
        let description = text::main_text(self.body, self.body_file.as_deref()).await?;
        let changes = change::parse(&self.fields)?;
        let client = ctx.client().await?;
        let project = match yt::config::profile::project(
            self.project.as_deref(),
            ctx.repo()?,
            &ctx.profile()?,
        ) {
            Some(project) => project,
            None => crate::cli::project::pick_project(ctx, client).await?,
        };
        let validate = !self.no_validate;
        let cache = ctx.schema_cache()?;
        let (schema, mut resolved) =
            yt::fields::resolve_for(client, &cache, &project, &changes, validate).await?;
        let summary = match self.summary {
            Some(summary) => summary,
            None => ctx.ui.input("Summary", None, None, "-s").await?,
        };
        let summary = summary.trim().to_string();
        if summary.is_empty() {
            return Err(YtError::Usage {
                message: "the summary cannot be empty".into(),
            }
            .into());
        }
        let description = match description {
            Some(text) => Some(text),
            None if ctx.ui.is_interactive()
                && ctx
                    .ui
                    .confirm("Write a description in your editor?", false, "-b")
                    .await? =>
            {
                let text = yt::editor::edit(&ctx.env, &ctx.dirs.state, "").await?;
                Some(text).filter(|text| !text.trim().is_empty())
            }
            None => None,
        };
        if validate {
            let missing: Vec<SchemaField> = resolve::missing_required(&schema, &resolved)
                .into_iter()
                .cloned()
                .collect();
            for field in missing {
                resolved.extend(ask_field(ctx, &schema, &field).await?);
            }
        }
        let (body, commands) =
            plan::create_request(&project, &summary, description.as_deref(), &resolved)?;
        if self.dry_run {
            return print_dry_run(ctx, &project, &summary, &resolved, &body, &commands);
        }
        let id = yt::api::issues::create(client, &body).await?;
        let operations: Vec<Operation> = commands
            .into_iter()
            .map(|command| Operation::Command {
                issues: vec![id.clone()],
                command,
            })
            .collect();
        yt::fields::apply::execute(client, &operations)
            .await
            .map_err(|error| {
                error.wrap_err(format!("{id} was created, but setting its fields failed"))
            })?;
        let url = client.instance().issue_url(&id);
        if self.web && yt::browser::open(&ctx.env, url.as_str()) {
            return Ok(());
        }
        let format = ctx.output.format();
        if format == Format::Table {
            return ctx.output.write(url.as_str());
        }
        let issue = yt::api::issues::get(client, &id).await?;
        if format == Format::Json {
            ctx.output.json(&issue_json(&issue))
        } else {
            ctx.output.ndjson(&issue_json(&issue))
        }
    }
}

/// Asks for a required field the flags left out; without a terminal, an error names the flag.
async fn ask_field(
    ctx: &Ctx,
    schema: &ProjectSchema,
    field: &SchemaField,
) -> yt::Result<Vec<Resolved>> {
    let flag = format!("-f '{}=…'", field.name);
    let value = if field.values.is_empty() {
        ctx.ui.input(&field.name, None, None, &flag).await?
    } else {
        let choices = field
            .values
            .iter()
            .map(|value| Choice {
                value: value.clone(),
                label: value.clone(),
                description: None,
            })
            .collect();
        ctx.ui.select(&field.name, choices, &flag).await?
    };
    let change = Change {
        name: field.name.clone(),
        op: Op::Set(vec![value]),
    };
    resolve::resolve(schema, &[change], None, true)
}

fn print_dry_run(
    ctx: &Ctx,
    project: &str,
    summary: &str,
    resolved: &[Resolved],
    body: &Value,
    commands: &[String],
) -> yt::Result<()> {
    if ctx.output.format() != Format::Table {
        return ctx.output.json(&json!({"operations": [
            {"kind": "create", "project": project, "body": body, "commands": commands}
        ]}));
    }
    let mut lines = vec![format!("create in {project}: summary \"{summary}\"")];
    for change in resolved {
        match &change.op {
            ResolvedOp::Set(values) => lines.push(format!(
                "  set {} to {}",
                change.field.name,
                plan::describe_values(&change.field, values)
            )),
            ResolvedOp::Add(value) => lines.push(format!("  add {value} to {}", change.field.name)),
            ResolvedOp::Remove(_) | ResolvedOp::Clear => {}
        }
    }
    ctx.output.write(&lines.join("\n"))
}
