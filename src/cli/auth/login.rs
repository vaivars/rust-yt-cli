use std::io::Read as _;

use miette::IntoDiagnostic as _;
use secrecy::SecretString;
use usage_rs::{Args, RunAsyncWith, ValueEnum};
use yt::YtError;
use yt::auth::login::{LoginEnv, LoginOutcome, LoginRequest, Method};
use yt::http::HttpConfig;

use crate::cli::Ctx;

#[derive(Clone, Copy, Debug, ValueEnum)]
pub(crate) enum MethodArg {
    /// Approve access in the browser
    Browser,
    /// Paste a permanent token
    Token,
}

/// Sign in to a YouTrack instance
#[derive(Args)]
#[usage(effect = "write")]
pub(crate) struct Login {
    /// YouTrack URL, e.g. https://acme.youtrack.cloud
    #[usage(arg, name = "URL")]
    url: Option<String>,
    /// How to sign in
    #[usage(long, value_enum)]
    method: Option<MethodArg>,
    /// OAuth client ID from your YouTrack administrator [env: YT_OAUTH_CLIENT_ID]
    #[usage(long, value_name = "ID")]
    client_id: Option<String>,
    /// Read a permanent token from stdin
    #[usage(long)]
    token_stdin: bool,
    /// Do not open a browser; finish by pasting the address instead
    #[usage(long)]
    no_browser: bool,
    /// Also allow `yt auth logout` to revoke this sign-in on the server (shown to you as "YouTrack Administration" access)
    #[usage(long)]
    revocable: bool,
}

impl RunAsyncWith<&Ctx> for Login {
    type Output = yt::Result<()>;

    async fn run_async_with(self, ctx: &Ctx) -> Self::Output {
        if self.token_stdin && self.url.is_none() {
            return Err(YtError::InputRequired {
                what: "the YouTrack URL".into(),
                flag: "<URL>".into(),
            }
            .into());
        }
        let token = if self.token_stdin {
            Some(read_token().await?)
        } else {
            None
        };
        let request = LoginRequest {
            url: self.url,
            method: self.method.map(|method| match method {
                MethodArg::Browser => Method::Browser,
                MethodArg::Token => Method::Token,
            }),
            client_id: self
                .client_id
                .or_else(|| ctx.env.get("YT_OAUTH_CLIENT_ID").map(str::to_string)),
            profile: ctx.profile_flag.clone(),
            token,
            paste: self.no_browser,
            revocable: self.revocable,
        };
        let outcome = sign_in(ctx, request).await?;
        ctx.note(&format!(
            "Signed in to {} ({}) as {}",
            outcome.profile, outcome.url, outcome.login
        ));
        Ok(())
    }
}

pub(crate) async fn sign_in(ctx: &Ctx, request: LoginRequest) -> yt::Result<LoginOutcome> {
    yt::credentials::init_os_store()?;
    let config = ctx.global_config()?;
    let ca_file = request
        .profile
        .as_deref()
        .and_then(|name| config.profiles.get(name))
        .and_then(|profile| profile.ca_file.clone());
    let http = HttpConfig::from_settings(&ctx.settings()?.settings, ca_file)?;
    let store = ctx.config_store();
    let open = |url: &str| yt::browser::open(&ctx.env, url);
    let login_env = LoginEnv {
        env: &ctx.env,
        ui: &ctx.ui,
        store: &store,
        http: &http,
        open_browser: &open,
    };
    yt::auth::login::login(&login_env, request).await
}

async fn read_token() -> yt::Result<SecretString> {
    let text = tokio::task::spawn_blocking(|| -> std::io::Result<String> {
        let mut text = String::new();
        std::io::stdin().read_to_string(&mut text)?;
        Ok(text)
    })
    .await
    .into_diagnostic()?
    .into_diagnostic()?;
    let token = text.trim();
    if token.is_empty() {
        return Err(YtError::Usage {
            message: "no token on stdin".into(),
        }
        .into());
    }
    Ok(SecretString::from(token.to_string()))
}
