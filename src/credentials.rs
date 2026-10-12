use std::fmt;
use std::sync::OnceLock;

use chrono::{DateTime, Utc};
use miette::IntoDiagnostic as _;
use serde::{Deserialize, Serialize};

use crate::error::{Result, YtError};

const SERVICE: &str = "yt";

#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum StoredCredential {
    Token { token: String },
    Oauth(OauthCredential),
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct OauthCredential {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at: DateTime<Utc>,
    pub client_id: String,
    pub token_endpoint: String,
    pub scope: String,
    pub created_at: DateTime<Utc>,
}

impl fmt::Debug for StoredCredential {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Token { .. } => f.write_str("Token([REDACTED])"),
            Self::Oauth(credential) => credential.fmt(f),
        }
    }
}

impl fmt::Debug for OauthCredential {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OauthCredential")
            .field("expires_at", &self.expires_at)
            .field("client_id", &self.client_id)
            .field("scope", &self.scope)
            .field("created_at", &self.created_at)
            .finish_non_exhaustive()
    }
}

impl StoredCredential {
    fn register_secrets(&self) {
        match self {
            Self::Token { token } => crate::redact::register(token),
            Self::Oauth(credential) => {
                crate::redact::register(&credential.access_token);
                crate::redact::register(&credential.refresh_token);
            }
        }
    }
}

static OS_STORE: OnceLock<std::result::Result<(), String>> = OnceLock::new();

/// Connects to the OS keyring: Keychain on macOS, Secret Service on Linux.
pub fn init_os_store() -> Result<()> {
    let ready = OS_STORE.get_or_init(|| {
        #[cfg(target_os = "macos")]
        let store = apple_native_keyring_store::keychain::Store::new();
        #[cfg(target_os = "linux")]
        let store = zbus_secret_service_keyring_store::Store::new();
        store
            .map(|store| keyring_core::set_default_store(store))
            .map_err(|error| error.to_string())
    });
    ready
        .clone()
        .map_err(|reason| YtError::NoKeyring { reason }.into())
}

pub async fn load(account: &str) -> Result<Option<StoredCredential>> {
    let account = account.to_string();
    let stored = blocking(move || match entry(&account)?.get_password() {
        Ok(text) => Ok(Some(text)),
        Err(keyring_core::Error::NoEntry) => Ok(None),
        Err(error) => Err(keyring_error(error)),
    })
    .await?;
    let Some(text) = stored else {
        return Ok(None);
    };
    let credential: StoredCredential = serde_json::from_str(&text).map_err(|_| {
        YtError::Keyring("the stored credential is unreadable; sign in again".into())
    })?;
    credential.register_secrets();
    Ok(Some(credential))
}

pub async fn save(account: &str, credential: &StoredCredential) -> Result<()> {
    credential.register_secrets();
    let account = account.to_string();
    let text = serde_json::to_string(credential).into_diagnostic()?;
    blocking(move || entry(&account)?.set_password(&text).map_err(keyring_error)).await
}

pub async fn delete(account: &str) -> Result<()> {
    let account = account.to_string();
    blocking(move || match entry(&account)?.delete_credential() {
        Ok(()) | Err(keyring_core::Error::NoEntry) => Ok(()),
        Err(error) => Err(keyring_error(error)),
    })
    .await
}

fn entry(account: &str) -> Result<keyring_core::Entry> {
    keyring_core::Entry::new(SERVICE, account).map_err(keyring_error)
}

fn keyring_error(error: keyring_core::Error) -> miette::Report {
    match error {
        keyring_core::Error::NoDefaultStore | keyring_core::Error::NoStorageAccess(_) => {
            YtError::NoKeyring {
                reason: error.to_string(),
            }
            .into()
        }
        other => YtError::Keyring(other.to_string()).into(),
    }
}

/// Keyring calls may block on an OS dialog, so they run off the async runtime.
async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T> + Send + 'static,
) -> Result<T> {
    tokio::task::spawn_blocking(work).await.into_diagnostic()?
}

#[cfg(test)]
mod tests {
    use super::*;

    fn use_mock_store() {
        keyring_core::set_default_store(keyring_core::mock::Store::new().unwrap());
    }

    fn oauth() -> StoredCredential {
        let now = Utc::now();
        StoredCredential::Oauth(OauthCredential {
            access_token: "access-token-1234567890".into(),
            refresh_token: "refresh-token-1234567890".into(),
            expires_at: now,
            client_id: "yt-cli".into(),
            token_endpoint: "https://acme.youtrack.cloud/hub/api/rest/oauth2/token".into(),
            scope: "service-id".into(),
            created_at: now,
        })
    }

    #[tokio::test]
    async fn round_trips_through_the_keyring() {
        use_mock_store();
        let account = "jane@roundtrip.youtrack.cloud";
        assert_eq!(load(account).await.unwrap(), None);
        let credential = oauth();
        save(account, &credential).await.unwrap();
        assert_eq!(load(account).await.unwrap(), Some(credential));
        delete(account).await.unwrap();
        assert_eq!(load(account).await.unwrap(), None);
        delete(account).await.unwrap();
    }

    #[test]
    fn json_shape_is_tagged_by_kind() {
        let text = serde_json::to_string(&StoredCredential::Token {
            token: "perm-abc".into(),
        })
        .unwrap();
        assert_eq!(text, r#"{"kind":"token","token":"perm-abc"}"#);
    }

    #[test]
    fn debug_never_shows_secrets() {
        let text = format!("{:?}", oauth());
        assert!(
            !text.contains("access-token") && !text.contains("refresh-token"),
            "{text}"
        );
    }

    #[tokio::test]
    async fn loaded_secrets_are_redacted_in_logs() {
        use_mock_store();
        let account = "jane@redact.youtrack.cloud";
        save(
            account,
            &StoredCredential::Token {
                token: "perm-cmVkYWN0.dGVzdA==.loaded1234".into(),
            },
        )
        .await
        .unwrap();
        load(account).await.unwrap();
        assert_eq!(
            crate::redact::redact("perm-cmVkYWN0.dGVzdA==.loaded1234"),
            "[REDACTED]"
        );
    }
}
