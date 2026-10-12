mod login;
mod logout;
mod status;

use secrecy::SecretString;
use url::Url;
use usage_rs::{Args, Subcommands};
use yt::api::TokenSource as _;
use yt::auth::oauth::{HubRefreshToken, issued_refresh_token};
use yt::auth::token_source::OAuthToken;
use yt::config::file::Profile;
use yt::credentials::OauthCredential;
use yt::http::HttpConfig;
use yt::instance::InstanceUrl;

use crate::cli::Ctx;

pub(crate) use login::sign_in;

/// Sign in and manage credentials
#[derive(Args)]
#[usage(run_async_with)]
pub(crate) struct Auth {
    #[usage(subcommand)]
    command: AuthCommand,
}

#[derive(Subcommands)]
#[usage(run_async_with)]
pub(crate) enum AuthCommand {
    /// Sign in to a YouTrack instance
    Login(login::Login),
    /// Show which profile is used and whether its sign-in works
    Status(status::Status),
    /// Sign out: delete the stored credential
    Logout(logout::Logout),
}

/// A revocable browser sign-in as Hub records it, with what is needed to act on it.
pub(crate) struct ServerSignIn {
    pub(crate) http: reqwest::Client,
    pub(crate) hub: Url,
    pub(crate) access: SecretString,
    pub(crate) token: HubRefreshToken,
}

/// Finds the profile's refresh token on the server; `None` when Hub no longer lists it.
pub(crate) async fn server_sign_in(
    ctx: &Ctx,
    name: &str,
    profile: &Profile,
    account: &str,
    credential: OauthCredential,
) -> yt::Result<Option<ServerSignIn>> {
    let config = HttpConfig::from_settings(&ctx.settings()?.settings, profile.ca_file.clone())?;
    let http = yt::http::client(&config)?;
    let url = InstanceUrl::parse(&profile.url)?;
    let (client_id, created) = (credential.client_id.clone(), credential.created_at);
    let source = OAuthToken::new(http.clone(), account, name, &ctx.dirs.state, credential);
    let access = source.bearer().await?;
    let found = issued_refresh_token(&http, &url, &access, &client_id, created).await?;
    Ok(found.map(|(hub, token)| ServerSignIn {
        http,
        hub,
        access,
        token,
    }))
}
