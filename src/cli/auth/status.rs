use serde::Serialize;
use usage_rs::{Args, RunAsyncWith};
use yt::config::file::AuthKind;
use yt::config::profile::ProfileChoice;
use yt::credentials::{OauthCredential, StoredCredential};
use yt::instance::InstanceUrl;
use yt::output::Format;

use crate::cli::Ctx;
use crate::cli::auth::server_sign_in;

/// Show which profile is used and whether its sign-in works
#[derive(Args)]
#[usage(effect = "read")]
pub(crate) struct Status {
    /// Do not contact the server
    #[usage(long)]
    offline: bool,
}

#[derive(Serialize)]
struct StatusJson {
    profile: String,
    url: String,
    login: Option<String>,
    method: &'static str,
    credential: String,
    #[serde(serialize_with = "utc")]
    access_expires: Option<i64>,
    #[serde(serialize_with = "utc")]
    sign_in_expires: Option<i64>,
    check: Option<String>,
}

fn utc<S: serde::Serializer>(millis: &Option<i64>, serializer: S) -> Result<S::Ok, S::Error> {
    match millis {
        Some(millis) => serializer.serialize_some(&yt::time::utc(*millis)),
        None => serializer.serialize_none(),
    }
}

impl RunAsyncWith<&Ctx> for Status {
    type Output = yt::Result<()>;

    async fn run_async_with(self, ctx: &Ctx) -> Self::Output {
        let choice = ctx.profile()?;
        let mut status = StatusJson {
            profile: choice.label().to_string(),
            url: choice.url()?.to_string(),
            login: None,
            method: "token",
            credential: "missing".into(),
            access_expires: None,
            sign_in_expires: None,
            check: None,
        };
        match &choice {
            ProfileChoice::Env { .. } => {
                status.credential = "YOUTRACK_URL and YOUTRACK_TOKEN".into()
            }
            ProfileChoice::Named { name, profile } => {
                status.login = profile.login.clone();
                status.method = match profile.auth {
                    AuthKind::Oauth => "browser",
                    AuthKind::Token => "token",
                };
                if ctx.env.get("YOUTRACK_TOKEN").is_some() {
                    status.credential = "YOUTRACK_TOKEN".into();
                } else if let Some(login) = &profile.login {
                    let account = InstanceUrl::parse(&profile.url)?.account(login);
                    let oauth;
                    (status.credential, oauth) = stored(&account).await;
                    if let Some(credential) = oauth {
                        status.access_expires = Some(credential.expires_at.timestamp_millis());
                        // Only a revocable sign-in may read its refresh token's expiry from Hub.
                        if profile.revocable && !self.offline {
                            status.sign_in_expires = match server_sign_in(
                                ctx, name, profile, &account, credential,
                            )
                            .await
                            {
                                Ok(found) => found.and_then(|sign_in| sign_in.token.expires),
                                Err(error) => {
                                    log::warn!(
                                        "could not read the sign-in from the server: {error}"
                                    );
                                    None
                                }
                            };
                        }
                    }
                }
            }
        }
        let failure = if self.offline {
            None
        } else {
            let check = match ctx.client().await {
                Ok(client) => yt::api::users::me(client).await.map(|me| me.login),
                Err(error) => Err(error),
            };
            match check {
                Ok(login) => {
                    status.check = Some(format!("ok (signed in as {login})"));
                    None
                }
                Err(error) => {
                    status.check = Some(format!("failed: {error}"));
                    Some(error)
                }
            }
        };
        match ctx.output.format() {
            Format::Json => ctx.output.json(&status)?,
            Format::Ndjson => ctx.output.ndjson(&status)?,
            Format::Table => ctx.output.write(&render(&status))?,
        }
        failure.map_or(Ok(()), Err)
    }
}

async fn stored(account: &str) -> (String, Option<OauthCredential>) {
    if let Err(error) = yt::credentials::init_os_store() {
        return (format!("unavailable: {error}"), None);
    }
    match yt::credentials::load(account).await {
        Ok(Some(StoredCredential::Oauth(credential))) => ("keyring".into(), Some(credential)),
        Ok(Some(StoredCredential::Token { .. })) => ("keyring".into(), None),
        Ok(None) => ("missing".into(), None),
        Err(error) => (format!("unavailable: {error}"), None),
    }
}

fn render(status: &StatusJson) -> String {
    let local = |millis: Option<i64>| millis.map(yt::time::local);
    let rows = [
        ("Profile", Some(status.profile.clone())),
        ("URL", Some(status.url.clone())),
        ("Login", status.login.clone()),
        ("Method", Some(status.method.to_string())),
        ("Credential", Some(status.credential.clone())),
        ("Access expires", local(status.access_expires)),
        ("Sign-in expires", local(status.sign_in_expires)),
        ("Check", status.check.clone()),
    ];
    rows.into_iter()
        .filter_map(|(label, value)| {
            value.map(|value| format!("{:<17}{value}", format!("{label}:")))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_times_are_utc_and_tables_are_local() {
        let millis = 1_791_590_400_000;
        let status = StatusJson {
            profile: "acme".into(),
            url: "https://acme.youtrack.cloud".into(),
            login: Some("jane".into()),
            method: "browser",
            credential: "keyring".into(),
            access_expires: Some(millis),
            sign_in_expires: Some(millis),
            check: None,
        };
        let value = serde_json::to_value(&status).unwrap();
        assert_eq!(value["access_expires"], "2026-10-10T00:00:00Z");
        assert_eq!(value["sign_in_expires"], "2026-10-10T00:00:00Z");
        assert!(render(&status).contains(&yt::time::local(millis)));
    }

    #[test]
    fn the_table_lists_the_login_after_the_url() {
        let status = StatusJson {
            profile: "acme".into(),
            url: "https://acme.youtrack.cloud".into(),
            login: Some("jane".into()),
            method: "browser",
            credential: "keyring".into(),
            access_expires: None,
            sign_in_expires: None,
            check: Some("ok".into()),
        };
        let labels: Vec<String> = render(&status)
            .lines()
            .map(|line| line.split(':').next().unwrap_or_default().to_string())
            .collect();
        assert_eq!(
            labels,
            ["Profile", "URL", "Login", "Method", "Credential", "Check"]
        );
    }
}
