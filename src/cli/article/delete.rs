use usage_rs::{Args, RunAsyncWith};
use yt::api::Client;
use yt::output::Format;

use crate::cli::Ctx;

/// Delete articles
#[derive(Args)]
#[usage(effect = "destructive")]
pub(crate) struct Delete {
    /// Article IDs or URLs
    #[usage(arg, name = "ID", required)]
    ids: Vec<String>,
}

impl RunAsyncWith<&Ctx> for Delete {
    type Output = yt::Result<()>;

    async fn run_async_with(self, ctx: &Ctx) -> Self::Output {
        let instance = ctx.instance()?;
        let mut ids: Vec<String> = Vec::new();
        for raw in &self.ids {
            let id = instance.article_id(raw)?;
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
        let client = ctx.client().await?;
        let mut cascaded: Vec<String> = Vec::new();
        for id in &ids {
            for descendant in descendants(client, id).await? {
                if !cascaded.contains(&descendant) {
                    cascaded.push(descendant);
                }
            }
        }
        let children: Vec<&str> = cascaded
            .iter()
            .filter(|id| !ids.contains(id))
            .map(String::as_str)
            .collect();
        let mut action = format!("delete {}", ids.join(", "));
        if !children.is_empty() {
            // YouTrack deletes child articles with their parent.
            action.push_str(&format!(
                " (also deletes child articles: {})",
                children.join(", ")
            ));
        }
        ctx.ui.confirm_destructive(&action).await?;
        let mut deleted: Vec<String> = Vec::new();
        for id in ids.iter().filter(|id| !cascaded.contains(id)) {
            if let Err(error) = yt::api::articles::delete(client, id).await {
                return Err(if deleted.is_empty() {
                    error
                } else {
                    error.wrap_err(format!(
                        "stopped after a failure; already deleted: {}",
                        deleted.join(", ")
                    ))
                });
            }
            deleted.push(id.clone());
        }
        let deleted = ids;
        match ctx.output.format() {
            Format::Table => {
                ctx.note(&format!("Deleted {}", deleted.join(", ")));
                Ok(())
            }
            Format::Json => ctx.output.json(&deleted),
            Format::Ndjson => deleted.iter().try_for_each(|id| ctx.output.ndjson(id)),
        }
    }
}

/// Every article below `id`, nearest first.
async fn descendants(client: &Client, id: &str) -> yt::Result<Vec<String>> {
    let mut found = vec![id.to_string()];
    let mut next = 0;
    while next < found.len() {
        let article = yt::api::articles::get(client, &found[next]).await?;
        for child in article.child_articles {
            if !found.contains(&child.id_readable) {
                found.push(child.id_readable);
            }
        }
        next += 1;
    }
    Ok(found.split_off(1))
}
