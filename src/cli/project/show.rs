use usage_rs::{Args, RunAsyncWith};
use yt::output::Format;

use super::project_json;
use crate::cli::Ctx;

/// Show a project
#[derive(Args)]
#[usage(effect = "read")]
pub(crate) struct Show {
    /// Project short name
    #[usage(arg, name = "PROJECT")]
    project: String,
    /// Open the project in the browser instead of printing it
    #[usage(short, long)]
    web: bool,
}

impl RunAsyncWith<&Ctx> for Show {
    type Output = yt::Result<()>;

    async fn run_async_with(self, ctx: &Ctx) -> Self::Output {
        let client = ctx.client().await?;
        let instance = client.instance();
        let projects = yt::api::projects::list(client).await?;
        let project = yt::api::projects::find(&projects, &self.project)?;
        let url = instance.project_url(&project.short_name);
        if self.web {
            if !yt::browser::open(&ctx.env, url.as_str()) {
                ctx.output.write(url.as_str())?;
            }
            return Ok(());
        }
        // Some instances hide project settings from non-admins; the project is still worth showing.
        let tracking = match yt::api::projects::time_tracking(client, &project.short_name).await {
            Ok(settings) => Some(settings.enabled),
            Err(error) if yt::api::work_items::forbidden(&error) => None,
            Err(error) => return Err(error),
        };
        let cache = ctx.schema_cache()?;
        let schema = match cache.load(&project.short_name) {
            Some(schema) => schema,
            None => cache.refresh(client, &project.short_name).await?,
        };
        let fields: Vec<String> = schema
            .fields
            .iter()
            .map(|field| field.name.clone())
            .collect();
        if ctx.output.format() != Format::Table {
            let mut value = project_json(instance, project);
            value["description"] = project.description.clone().into();
            value["time_tracking"] = tracking.into();
            value["fields"] = fields.into();
            return if ctx.output.format() == Format::Json {
                ctx.output.json(&value)
            } else {
                ctx.output.ndjson(&value)
            };
        }
        let mut rows: Vec<(String, String)> = Vec::new();
        if let Some(leader) = &project.leader {
            rows.push(("Leader".into(), leader.login.clone()));
        }
        if project.archived {
            rows.push(("Archived".into(), "yes".into()));
        }
        if let Some(enabled) = tracking {
            rows.push((
                "Time tracking".into(),
                if enabled { "on" } else { "off" }.into(),
            ));
        }
        if !fields.is_empty() {
            rows.push(("Fields".into(), fields.join(", ")));
        }
        rows.push(("URL".into(), url.to_string()));
        let heading = format!("{}  {}", project.short_name, project.name);
        ctx.output.write(&crate::cli::details(
            &heading,
            rows,
            project.description.as_deref(),
        ))
    }
}
