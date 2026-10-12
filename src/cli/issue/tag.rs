use serde_json::json;
use usage_rs::{Args, RunAsyncWith, Subcommands};
use yt::fields::apply::{self, Planned};
use yt::fields::plan::Operation;

use super::write::{print_planned, report_issue};
use crate::cli::Ctx;

/// Add or remove issue tags
#[derive(Args)]
#[usage(run_async_with)]
pub(crate) struct TagGroup {
    #[usage(subcommand)]
    command: TagCommand,
}

#[derive(Subcommands)]
#[usage(run_async_with)]
pub(crate) enum TagCommand {
    /// Tag an issue
    Add(AddTags),
    /// Remove tags from an issue
    Remove(RemoveTags),
}

/// Tag an issue
#[derive(Args)]
#[usage(effect = "write")]
pub(crate) struct AddTags {
    /// Issue ID or URL
    #[usage(arg, name = "ID")]
    id: String,
    /// Tag names
    #[usage(arg, name = "TAG", required)]
    tags: Vec<String>,
    /// Show the changes without applying them
    #[usage(long)]
    dry_run: bool,
    /// Create tags that don't exist yet
    #[usage(long)]
    no_validate: bool,
}

/// Remove tags from an issue
#[derive(Args)]
#[usage(effect = "write")]
pub(crate) struct RemoveTags {
    /// Issue ID or URL
    #[usage(arg, name = "ID")]
    id: String,
    /// Tag names
    #[usage(arg, name = "TAG", required)]
    tags: Vec<String>,
    /// Show the changes without applying them
    #[usage(long)]
    dry_run: bool,
    /// Send tag names as given
    #[usage(long)]
    no_validate: bool,
}

impl RunAsyncWith<&Ctx> for AddTags {
    type Output = yt::Result<()>;

    async fn run_async_with(self, ctx: &Ctx) -> Self::Output {
        change_tags(
            ctx,
            &self.id,
            &self.tags,
            "tag",
            "pass --no-validate to create it",
            self.dry_run,
            if self.no_validate {
                Unknown::Create
            } else {
                Unknown::Refuse
            },
        )
        .await
    }
}

impl RunAsyncWith<&Ctx> for RemoveTags {
    type Output = yt::Result<()>;

    async fn run_async_with(self, ctx: &Ctx) -> Self::Output {
        change_tags(
            ctx,
            &self.id,
            &self.tags,
            "untag",
            "pass --no-validate to send it as given",
            self.dry_run,
            if self.no_validate {
                Unknown::Send
            } else {
                Unknown::Refuse
            },
        )
        .await
    }
}

/// What to do with a tag name that matches none of the user's tags.
#[derive(Clone, Copy, PartialEq)]
enum Unknown {
    Refuse,
    Create,
    Send,
}

async fn change_tags(
    ctx: &Ctx,
    id: &str,
    tags: &[String],
    verb: &str,
    hint: &str,
    dry_run: bool,
    unknown: Unknown,
) -> yt::Result<()> {
    let client = ctx.client().await?;
    let id = client.instance().issue_id(id)?;
    let mut names = Vec::new();
    let mut new = Vec::new();
    if unknown == Unknown::Send {
        names = tags.to_vec();
    } else {
        let visible = yt::api::tags::visible(client).await?;
        for tag in tags {
            match yt::api::tags::resolve(&visible, tag, hint) {
                Ok(name) => names.push(name),
                // A command can't create a tag whose name has spaces, so new tags are created first.
                Err(_) if unknown == Unknown::Create => {
                    names.push(tag.clone());
                    new.push(tag.clone());
                }
                Err(error) => return Err(error),
            }
        }
    }
    let command = |name: &str| Operation::Command {
        issues: vec![id.clone()],
        command: format!("{verb} {name}"),
    };
    if dry_run {
        let mut planned = Vec::new();
        for name in &names {
            let operation = command(name);
            // YouTrack can't check a command for a tag that doesn't exist yet.
            if new.contains(name) {
                planned.push(Planned {
                    operation,
                    problems: Vec::new(),
                });
            } else {
                planned.extend(apply::check(client, &[operation]).await?);
            }
        }
        let steps: Vec<(String, serde_json::Value)> = new
            .iter()
            .map(|name| {
                (
                    format!("create tag \"{name}\""),
                    json!({"kind": "create tag", "name": name}),
                )
            })
            .collect();
        return print_planned(ctx, &steps, &planned);
    }
    for name in &new {
        yt::api::tags::create(client, name).await?;
    }
    let operations: Vec<Operation> = names.iter().map(|name| command(name)).collect();
    apply::execute(client, &operations).await?;
    report_issue(ctx, client, &id).await
}
