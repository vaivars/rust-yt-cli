use std::collections::{HashMap, HashSet};

use futures::TryStreamExt as _;
use usage_rs::{Args, RunAsyncWith};
use yt::YtError;
use yt::api::articles::ArticleRow;
use yt::output::{Format, Table};

use super::article_row_json;
use crate::cli::Ctx;

/// List articles matching a YouTrack query
#[derive(Args)]
#[usage(effect = "read")]
pub(crate) struct List {
    /// YouTrack search query
    #[usage(arg, name = "QUERY")]
    words: Vec<String>,
    /// YouTrack search query, instead of QUERY
    #[usage(long, value_name = "QUERY")]
    query: Option<String>,
    /// Only articles of this project
    #[usage(short, long, value_name = "PROJECT")]
    project: Option<String>,
    /// How many articles to show
    #[usage(short = 'n', long, default = "50")]
    limit: usize,
    /// Show every matching article
    #[usage(long, conflicts("--limit"))]
    all: bool,
    /// Show every matching article as a tree under its parent
    #[usage(long, conflicts("--limit", "--all"))]
    tree: bool,
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
        let instance = client.instance();
        let project =
            yt::config::profile::project(self.project.as_deref(), ctx.repo()?, &ctx.profile()?);
        let query = yt::api::issues::build_query(user_query.as_deref(), project.as_deref(), false)?;
        let every = self.all || self.tree;
        let page_size = if every { 100 } else { self.limit.min(100) };
        let mut rows: Vec<ArticleRow> = Vec::new();
        let mut stream = std::pin::pin!(yt::api::articles::search(client, &query, page_size));
        while let Some(row) = stream.try_next().await? {
            rows.push(row);
            if !every && rows.len() >= self.limit {
                break;
            }
        }
        if rows.is_empty() {
            ctx.note("No articles found.");
        }
        match ctx.output.format() {
            Format::Json => ctx.output.json(
                &rows
                    .iter()
                    .map(|row| article_row_json(instance, row))
                    .collect::<Vec<_>>(),
            ),
            Format::Ndjson => rows
                .iter()
                .try_for_each(|row| ctx.output.ndjson(&article_row_json(instance, row))),
            Format::Table if self.tree => {
                let mut table = Table::new(["ID", "UPDATED", "TITLE"]);
                for (depth, row) in tree(&rows) {
                    table.row([
                        format!("{}{}", "  ".repeat(depth), row.id_readable),
                        yt::time::local(row.updated),
                        row.summary.clone().unwrap_or_default(),
                    ]);
                }
                ctx.output.table(&table)
            }
            Format::Table => {
                let mut table = Table::new(["ID", "PARENT", "UPDATED", "TITLE"]);
                for row in &rows {
                    table.row([
                        row.id_readable.clone(),
                        row.parent_article
                            .as_ref()
                            .map(|parent| parent.id_readable.clone())
                            .unwrap_or_default(),
                        yt::time::local(row.updated),
                        row.summary.clone().unwrap_or_default(),
                    ]);
                }
                ctx.output.table(&table)
            }
        }
    }
}

/// Rows with their depth, each under its parent in YouTrack's order; an article whose parent is not
/// among the rows starts its own branch.
pub(crate) fn tree(rows: &[ArticleRow]) -> Vec<(usize, &ArticleRow)> {
    let ids: HashSet<&str> = rows.iter().map(|row| row.id_readable.as_str()).collect();
    let mut children: HashMap<&str, Vec<&ArticleRow>> = HashMap::new();
    let mut roots = Vec::new();
    for row in rows {
        match parent_in(row, &ids) {
            Some(parent) => children.entry(parent).or_default().push(row),
            None => roots.push(row),
        }
    }
    let mut ordered = Vec::new();
    let mut placed: HashSet<&str> = HashSet::new();
    let mut stack: Vec<(usize, &ArticleRow)> =
        roots.into_iter().rev().map(|row| (0, row)).collect();
    while let Some((depth, row)) = stack.pop() {
        if !placed.insert(row.id_readable.as_str()) {
            continue;
        }
        ordered.push((depth, row));
        if let Some(below) = children.get(row.id_readable.as_str()) {
            stack.extend(below.iter().rev().map(|child| (depth + 1, *child)));
        }
    }
    // Articles in a parent cycle have no root; they still appear, at the top level.
    for row in rows {
        if placed.insert(row.id_readable.as_str()) {
            ordered.push((0, row));
        }
    }
    ordered
}

fn parent_in<'a>(row: &'a ArticleRow, ids: &HashSet<&str>) -> Option<&'a str> {
    row.parent_article
        .as_ref()
        .map(|parent| parent.id_readable.as_str())
        .filter(|parent| *parent != row.id_readable && ids.contains(parent))
}

#[cfg(test)]
mod tests {
    use yt::api::articles::{ArticleRef, ArticleRow};

    use super::tree;

    fn row(id: &str, parent: Option<&str>) -> ArticleRow {
        ArticleRow {
            id_readable: id.into(),
            summary: Some(id.into()),
            project: None,
            parent_article: parent.map(|parent| ArticleRef {
                id_readable: parent.into(),
            }),
            child_articles: vec![],
            updated: 0,
        }
    }

    #[test]
    fn the_tree_nests_children_in_youtracks_order() {
        let rows = [
            row("KB-A-1", None),
            row("KB-A-2", Some("KB-A-1")),
            row("KB-A-3", Some("KB-A-2")),
            row("KB-A-4", Some("KB-A-9")),
            row("KB-A-5", Some("KB-A-1")),
        ];
        let order: Vec<(usize, &str)> = tree(&rows)
            .into_iter()
            .map(|(depth, row)| (depth, row.id_readable.as_str()))
            .collect();
        assert_eq!(
            order,
            [
                (0, "KB-A-1"),
                (1, "KB-A-2"),
                (2, "KB-A-3"),
                (1, "KB-A-5"),
                (0, "KB-A-4")
            ]
        );
    }

    #[test]
    fn a_deep_chain_nests_every_level() {
        let rows: Vec<ArticleRow> = (0..3000)
            .map(|n| {
                let parent = (n > 0).then(|| format!("KB-A-{}", n - 1));
                row(&format!("KB-A-{n}"), parent.as_deref())
            })
            .rev()
            .collect();
        let order = tree(&rows);
        assert_eq!(order.len(), 3000);
        assert!(
            order
                .iter()
                .enumerate()
                .all(|(index, (depth, _))| *depth == index)
        );
    }

    #[test]
    fn a_parent_cycle_still_lists_every_article() {
        let rows = [row("KB-A-1", Some("KB-A-2")), row("KB-A-2", Some("KB-A-1"))];
        assert_eq!(tree(&rows).len(), 2);
    }
}
