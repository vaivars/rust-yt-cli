use std::path::{Path, PathBuf};

use chrono::Utc;
use futures::future::BoxFuture;
use miette::IntoDiagnostic as _;
use secrecy::{ExposeSecret as _, SecretString};
use sha2::{Digest as _, Sha256};
use url::Url;

use crate::api::TokenSource;
use crate::auth::oauth::{self, Refresh};
use crate::credentials::{self, OauthCredential, StoredCredential};
use crate::error::{Result, YtError};

/// A permanent token, from the keyring or `YOUTRACK_TOKEN`.
pub struct StaticToken {
    token: SecretString,
    label: String,
}

impl StaticToken {
    pub fn new(token: SecretString, label: impl Into<String>) -> Self {
        crate::redact::register(token.expose_secret());
        Self {
            token,
            label: label.into(),
        }
    }
}

impl TokenSource for StaticToken {
    fn bearer(&self) -> BoxFuture<'_, Result<SecretString>> {
        Box::pin(async move { Ok(SecretString::from(self.token.expose_secret().to_string())) })
    }

    fn refresh_after_unauthorized(&self) -> BoxFuture<'_, Result<bool>> {
        Box::pin(async { Ok(false) })
    }

    fn label(&self) -> &str {
        &self.label
    }
}

/// A browser sign-in: refreshes the access token when it is about to expire or is rejected.
pub struct OAuthToken {
    http: reqwest::Client,
    account: String,
    label: String,
    lock_path: PathBuf,
    current: tokio::sync::Mutex<OauthCredential>,
}

impl OAuthToken {
    pub fn new(
        http: reqwest::Client,
        account: impl Into<String>,
        label: impl Into<String>,
        state_dir: &Path,
        credential: OauthCredential,
    ) -> Self {
        let account = account.into();
        crate::redact::register(&credential.access_token);
        crate::redact::register(&credential.refresh_token);
        let digest = Sha256::digest(account.as_bytes());
        let name: String = digest
            .iter()
            .take(8)
            .map(|byte| format!("{byte:02x}"))
            .collect();
        Self {
            http,
            lock_path: state_dir.join(format!("refresh-{name}.lock")),
            account,
            label: label.into(),
            current: tokio::sync::Mutex::new(credential),
        }
    }

    /// Refreshes under a file lock shared by every yt process, then re-reads the keyring first in
    /// case another process already refreshed.
    async fn refresh(&self, current: &mut OauthCredential) -> Result<()> {
        let _lock = lock(self.lock_path.clone()).await?;
        if let Some(StoredCredential::Oauth(stored)) = credentials::load(&self.account).await?
            && stored.access_token != current.access_token
            && !expiring(&stored)
        {
            *current = stored;
            return Ok(());
        }
        let endpoint = Url::parse(&current.token_endpoint).map_err(|error| YtError::OAuth {
            message: format!("invalid token endpoint: {error}"),
            help: None,
        })?;
        let refresh_token = SecretString::from(current.refresh_token.clone());
        match oauth::refresh(
            &self.http,
            &endpoint,
            &current.client_id,
            &refresh_token,
            &current.scope,
        )
        .await?
        {
            Refresh::Expired => Err(YtError::SessionExpired {
                profile: self.label.clone(),
            }
            .into()),
            Refresh::Fresh(tokens) => {
                current.access_token = tokens.access_token.expose_secret().to_string();
                if let Some(rotated) = tokens.refresh_token {
                    current.refresh_token = rotated.expose_secret().to_string();
                }
                current.expires_at = Utc::now() + chrono::Duration::seconds(tokens.expires_in);
                credentials::save(&self.account, &StoredCredential::Oauth(current.clone())).await
            }
        }
    }
}

impl TokenSource for OAuthToken {
    fn bearer(&self) -> BoxFuture<'_, Result<SecretString>> {
        Box::pin(async move {
            let mut current = self.current.lock().await;
            if expiring(&current) {
                self.refresh(&mut current).await?;
            }
            Ok(SecretString::from(current.access_token.clone()))
        })
    }

    fn refresh_after_unauthorized(&self) -> BoxFuture<'_, Result<bool>> {
        Box::pin(async move {
            let mut current = self.current.lock().await;
            self.refresh(&mut current).await?;
            Ok(true)
        })
    }

    fn label(&self) -> &str {
        &self.label
    }
}

fn expiring(credential: &OauthCredential) -> bool {
    credential.expires_at - Utc::now() < chrono::Duration::seconds(60)
}

async fn lock(path: PathBuf) -> Result<std::fs::File> {
    tokio::task::spawn_blocking(move || -> std::io::Result<std::fs::File> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&path)?;
        file.lock()?;
        Ok(file)
    })
    .await
    .into_diagnostic()?
    .into_diagnostic()
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use serde_json::json;
    use wiremock::matchers::{body_string_contains, method};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;

    const ACCOUNT: &str = "jane@refresh.youtrack.cloud";

    fn http() -> reqwest::Client {
        crate::http::client(&crate::http::HttpConfig {
            timeout: Duration::from_secs(5),
            retries: 0,
            ca_file: None,
        })
        .unwrap()
    }

    fn credential(server: &MockServer, expires_in: i64) -> OauthCredential {
        let now = Utc::now();
        OauthCredential {
            access_token: "old-access-0123456789".into(),
            refresh_token: "refresh-0123456789".into(),
            expires_at: now + chrono::Duration::seconds(expires_in),
            client_id: "yt-cli".into(),
            token_endpoint: format!("{}/hub/api/rest/oauth2/token", server.uri()),
            scope: "svc".into(),
            created_at: now,
        }
    }

    async fn setup(server: &MockServer, expires_in: i64) -> (tempfile::TempDir, OauthCredential) {
        keyring_core::set_default_store(keyring_core::mock::Store::new().unwrap());
        let credential = credential(server, expires_in);
        credentials::save(ACCOUNT, &StoredCredential::Oauth(credential.clone()))
            .await
            .unwrap();
        (tempfile::tempdir().unwrap(), credential)
    }

    fn fresh_tokens() -> ResponseTemplate {
        ResponseTemplate::new(200).set_body_json(json!({"access_token": "new-access-0123456789", "expires_in": 3600, "token_type": "Bearer"}))
    }

    #[tokio::test]
    async fn a_valid_token_is_used_as_is() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(fresh_tokens())
            .expect(0)
            .mount(&server)
            .await;
        let (state, credential) = setup(&server, 3600).await;
        let source = OAuthToken::new(http(), ACCOUNT, "acme", state.path(), credential);
        assert_eq!(
            source.bearer().await.unwrap().expose_secret(),
            "old-access-0123456789"
        );
    }

    #[tokio::test]
    async fn an_expiring_token_is_refreshed_and_saved() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(body_string_contains("grant_type=refresh_token"))
            .respond_with(fresh_tokens())
            .expect(1)
            .mount(&server)
            .await;
        let (state, credential) = setup(&server, 10).await;
        let source = OAuthToken::new(http(), ACCOUNT, "acme", state.path(), credential);
        assert_eq!(
            source.bearer().await.unwrap().expose_secret(),
            "new-access-0123456789"
        );
        let Some(StoredCredential::Oauth(saved)) = credentials::load(ACCOUNT).await.unwrap() else {
            panic!("not saved")
        };
        assert_eq!(saved.access_token, "new-access-0123456789");
        assert_eq!(saved.refresh_token, "refresh-0123456789");
    }

    #[tokio::test]
    async fn an_invalid_grant_means_the_sign_in_expired() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(400).set_body_json(
                json!({"error": "invalid_grant", "error_description": "Unknown refresh token"}),
            ))
            .mount(&server)
            .await;
        let (state, credential) = setup(&server, 10).await;
        let source = OAuthToken::new(http(), ACCOUNT, "acme", state.path(), credential);
        let error = source.bearer().await.unwrap_err();
        assert!(
            matches!(error.downcast_ref::<YtError>(), Some(YtError::SessionExpired { profile }) if profile == "acme")
        );
        assert_eq!(crate::error::exit_code(&error), 4);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn two_processes_refreshing_at_once_send_one_request() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(fresh_tokens().set_delay(Duration::from_millis(300)))
            .expect(1)
            .mount(&server)
            .await;
        let (state, credential) = setup(&server, 10).await;
        // Two sources with separate in-memory state stand in for two yt processes.
        let first = OAuthToken::new(http(), ACCOUNT, "acme", state.path(), credential.clone());
        let second = OAuthToken::new(http(), ACCOUNT, "acme", state.path(), credential);
        let (a, b) = tokio::join!(first.bearer(), second.bearer());
        assert_eq!(a.unwrap().expose_secret(), "new-access-0123456789");
        assert_eq!(b.unwrap().expose_secret(), "new-access-0123456789");
    }
}
