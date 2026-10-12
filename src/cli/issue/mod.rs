mod attachment;
mod command;
mod comment;
mod create;
mod delete;
mod edit;
pub(crate) mod ids;
mod json;
mod link;
mod list;
mod show;
mod tag;
pub(crate) mod text;
pub(crate) mod write;

use std::collections::BTreeMap;

use usage_rs::{Args, Subcommands};
use yt::api::custom_fields::IssueCustomField;

/// Work with issues
#[derive(Args)]
#[usage(run_async_with)]
pub(crate) struct Issue {
    #[usage(subcommand)]
    command: IssueCommand,
}

#[derive(Subcommands)]
#[usage(run_async_with)]
pub(crate) enum IssueCommand {
    /// Create an issue
    Create(create::Create),
    /// Change issues
    Edit(edit::Edit),
    /// Apply a YouTrack command to issues
    Command(command::Apply),
    /// Delete issues
    Delete(delete::Delete),
    /// Work with issue comments
    Comment(comment::CommentGroup),
    /// Add or remove issue tags
    Tag(tag::TagGroup),
    /// Work with issue links
    Link(link::LinkGroup),
    /// Work with issue attachments
    Attachment(attachment::AttachmentGroup),
    /// List issues matching a YouTrack query
    #[usage(visible_alias = "ls")]
    List(list::List),
    /// Show an issue
    Show(show::Show),
}

/// Custom fields as `name → value` for JSON output.
pub(crate) fn fields_json(fields: &[IssueCustomField]) -> BTreeMap<&str, serde_json::Value> {
    fields
        .iter()
        .map(|field| (field.name.as_str(), field.json()))
        .collect()
}
