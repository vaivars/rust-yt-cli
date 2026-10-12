use yt::YtError;

use super::text::stdin_text;
use crate::cli::Ctx;

/// Issue IDs from arguments; `-` reads more from stdin, one per line. URLs become readable IDs; duplicates are dropped.
pub(crate) async fn issue_ids(ctx: &Ctx, args: &[String]) -> yt::Result<Vec<String>> {
    if args.iter().filter(|arg| arg.as_str() == "-").count() > 1 {
        return Err(YtError::Usage {
            message: "only one '-' can read IDs from stdin".into(),
        }
        .into());
    }
    let instance = ctx.instance()?;
    let mut ids: Vec<String> = Vec::new();
    for arg in args {
        let words: Vec<String> = if arg == "-" {
            stdin_text()
                .await?
                // `yt issue list` output pipes in: the ID is the first word of each line.
                .lines()
                .filter_map(|line| line.split_whitespace().next())
                .map(str::to_string)
                .collect()
        } else {
            vec![arg.clone()]
        };
        for word in words {
            let id = instance.issue_id(&word)?;
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
    }
    if ids.is_empty() {
        return Err(YtError::Usage {
            message: "no issue IDs were given".into(),
        }
        .into());
    }
    Ok(ids)
}
