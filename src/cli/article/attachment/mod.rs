mod delete;
mod download;
mod list;
mod upload;

use usage_rs::{Args, Subcommands};

/// Work with article attachments
#[derive(Args)]
#[usage(run_async_with)]
pub(crate) struct AttachmentGroup {
    #[usage(subcommand)]
    command: AttachmentCommand,
}

#[derive(Subcommands)]
#[usage(run_async_with)]
pub(crate) enum AttachmentCommand {
    /// List an article's attachments
    #[usage(visible_alias = "ls")]
    List(list::List),
    /// Upload files to an article
    Upload(upload::Upload),
    /// Download an attachment
    Download(download::Download),
    /// Delete an attachment
    Delete(delete::Delete),
}
