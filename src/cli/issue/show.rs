use usage_rs::{Args, RunAsyncWith};
use yt::api::issues::{IssueDetail, UserRef};
use yt::output::Format;

use super::json::issue_json;
use crate::cli::Ctx;

/// Show an issue
#[derive(Args)]
#[usage(effect = "read")]
pub(crate) struct Show {
    /// Issue ID (WEB-42) or issue URL
    #[usage(arg, name = "ID")]
    id: String,
    /// Open the issue in the browser instead of printing it
    #[usage(short, long)]
    web: bool,
}

impl RunAsyncWith<&Ctx> for Show {
    type Output = yt::Result<()>;

    async fn run_async_with(self, ctx: &Ctx) -> Self::Output {
        if self.web {
            let instance = ctx.instance()?;
            let url = instance.issue_url(&instance.issue_id(&self.id)?);
            if !yt::browser::open(&ctx.env, url.as_str()) {
                ctx.output.write(url.as_str())?;
            }
            return Ok(());
        }
        let client = ctx.client().await?;
        let id = client.instance().issue_id(&self.id)?;
        let issue = yt::api::issues::get(client, &id).await?;
        match ctx.output.format() {
            Format::Table => ctx.output.write(&render(&issue)),
            Format::Json => ctx.output.json(&issue_json(&issue)),
            Format::Ndjson => ctx.output.ndjson(&issue_json(&issue)),
        }
    }
}

fn render(issue: &IssueDetail) -> String {
    let mut rows: Vec<(String, String)> = Vec::new();
    let project = match &issue.project.name {
        Some(name) => format!("{} ({name})", issue.project.short_name),
        None => issue.project.short_name.clone(),
    };
    rows.push(("Project".into(), project));
    for field in &issue.custom_fields {
        let value = field.display();
        if !value.is_empty() {
            rows.push((field.name.clone(), value));
        }
    }
    if let Some(reporter) = &issue.reporter {
        rows.push(("Reporter".into(), user(reporter)));
    }
    rows.push(("Created".into(), yt::time::local(issue.created)));
    rows.push(("Updated".into(), yt::time::local(issue.updated)));
    if let Some(resolved) = issue.resolved {
        rows.push(("Resolved".into(), yt::time::local(resolved)));
    }
    if !issue.tags.is_empty() {
        rows.push((
            "Tags".into(),
            issue
                .tags
                .iter()
                .map(|tag| tag.name.as_str())
                .collect::<Vec<_>>()
                .join(", "),
        ));
    }
    let heading = format!(
        "{}  {}",
        issue.id_readable,
        issue.summary.as_deref().unwrap_or("")
    );
    crate::cli::details(&heading, rows, issue.description.as_deref())
}

fn user(user: &UserRef) -> String {
    match &user.full_name {
        Some(name) if name != &user.login => format!("{} ({name})", user.login),
        _ => user.login.clone(),
    }
}
