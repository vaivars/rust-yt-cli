use serde_json::json;
use usage_rs::{Args, RunAsyncWith};
use yt::output::Format;

use crate::cli::Ctx;

/// Show one setting's value and where it came from
#[derive(Args)]
#[usage(effect = "read")]
pub(crate) struct Get {
    /// Setting name, e.g. http_timeout
    #[usage(arg, name = "KEY")]
    key: String,
}

impl RunAsyncWith<&Ctx> for Get {
    type Output = yt::Result<()>;

    async fn run_async_with(self, ctx: &Ctx) -> Self::Output {
        let settings = ctx.settings()?;
        let explanation = settings.explain(&self.key)?;
        let value = settings.values().get(&self.key).cloned();
        match ctx.output.format() {
            Format::Table => ctx.output.write(explanation.trim_end()),
            Format::Json => ctx.output.json(&json!({ "key": self.key, "value": value })),
            Format::Ndjson => ctx
                .output
                .ndjson(&json!({ "key": self.key, "value": value })),
        }
    }
}
