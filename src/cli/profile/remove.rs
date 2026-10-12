use usage_rs::{Args, RunAsyncWith};
use yt::config::profile::{sharing_account, unknown_profile};
use yt::instance::InstanceUrl;

use crate::cli::Ctx;

/// Remove a profile and its stored credential
#[derive(Args)]
#[usage(effect = "destructive")]
pub(crate) struct Remove {
    /// Profile name
    #[usage(arg, name = "PROFILE")]
    name: String,
}

impl RunAsyncWith<&Ctx> for Remove {
    type Output = yt::Result<()>;

    async fn run_async_with(self, ctx: &Ctx) -> Self::Output {
        let config = ctx.global_config()?;
        let profile = config
            .profiles
            .get(&self.name)
            .ok_or_else(|| unknown_profile(&self.name, config))?;
        ctx.ui
            .confirm_destructive(&format!("remove profile {}", self.name))
            .await?;
        let sharing = sharing_account(config, &self.name);
        if !sharing.is_empty() {
            ctx.note(&format!(
                "Kept the stored sign-in: profile {} uses the same account.",
                sharing.join(", ")
            ));
        } else if let Some(login) = &profile.login {
            let account = InstanceUrl::parse(&profile.url)?.account(login);
            let deleted = match yt::credentials::init_os_store() {
                Ok(()) => yt::credentials::delete(&account).await,
                Err(error) => Err(error),
            };
            if let Err(error) = deleted {
                log::warn!("the stored credential for {account} was not deleted: {error}");
            }
        }
        ctx.config_store().remove_profile(&self.name)?;
        ctx.note(&format!("Removed profile {}", self.name));
        Ok(())
    }
}
