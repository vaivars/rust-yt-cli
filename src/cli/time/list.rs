use usage_rs::{Args, RunAsyncWith};
use yt::YtError;
use yt::api::work_items::{ListFilter, WorkItem};
use yt::output::{Format, Table};

use super::{day, work_item_json};
use crate::cli::Ctx;

/// List work items, newest first
#[derive(Args)]
#[usage(effect = "read")]
pub(crate) struct List {
    /// YouTrack issue query, e.g. `WEB-42` or `#Unresolved`
    #[usage(arg, name = "QUERY")]
    words: Vec<String>,
    /// YouTrack issue query, instead of QUERY
    #[usage(long, value_name = "QUERY")]
    query: Option<String>,
    /// Only work items on issues of this project
    #[usage(short, long, value_name = "PROJECT")]
    project: Option<String>,
    /// First day, YYYY-MM-DD
    #[usage(long, value_name = "DATE")]
    from: Option<String>,
    /// Last day, YYYY-MM-DD
    #[usage(long, value_name = "DATE")]
    to: Option<String>,
    /// Whose work items: a login, `me` or `all`
    #[usage(long, value_name = "LOGIN", default = "me")]
    author: String,
    /// How many work items to show
    #[usage(short = 'n', long, default = "50")]
    limit: usize,
    /// Show every matching work item
    #[usage(long, conflicts("--limit"))]
    all: bool,
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
        let start = self
            .from
            .as_deref()
            .map(|value| day("--from", value).map(yt::time::date))
            .transpose()?;
        let end = self
            .to
            .as_deref()
            .map(|value| day("--to", value).map(yt::time::date))
            .transpose()?;
        let client = ctx.client().await?;
        let project =
            yt::config::profile::project(self.project.as_deref(), ctx.repo()?, &ctx.profile()?);
        let query = yt::api::issues::build_query(user_query.as_deref(), project.as_deref(), false)?;
        let author = match self.author.as_str() {
            "all" => None,
            "me" => Some(yt::api::users::me(client).await?.login),
            login => Some(login.to_string()),
        };
        let filter = ListFilter {
            query: Some(query).filter(|query| !query.is_empty()),
            start,
            end,
            author,
        };
        let mut items = yt::api::work_items::list(client, &filter).await?;
        if !self.all && items.len() > self.limit {
            let total: i64 = items.iter().map(|item| item.minutes()).sum();
            ctx.note(&format!(
                "Showing {} of {} work items ({} in all); --all lists every one.",
                self.limit,
                items.len(),
                yt::time::hours_minutes(total)
            ));
            items.truncate(self.limit);
        }
        if items.is_empty() {
            ctx.note("No work items found.");
        }
        match ctx.output.format() {
            Format::Json => ctx
                .output
                .json(&items.iter().map(work_item_json).collect::<Vec<_>>()),
            Format::Ndjson => items
                .iter()
                .try_for_each(|item| ctx.output.ndjson(&work_item_json(item))),
            Format::Table => {
                let mut table = Table::new([
                    "ITEM", "DATE", "ISSUE", "DURATION", "TYPE", "AUTHOR", "TEXT",
                ]);
                for item in &items {
                    table.row([
                        item.id.clone(),
                        yt::time::date(item.date),
                        item.issue
                            .as_ref()
                            .map(|issue| issue.id_readable.clone())
                            .unwrap_or_default(),
                        item.presentation(),
                        item.work_type
                            .as_ref()
                            .map(|work_type| work_type.name.clone())
                            .unwrap_or_default(),
                        item.author
                            .as_ref()
                            .map(|author| author.login.clone())
                            .unwrap_or_default(),
                        item.text
                            .as_deref()
                            .and_then(|text| text.lines().next())
                            .unwrap_or("")
                            .to_string(),
                    ]);
                }
                ctx.output.table(&table)?;
                if ctx.output.is_terminal() && !items.is_empty() {
                    ctx.output.write(&total(&items))?;
                }
                Ok(())
            }
        }
    }
}

/// The last line of a terminal table: the time of the rows shown.
fn total(items: &[WorkItem]) -> String {
    let minutes: i64 = items.iter().map(WorkItem::minutes).sum();
    format!("Total {}", yt::time::hours_minutes(minutes))
}

#[cfg(test)]
mod tests {
    use yt::api::work_items::WorkItem;

    use super::total;

    fn item(minutes: i64) -> WorkItem {
        serde_json::from_value(serde_json::json!({"id": "270-1", "duration": {"minutes": minutes, "presentation": ""}}))
            .unwrap()
    }

    #[test]
    fn the_total_adds_the_rows_shown() {
        assert_eq!(total(&[item(90), item(45)]), "Total 2h 15m");
    }
}
