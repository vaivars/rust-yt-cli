use usage_rs::{Args, RunAsyncWith};
use yt::fields::plan::Operation;

use crate::cli::Ctx;
use crate::cli::issue::write::{print_dry_run, report_issue};

/// Link an issue to another
#[derive(Args)]
#[usage(effect = "write")]
pub(crate) struct AddLink {
    /// Issue ID or URL
    #[usage(arg, name = "ID")]
    id: String,
    /// Link direction words, e.g. `depends on` or `relates to` (see `yt issue link types`)
    #[usage(arg, name = "TYPE")]
    link_type: String,
    /// The other issue's ID or URL
    #[usage(arg, name = "TARGET")]
    target: String,
    /// Show the change without applying it
    #[usage(long)]
    dry_run: bool,
    /// Send the type as given
    #[usage(long)]
    no_validate: bool,
}

/// Remove a link between two issues
#[derive(Args)]
#[usage(effect = "write")]
pub(crate) struct RemoveLink {
    /// Issue ID or URL
    #[usage(arg, name = "ID")]
    id: String,
    /// Link direction words, e.g. `depends on`
    #[usage(arg, name = "TYPE")]
    link_type: String,
    /// The other issue's ID or URL
    #[usage(arg, name = "TARGET")]
    target: String,
    /// Show the change without applying it
    #[usage(long)]
    dry_run: bool,
    /// Send the type as given
    #[usage(long)]
    no_validate: bool,
}

impl RunAsyncWith<&Ctx> for AddLink {
    type Output = yt::Result<()>;

    async fn run_async_with(self, ctx: &Ctx) -> Self::Output {
        change_link(
            ctx,
            &self.id,
            &self.link_type,
            &self.target,
            "",
            self.dry_run,
            !self.no_validate,
        )
        .await
    }
}

impl RunAsyncWith<&Ctx> for RemoveLink {
    type Output = yt::Result<()>;

    async fn run_async_with(self, ctx: &Ctx) -> Self::Output {
        change_link(
            ctx,
            &self.id,
            &self.link_type,
            &self.target,
            "remove ",
            self.dry_run,
            !self.no_validate,
        )
        .await
    }
}

async fn change_link(
    ctx: &Ctx,
    id: &str,
    link_type: &str,
    target: &str,
    prefix: &str,
    dry_run: bool,
    validate: bool,
) -> yt::Result<()> {
    let client = ctx.client().await?;
    let instance = client.instance();
    let id = instance.issue_id(id)?;
    let target = instance.issue_id(target)?;
    let word = if validate {
        yt::api::links::resolve_word(&yt::api::links::types(client).await?, link_type)?
    } else {
        link_type.to_string()
    };
    let operations = vec![Operation::Command {
        issues: vec![id.clone()],
        command: format!("{prefix}{word} {target}"),
    }];
    if dry_run {
        return print_dry_run(ctx, client, &operations).await;
    }
    yt::fields::apply::execute(client, &operations).await?;
    report_issue(ctx, client, &id).await
}
