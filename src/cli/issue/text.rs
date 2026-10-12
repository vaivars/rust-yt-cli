use std::io::Read as _;

use miette::IntoDiagnostic as _;
use yt::YtError;

/// All of stdin as text.
pub(crate) async fn stdin_text() -> yt::Result<String> {
    tokio::task::spawn_blocking(|| -> std::io::Result<String> {
        let mut text = String::new();
        std::io::stdin().read_to_string(&mut text)?;
        Ok(text)
    })
    .await
    .into_diagnostic()?
    .into_diagnostic()
}

/// The main text from `-b` or `--body-file` (`-` reads stdin). Giving both is a usage error.
pub(crate) async fn main_text(
    body: Option<String>,
    body_file: Option<&str>,
) -> yt::Result<Option<String>> {
    match (body, body_file) {
        (Some(_), Some(_)) => Err(YtError::Usage {
            message: "pass either -b or --body-file, not both".into(),
        }
        .into()),
        (Some(text), None) => Ok(Some(text)),
        (None, Some("-")) => stdin_text().await.map(Some),
        (None, Some(path)) => {
            let path = path.to_string();
            let text = tokio::task::spawn_blocking(move || {
                std::fs::read_to_string(&path).map_err(|error| -> miette::Report {
                    YtError::Usage {
                        message: format!("cannot read {path}: {error}"),
                    }
                    .into()
                })
            })
            .await
            .into_diagnostic()??;
            Ok(Some(text))
        }
        (None, None) => Ok(None),
    }
}
