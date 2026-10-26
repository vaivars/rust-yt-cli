use std::io::Read as _;

use miette::IntoDiagnostic as _;
use reqwest::Method;
use usage_rs::{Args, RunAsyncWith, ValueEnum};
use yt::YtError;

use crate::cli::Ctx;

#[derive(Clone, Copy, Debug, ValueEnum)]
#[usage(ignore_case)]
pub(crate) enum HttpMethod {
    #[usage(name = "GET")]
    Get,
    #[usage(name = "POST")]
    Post,
    #[usage(name = "DELETE")]
    Delete,
}

/// Send a request to the YouTrack REST API and print its JSON response
#[derive(Args)]
#[usage(effect = "destructive")]
pub(crate) struct Api {
    /// Path under /api, e.g. `issues/WEB-1` or `admin/projects?$top=5`
    #[usage(arg, name = "PATH")]
    path: String,
    /// HTTP method
    #[usage(short = 'X', long, value_enum, default = "GET")]
    method: HttpMethod,
    /// JSON request body from a file, or `-` for stdin
    #[usage(long, value_name = "FILE")]
    input: Option<String>,
    /// `fields` selector to add to the query
    #[usage(long, value_name = "SELECTOR")]
    fields: Option<String>,
}

impl RunAsyncWith<&Ctx> for Api {
    type Output = yt::Result<()>;

    async fn run_async_with(self, ctx: &Ctx) -> Self::Output {
        if matches!(self.method, HttpMethod::Delete) {
            ctx.ui
                .confirm_destructive(&format!("send DELETE {}", self.path))
                .await?;
        }
        let body = match &self.input {
            Some(source) => Some(read_body(source).await?),
            None => None,
        };
        let mut path = self.path.clone();
        if let Some(fields) = &self.fields {
            let encoded: String = url::form_urlencoded::byte_serialize(fields.as_bytes()).collect();
            path.push(if path.contains('?') { '&' } else { '?' });
            path.push_str("fields=");
            path.push_str(&encoded);
        }
        let method = match self.method {
            HttpMethod::Get => Method::GET,
            HttpMethod::Post => Method::POST,
            HttpMethod::Delete => Method::DELETE,
        };
        let client = ctx.client().await?;
        match client.raw(method, &path, body.as_ref()).await? {
            Some(text) => ctx.output.raw_json(&text),
            None => Ok(()),
        }
    }
}

pub(crate) async fn read_body(source: &str) -> yt::Result<serde_json::Value> {
    let source = source.to_string();
    let text = tokio::task::spawn_blocking(move || -> std::io::Result<String> {
        if source == "-" {
            let mut text = String::new();
            std::io::stdin().read_to_string(&mut text)?;
            Ok(text)
        } else {
            std::fs::read_to_string(&source)
        }
    })
    .await
    .into_diagnostic()?
    .into_diagnostic()?;
    serde_json::from_str(&text).map_err(|error| {
        YtError::Usage {
            message: format!("the request body is not valid JSON: {error}"),
        }
        .into()
    })
}
