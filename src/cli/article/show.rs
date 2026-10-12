use usage_rs::{Args, RunAsyncWith};
use yt::output::Format;

use super::article_json;
use crate::cli::Ctx;

/// Show an article
#[derive(Args)]
#[usage(effect = "read")]
pub(crate) struct Show {
    /// Article ID (KB-A-12) or article URL
    #[usage(arg, name = "ID")]
    id: String,
    /// Open the article in the browser instead of printing it
    #[usage(short, long)]
    web: bool,
}

impl RunAsyncWith<&Ctx> for Show {
    type Output = yt::Result<()>;

    async fn run_async_with(self, ctx: &Ctx) -> Self::Output {
        let instance = ctx.instance()?;
        let id = instance.article_id(&self.id)?;
        if self.web {
            let url = instance.article_url(&id);
            if !yt::browser::open(&ctx.env, url.as_str()) {
                ctx.output.write(url.as_str())?;
            }
            return Ok(());
        }
        let client = ctx.client().await?;
        let article = yt::api::articles::get(client, &id).await?;
        match ctx.output.format() {
            Format::Json => return ctx.output.json(&article_json(&instance, &article)),
            Format::Ndjson => return ctx.output.ndjson(&article_json(&instance, &article)),
            Format::Table => {}
        }
        let mut rows: Vec<(String, String)> = Vec::new();
        if let Some(project) = &article.project {
            rows.push(("Project".into(), project.short_name.clone()));
        }
        if let Some(parent) = &article.parent_article {
            rows.push(("Parent".into(), parent.id_readable.clone()));
        }
        if !article.child_articles.is_empty() {
            let children: Vec<&str> = article
                .child_articles
                .iter()
                .map(|child| child.id_readable.as_str())
                .collect();
            rows.push(("Children".into(), children.join(", ")));
        }
        if let Some(author) = &article.reporter {
            rows.push(("Author".into(), author.login.clone()));
        }
        rows.push(("Created".into(), yt::time::local(article.created)));
        rows.push(("Updated".into(), yt::time::local(article.updated)));
        rows.push(("URL".into(), instance.article_url(&id).to_string()));
        let heading = format!("{id}  {}", article.summary.as_deref().unwrap_or(""));
        ctx.output.write(&crate::cli::details(
            &heading,
            rows,
            article.content.as_deref(),
        ))
    }
}
