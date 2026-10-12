use usage_rs::{Args, RunAsyncWith};
use yt::config::settings::setting_value;

use crate::cli::Ctx;

/// Change a setting in the global config file
#[derive(Args)]
#[usage(effect = "write")]
pub(crate) struct Set {
    /// Setting name, e.g. http_timeout
    #[usage(arg, name = "KEY")]
    key: String,
    /// New value
    #[usage(arg, name = "VALUE")]
    value: String,
}

impl RunAsyncWith<&Ctx> for Set {
    type Output = yt::Result<()>;

    async fn run_async_with(self, ctx: &Ctx) -> Self::Output {
        let setting = setting_value(&self.key, &self.value)?;
        ctx.config_store().set_setting(&self.key, setting)?;
        ctx.note(&format!(
            "Set {} = {} in {}",
            self.key,
            self.value,
            ctx.dirs.config_file().display()
        ));
        Ok(())
    }
}
