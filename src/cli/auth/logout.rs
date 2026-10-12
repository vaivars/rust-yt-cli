use usage_rs::{Args, RunAsyncWith};
use yt::YtError;
use yt::config::file::Profile;
use yt::config::profile::{ProfileChoice, sharing_account};
use yt::credentials::{OauthCredential, StoredCredential};
use yt::instance::InstanceUrl;

use crate::cli::Ctx;
use crate::cli::auth::server_sign_in;

/// Sign out: delete the stored credential
#[derive(Args)]
#[usage(effect = "destructive")]
pub(crate) struct Logout;

impl RunAsyncWith<&Ctx> for Logout {
    type Output = yt::Result<()>;

    async fn run_async_with(self, ctx: &Ctx) -> Self::Output {
        let ProfileChoice::Named { name, profile } = ctx.profile()? else {
            return Err(YtError::Usage {
                message: "the credentials come from YOUTRACK_URL and YOUTRACK_TOKEN; unset them to sign out".into(),
            }
            .into());
        };
        let Some(login) = profile.login.clone() else {
            ctx.note(&format!("Not signed in to {name}."));
            return Ok(());
        };
        ctx.ui
            .confirm_destructive(&format!("sign out of {name}"))
            .await?;
        let url = InstanceUrl::parse(&profile.url)?;
        let account = url.account(&login);
        yt::credentials::init_os_store()?;
        let mut revoked = false;
        if profile.revocable
            && let Some(StoredCredential::Oauth(credential)) =
                yt::credentials::load(&account).await?
        {
            revoked = revoke(ctx, &name, &profile, &account, credential).await;
        }
        yt::credentials::delete(&account).await?;
        ctx.note(&format!("Signed out of {name}."));
        let sharing = sharing_account(ctx.global_config()?, &name);
        if !sharing.is_empty() {
            ctx.note(&format!(
                "Profile {} used the same sign-in and is signed out too.",
                sharing.join(", ")
            ));
        }
        if !revoked {
            ctx.note(&format!(
                "The server still lists this sign-in until you delete it under Account Security: {}",
                url.join("users/me?tab=account-security")
            ));
        }
        Ok(())
    }
}

/// Deletes this sign-in's refresh token on the server; best effort.
async fn revoke(
    ctx: &Ctx,
    name: &str,
    profile: &Profile,
    account: &str,
    credential: OauthCredential,
) -> bool {
    let result: yt::Result<bool> = async {
        let Some(sign_in) = server_sign_in(ctx, name, profile, account, credential).await? else {
            return Ok(false);
        };
        yt::auth::oauth::delete_refresh_token(
            &sign_in.http,
            &sign_in.hub,
            &sign_in.access,
            &sign_in.token.id,
        )
        .await?;
        Ok(true)
    }
    .await;
    result.unwrap_or_else(|error| {
        log::warn!("the sign-in was not revoked on the server: {error}");
        false
    })
}
