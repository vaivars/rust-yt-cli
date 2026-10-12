use usage_rs::{Args, RunAsyncWith};
use yt::output::Format;

use crate::cli::Ctx;

/// List every setting with its value
#[derive(Args)]
#[usage(effect = "read")]
pub(crate) struct List;

impl RunAsyncWith<&Ctx> for List {
    type Output = yt::Result<()>;

    async fn run_async_with(self, ctx: &Ctx) -> Self::Output {
        let settings = ctx.settings()?;
        match ctx.output.format() {
            Format::Table => {
                for line in settings.list() {
                    ctx.output.write(&line)?;
                }
                Ok(())
            }
            Format::Json => ctx.output.json(&settings.values()),
            Format::Ndjson => ctx.output.ndjson(&settings.values()),
        }
    }
}
