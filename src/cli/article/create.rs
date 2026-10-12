use usage_rs::{Args, RunAsyncWith};
use yt::YtError;
use yt::output::Format;

use super::article_json;
use crate::cli::Ctx;
use crate::cli::issue::text;

/// Create an article
#[derive(Args)]
#[usage(effect = "write")]
pub(crate) struct Create {
    /// Project short name
    #[usage(short, long, value_name = "PROJECT")]
    project: Option<String>,
    /// Title
    #[usage(short, long, value_name = "TEXT")]
    summary: Option<String>,
    /// Content (Markdown)
    #[usage(short, long, value_name = "TEXT")]
    body: Option<String>,
    /// Read the content from a file, or `-` for stdin
    #[usage(long, value_name = "PATH")]
    body_file: Option<String>,
    /// Parent article ID or URL; the new article becomes its child
    #[usage(long, value_name = "ID")]
    parent: Option<String>,
    /// Open the new article in the browser instead of printing its URL
    #[usage(short, long)]
    web: bool,
}

impl RunAsyncWith<&Ctx> for Create {
    type Output = yt::Result<()>;

    async fn run_async_with(self, ctx: &Ctx) -> Self::Output {
        let content = text::main_text(self.body, self.body_file.as_deref()).await?;
        let instance = ctx.instance()?;
        let parent = match &self.parent {
            Some(raw) => Some(instance.article_id(raw)?),
            None => None,
        };
        let client = ctx.client().await?;
        let project = match (&self.project, &parent) {
            (Some(project), _) => known_project(client, project).await?,
            (None, Some(parent)) => yt::api::articles::get(client, parent)
                .await?
                .project
                .map(|project| project.short_name)
                .ok_or_else(|| YtError::Usage {
                    message: format!("{parent} has no project; pass -p"),
                })?,
            (None, None) => {
                match yt::config::profile::project(None, ctx.repo()?, &ctx.profile()?) {
                    Some(project) => known_project(client, &project).await?,
                    None => crate::cli::project::pick_project(ctx, client).await?,
                }
            }
        };
        let title = match self.summary {
            Some(title) => title,
            None => ctx.ui.input("Title", None, None, "-s").await?,
        };
        let title = title.trim().to_string();
        if title.is_empty() {
            return Err(YtError::Usage {
                message: "the title cannot be empty".into(),
            }
            .into());
        }
        let content = match content {
            Some(content) => Some(content),
            None if ctx.ui.is_interactive() => {
                Some(yt::editor::edit(&ctx.env, &ctx.dirs.state, "").await?)
            }
            None => None,
        }
        .map(|content| content.trim_end().to_string())
        .filter(|content| !content.is_empty());
        let id = yt::api::articles::create(
            client,
            &project,
            &title,
            content.as_deref(),
            parent.as_deref(),
        )
        .await?;
        let url = instance.article_url(&id);
        if self.web && yt::browser::open(&ctx.env, url.as_str()) {
            return Ok(());
        }
        match ctx.output.format() {
            Format::Table => ctx.output.write(url.as_str()),
            format => {
                let article = yt::api::articles::get(client, &id).await?;
                if format == Format::Json {
                    ctx.output.json(&article_json(&instance, &article))
                } else {
                    ctx.output.ndjson(&article_json(&instance, &article))
                }
            }
        }
    }
}

/// The project's short name as the instance spells it; checked before the editor opens so no writing is lost.
async fn known_project(client: &yt::api::Client, name: &str) -> yt::Result<String> {
    let projects = yt::api::projects::list(client).await?;
    Ok(yt::api::projects::find(&projects, name)?.short_name.clone())
}
