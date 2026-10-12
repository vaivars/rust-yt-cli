mod attachment;
mod comment;
mod create;
mod delete;
mod edit;
mod list;
mod show;

use serde_json::{Value, json};
use usage_rs::{Args, Subcommands};
use yt::api::articles::{Article, ArticleRow};
use yt::instance::InstanceUrl;

/// Work with knowledge base articles
#[derive(Args)]
#[usage(run_async_with)]
pub(crate) struct ArticleGroup {
    #[usage(subcommand)]
    command: ArticleCommand,
}

#[derive(Subcommands)]
#[usage(run_async_with)]
pub(crate) enum ArticleCommand {
    /// List articles matching a YouTrack query
    #[usage(visible_alias = "ls")]
    List(list::List),
    /// Show an article
    Show(show::Show),
    /// Create an article
    Create(create::Create),
    /// Change an article
    Edit(edit::Edit),
    /// Delete articles
    Delete(delete::Delete),
    /// Work with article comments
    Comment(comment::CommentGroup),
    /// Work with article attachments
    Attachment(attachment::AttachmentGroup),
}

pub(crate) fn article_row_json(instance: &InstanceUrl, row: &ArticleRow) -> Value {
    json!({
        "id": row.id_readable,
        "title": row.summary,
        "project": row.project.as_ref().map(|project| project.short_name.as_str()),
        "parent": row.parent_article.as_ref().map(|parent| parent.id_readable.as_str()),
        "children": row.child_articles.iter().map(|child| child.id_readable.as_str()).collect::<Vec<_>>(),
        "updated": yt::time::utc(row.updated),
        "url": instance.article_url(&row.id_readable).to_string(),
    })
}

pub(crate) fn article_json(instance: &InstanceUrl, article: &Article) -> Value {
    let mut value = article_row_json(instance, &article.row());
    value["content"] = json!(article.content);
    value["author"] = json!(
        article
            .reporter
            .as_ref()
            .map(|author| author.login.as_str())
    );
    value["created"] = json!(yt::time::utc(article.created));
    value
}
