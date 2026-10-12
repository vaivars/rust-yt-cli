use usage_rs::{Args, RunAsyncWith};
use yt::YtError;
use yt::output::Format;

use super::article_json;
use crate::cli::Ctx;
use crate::cli::issue::text;

/// Change an article
#[derive(Args)]
#[usage(effect = "write")]
pub(crate) struct Edit {
    /// Article ID or URL
    #[usage(arg, name = "ID")]
    id: String,
    /// New title
    #[usage(short, long, value_name = "TEXT")]
    summary: Option<String>,
    /// New content (Markdown)
    #[usage(short, long, value_name = "TEXT")]
    body: Option<String>,
    /// Read the content from a file, or `-` for stdin
    #[usage(long, value_name = "PATH")]
    body_file: Option<String>,
    /// New parent article ID or URL; empty moves the article to the top level
    #[usage(long, value_name = "ID")]
    parent: Option<String>,
}

impl RunAsyncWith<&Ctx> for Edit {
    type Output = yt::Result<()>;

    async fn run_async_with(self, ctx: &Ctx) -> Self::Output {
        let title = self.summary.as_deref().map(str::trim);
        if title == Some("") {
            return Err(blank_title());
        }
        let instance = ctx.instance()?;
        let id = instance.article_id(&self.id)?;
        let parent: Option<Option<String>> = match self.parent.as_deref().map(str::trim) {
            None => None,
            Some("") => Some(None),
            Some(raw) => Some(Some(instance.article_id(raw)?)),
        };
        let content = text::main_text(self.body, self.body_file.as_deref()).await?;
        let use_editor = title.is_none() && content.is_none() && parent.is_none();
        if use_editor && !ctx.ui.is_interactive() {
            return Err(YtError::Usage {
                message: "nothing to change: pass -s, -b, --body-file or --parent".into(),
            }
            .into());
        }
        let client = ctx.client().await?;
        let body = if use_editor {
            let article = yt::api::articles::get(client, &id).await?;
            let old_title = article.summary.unwrap_or_default();
            let old_content = article.content.unwrap_or_default();
            let buffer = yt::editor::edit(
                &ctx.env,
                &ctx.dirs.state,
                &yt::editor::join_issue(&old_title, &old_content),
            )
            .await?;
            let (new_title, new_content) = buffer_changes(&old_title, &old_content, &buffer);
            if new_title.is_none() && new_content.is_none() {
                ctx.note("No changes");
                return Ok(());
            }
            if new_title.as_deref() == Some("") {
                return Err(blank_title());
            }
            yt::api::articles::update_body(new_title.as_deref(), new_content.as_deref(), None)
        } else {
            let content = content.map(|content| content.trim_end().to_string());
            yt::api::articles::update_body(
                title,
                content.as_deref(),
                parent.as_ref().map(|parent| parent.as_deref()),
            )
        };
        yt::api::articles::update(client, &id, &body).await?;
        match ctx.output.format() {
            Format::Table => {
                ctx.note(&format!("Updated {id}"));
                Ok(())
            }
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

/// The title and content an edited buffer changes; `None` for a part left as it was.
fn buffer_changes(
    old_title: &str,
    old_content: &str,
    buffer: &str,
) -> (Option<String>, Option<String>) {
    let (title, content) = yt::editor::split_issue(buffer);
    // The old text goes through the same split, so a buffer saved unchanged compares equal.
    let (old_title, old_content) =
        yt::editor::split_issue(&yt::editor::join_issue(old_title, old_content));
    (
        (title != old_title).then_some(title),
        (content != old_content).then_some(content),
    )
}

fn blank_title() -> miette::Report {
    YtError::Usage {
        message: "the title cannot be empty".into(),
    }
    .into()
}

#[cfg(test)]
mod tests {
    use super::buffer_changes;

    #[test]
    fn an_unchanged_buffer_changes_nothing_even_with_indented_content() {
        let old = "    let x = 1;\nText";
        let buffer = yt::editor::join_issue("Guide", old);
        assert_eq!(buffer_changes("Guide", old, &buffer), (None, None));
    }

    #[test]
    fn only_the_changed_part_is_sent() {
        let buffer = yt::editor::join_issue("Guide", "New text");
        assert_eq!(
            buffer_changes("Guide", "Old text", &buffer),
            (None, Some("New text".to_string()))
        );
        let buffer = yt::editor::join_issue("Handbook", "Old text");
        assert_eq!(
            buffer_changes("Guide", "Old text", &buffer),
            (Some("Handbook".to_string()), None)
        );
    }
}
