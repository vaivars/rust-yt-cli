use usage_rs::{Args, RunAsyncWith};
use yt::config::profile::unknown_profile;

use crate::cli::Ctx;

/// Make a profile the default
#[derive(Args)]
#[usage(effect = "write")]
pub(crate) struct UseProfile {
    /// Profile name
    #[usage(arg, name = "PROFILE")]
    name: String,
}

impl RunAsyncWith<&Ctx> for UseProfile {
    type Output = yt::Result<()>;

    async fn run_async_with(self, ctx: &Ctx) -> Self::Output {
        let config = ctx.global_config()?;
        if !config.profiles.contains_key(&self.name) {
            return Err(unknown_profile(&self.name, config));
        }
        ctx.config_store().set_default_profile(&self.name)?;
        ctx.note(&format!("Now using profile {}", self.name));
        Ok(())
    }
}
