use std::collections::{BTreeMap, HashSet};

use futures::StreamExt as _;
use serde::Serialize;
use usage_rs::{Args, RunAsyncWith};
use yt::YtError;
use yt::api::issues::{IssueRow, build_query, search};
use yt::output::{Format, Table};

use super::fields_json;
use crate::cli::Ctx;

/// List issues matching a YouTrack query
#[derive(Args)]
#[usage(effect = "read")]
pub(crate) struct List {
    /// YouTrack search query, e.g. `for: me #Unresolved`
    #[usage(arg, name = "QUERY")]
    words: Vec<String>,
    /// YouTrack search query, instead of QUERY
    #[usage(long, value_name = "QUERY")]
    query: Option<String>,
    /// Only issues of this project
    #[usage(short, long, value_name = "PROJECT", value_hint = usage_rs::ValueHint::Other)]
    project: Option<String>,
    /// How many issues to show
    #[usage(short = 'n', long, default = "50")]
    limit: usize,
    /// Show every matching issue
    #[usage(long, conflicts("--limit"))]
    all: bool,
}

#[derive(Serialize)]
struct IssueJson<'a> {
    id: &'a str,
    summary: Option<&'a str>,
    project: &'a str,
    created: String,
    updated: String,
    resolved: Option<String>,
    fields: BTreeMap<&'a str, serde_json::Value>,
}

impl<'a> From<&'a IssueRow> for IssueJson<'a> {
    fn from(issue: &'a IssueRow) -> Self {
        Self {
            id: &issue.id_readable,
            summary: issue.summary.as_deref(),
            project: &issue.project.short_name,
            created: yt::time::utc(issue.created),
            updated: yt::time::utc(issue.updated),
            resolved: issue.resolved.map(yt::time::utc),
            fields: fields_json(&issue.custom_fields),
        }
    }
}

impl RunAsyncWith<&Ctx> for List {
    type Output = yt::Result<()>;

    async fn run_async_with(self, ctx: &Ctx) -> Self::Output {
        crate::cli::check_limit(self.limit)?;
        let user_query = match (self.words.is_empty(), &self.query) {
            (false, Some(_)) => {
                return Err(YtError::Usage {
                    message: "give the query as words or with --query, not both".into(),
                }
                .into());
            }
            (false, None) => Some(self.words.join(" ")),
            (true, query) => query.clone(),
        };
        let client = ctx.client().await?;
        let project =
            yt::config::profile::project(self.project.as_deref(), ctx.repo()?, &ctx.profile()?);
        let query = build_query(user_query.as_deref(), project.as_deref(), self.all)?;
        let page_size = if self.all { 100 } else { self.limit.min(100) };
        let format = ctx.output.format();
        let mut issues = std::pin::pin!(search(client, &query, page_size));
        let mut seen = HashSet::new();
        let mut rows = Vec::new();
        while let Some(issue) = issues.next().await {
            let issue = issue?;
            if !seen.insert(issue.id_readable.clone()) {
                continue;
            }
            if format == Format::Ndjson {
                ctx.output.ndjson(&IssueJson::from(&issue))?;
                if ctx.output.is_closed() {
                    return Ok(());
                }
            } else {
                rows.push(issue);
            }
            if !self.all && seen.len() >= self.limit {
                break;
            }
        }
        if seen.is_empty() {
            ctx.note("No issues found.");
        }
        match format {
            Format::Ndjson => Ok(()),
            Format::Json => ctx
                .output
                .json(&rows.iter().map(IssueJson::from).collect::<Vec<_>>()),
            Format::Table => {
                let mut table = Table::new(["ID", "STATE", "SUMMARY", "UPDATED"]);
                for issue in &rows {
                    table.row([
                        issue.id_readable.clone(),
                        issue.state().unwrap_or_default(),
                        issue.summary.clone().unwrap_or_default(),
                        yt::time::local(issue.updated),
                    ]);
                }
                ctx.output.table(&table)
            }
        }
    }
}
