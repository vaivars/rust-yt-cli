use serde::Serialize;
use yt::YtError;
use yt::api::Parent;
use yt::api::comments::Comment;
use yt::instance::InstanceUrl;
use yt::output::{Format, Table};

use crate::cli::Ctx;
use crate::cli::issue::text;

#[derive(Serialize)]
pub(crate) struct CommentJson {
    id: String,
    author: Option<String>,
    created: String,
    updated: Option<String>,
    text: String,
    url: String,
}

pub(crate) fn comment_json(
    instance: &InstanceUrl,
    parent: &Parent,
    comment: &Comment,
) -> CommentJson {
    CommentJson {
        id: comment.id.clone(),
        author: comment.author.as_ref().map(|author| author.login.clone()),
        created: yt::time::utc(comment.created),
        updated: comment.updated.map(yt::time::utc),
        text: comment.text.clone().unwrap_or_default(),
        url: yt::api::comments::url(instance, parent, &comment.id).to_string(),
    }
}

pub(crate) async fn list(ctx: &Ctx, parent: Parent) -> yt::Result<()> {
    let client = ctx.client().await?;
    let instance = client.instance();
    let comments = yt::api::comments::list(client, &parent).await?;
    match ctx.output.format() {
        Format::Table => {
            let mut table = Table::new(["ID", "AUTHOR", "CREATED", "TEXT"]);
            for comment in &comments {
                let text = comment.text.as_deref().unwrap_or("");
                table.row([
                    comment.id.clone(),
                    comment
                        .author
                        .as_ref()
                        .map(|author| author.login.clone())
                        .unwrap_or_default(),
                    yt::time::local(comment.created),
                    text.lines().next().unwrap_or("").to_string(),
                ]);
            }
            ctx.output.table(&table)
        }
        Format::Json => ctx.output.json(
            &comments
                .iter()
                .map(|comment| comment_json(instance, &parent, comment))
                .collect::<Vec<_>>(),
        ),
        Format::Ndjson => comments
            .iter()
            .try_for_each(|comment| ctx.output.ndjson(&comment_json(instance, &parent, comment))),
    }
}

pub(crate) async fn add(
    ctx: &Ctx,
    parent: Parent,
    body: Option<String>,
    body_file: Option<String>,
) -> yt::Result<()> {
    let client = ctx.client().await?;
    let instance = client.instance();
    let text = match text::main_text(body, body_file.as_deref()).await? {
        Some(text) => text,
        None if ctx.ui.is_interactive() => {
            yt::api::parent::exists(client, &parent).await?;
            yt::editor::edit(&ctx.env, &ctx.dirs.state, "").await?
        }
        None => {
            return Err(YtError::InputRequired {
                what: "the comment text".into(),
                flag: "-b or --body-file -".into(),
            }
            .into());
        }
    };
    if text.trim().is_empty() {
        return Err(YtError::Usage {
            message: "the comment is empty".into(),
        }
        .into());
    }
    let comment = yt::api::comments::add(client, &parent, text.trim_end()).await?;
    match ctx.output.format() {
        Format::Table => ctx
            .output
            .write(yt::api::comments::url(instance, &parent, &comment.id).as_str()),
        Format::Json => ctx.output.json(&comment_json(instance, &parent, &comment)),
        Format::Ndjson => ctx
            .output
            .ndjson(&comment_json(instance, &parent, &comment)),
    }
}

pub(crate) async fn edit(
    ctx: &Ctx,
    parent: Parent,
    comment: String,
    body: Option<String>,
    body_file: Option<String>,
) -> yt::Result<()> {
    let given = text::main_text(body, body_file.as_deref()).await?;
    let client = ctx.client().await?;
    let instance = client.instance();
    let id = parent.id().to_string();
    let text = match given {
        Some(text) => text,
        None if ctx.ui.is_interactive() => {
            let current = yt::api::comments::get(client, &parent, &comment).await?;
            let current = current.text.unwrap_or_default();
            let edited = yt::editor::edit(&ctx.env, &ctx.dirs.state, &current).await?;
            if edited.trim_end() == current.trim_end() {
                ctx.note("No changes");
                return Ok(());
            }
            edited
        }
        None => {
            return Err(YtError::InputRequired {
                what: "the new text".into(),
                flag: "-b or --body-file -".into(),
            }
            .into());
        }
    };
    if text.trim().is_empty() {
        return Err(YtError::Usage {
            message: "the comment cannot be empty; delete it instead".into(),
        }
        .into());
    }
    let updated = yt::api::comments::edit(client, &parent, &comment, text.trim_end()).await?;
    match ctx.output.format() {
        Format::Table => {
            ctx.note(&format!("Updated comment {} on {id}", updated.id));
            Ok(())
        }
        Format::Json => ctx.output.json(&comment_json(instance, &parent, &updated)),
        Format::Ndjson => ctx
            .output
            .ndjson(&comment_json(instance, &parent, &updated)),
    }
}

pub(crate) async fn delete(ctx: &Ctx, parent: Parent, comment: String) -> yt::Result<()> {
    let id = parent.id().to_string();
    yt::api::comments::check_id(&comment, &parent)?;
    ctx.ui
        .confirm_destructive(&format!("delete comment {comment} on {id}"))
        .await?;
    let client = ctx.client().await?;
    yt::api::comments::delete(client, &parent, &comment).await?;
    match ctx.output.format() {
        Format::Table => {
            ctx.note(&format!("Deleted comment {comment} on {id}"));
            Ok(())
        }
        Format::Json => ctx.output.json(&[&comment]),
        Format::Ndjson => ctx.output.ndjson(&comment),
    }
}
