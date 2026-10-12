use chrono::{DateTime, Utc};
use reqwest::header::{ACCEPT, AUTHORIZATION, HeaderValue, LOCATION};
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use url::Url;

use crate::auth::discovery::{Discovery, discover};
use crate::error::{Result, YtError};
use crate::instance::InstanceUrl;

/// Hub's own service ID; adding it to the scope lets yt manage its refresh tokens.
pub const HUB_SCOPE: &str = "0-0-0-0-0";

pub fn scope(service_id: &str, revocable: bool) -> String {
    if revocable {
        format!("{service_id} {HUB_SCOPE}")
    } else {
        service_id.to_string()
    }
}

pub struct AuthorizeRequest<'a> {
    pub client_id: &'a str,
    pub redirect_uri: &'a str,
    pub scope: &'a str,
    pub state: &'a str,
    pub challenge: &'a str,
}

pub fn authorize_url(discovery: &Discovery, request: &AuthorizeRequest<'_>) -> Url {
    let mut url = discovery.authorization_endpoint.clone();
    url.query_pairs_mut()
        .append_pair("response_type", "code")
        .append_pair("client_id", request.client_id)
        .append_pair("redirect_uri", request.redirect_uri)
        .append_pair("scope", request.scope)
        .append_pair("state", request.state)
        .append_pair("code_challenge", request.challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("access_type", "offline")
        .append_pair("request_credentials", "default");
    url
}

/// Sends the authorize request without following redirects, so a rejected request fails here with
/// Hub's reason; Hub's own error page can fail to render.
pub async fn preflight(
    no_redirects: &reqwest::Client,
    discovery: &Discovery,
    authorize: &Url,
) -> Result<()> {
    let response = no_redirects
        .get(authorize.clone())
        .send()
        .await
        .map_err(network)?;
    let status = response.status();
    let Some(location) = response
        .headers()
        .get(LOCATION)
        .and_then(|value| value.to_str().ok())
    else {
        return if status.is_success() {
            Ok(())
        } else {
            Err(YtError::OAuth {
                message: format!("the sign-in page answered HTTP {status}"),
                help: None,
            }
            .into())
        };
    };
    let Ok(target) = authorize.join(location) else {
        return Ok(());
    };
    if !target.path().ends_with("/auth/oauth/error") {
        return Ok(());
    }
    let token = target
        .query_pairs()
        .find(|(key, _)| key == "message_token")
        .map(|(_, value)| value.into_owned());
    let reason = match token {
        Some(token) => hub_message(no_redirects, &discovery.hub, &token).await,
        None => None,
    };
    let (error, description) =
        reason.unwrap_or_else(|| ("rejected".into(), "Hub rejected the sign-in request".into()));
    let message = if description.is_empty() {
        error.clone()
    } else {
        description
    };
    Err(YtError::OAuth {
        message,
        help: help_for(&error),
    }
    .into())
}

pub fn help_for(error: &str) -> Option<String> {
    let help = match error {
        "invalid_request" | "invalid_client" | "unauthorized_client" => {
            "check the OAuth client ID your administrator registered for yt"
        }
        "invalid_redirect_uri" => {
            "the OAuth client must allow the redirect URI http://127.0.0.1:<any port>/callback"
        }
        "access_denied" => "the sign-in was not approved",
        "invalid_grant" => "start `yt auth login` again",
        _ => return None,
    };
    Some(help.to_string())
}

#[derive(Debug)]
pub struct Tokens {
    pub access_token: SecretString,
    pub refresh_token: Option<SecretString>,
    pub expires_in: i64,
}

pub enum Refresh {
    Fresh(Tokens),
    Expired,
}

pub async fn exchange_code(
    http: &reqwest::Client,
    discovery: &Discovery,
    client_id: &str,
    code: &str,
    redirect_uri: &str,
    verifier: &SecretString,
) -> Result<Tokens> {
    let form = [
        ("grant_type", "authorization_code"),
        ("code", code),
        ("redirect_uri", redirect_uri),
        ("code_verifier", verifier.expose_secret()),
        ("client_id", client_id),
    ];
    let response = http
        .post(discovery.token_endpoint.clone())
        .header(ACCEPT, "application/json")
        .form(&form)
        .send()
        .await
        .map_err(network)?;
    read_tokens(response)
        .await
        .map_err(|failure| match failure {
            TokenFailure::Rejected { error, description } => {
                let message = if description.is_empty() {
                    error.clone()
                } else {
                    description
                };
                YtError::OAuth {
                    message,
                    help: help_for(&error),
                }
                .into()
            }
            TokenFailure::Other(report) => report,
        })
}

pub async fn refresh(
    http: &reqwest::Client,
    token_endpoint: &Url,
    client_id: &str,
    refresh_token: &SecretString,
    scope: &str,
) -> Result<Refresh> {
    let form = [
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh_token.expose_secret()),
        ("scope", scope),
        ("client_id", client_id),
    ];
    let response = http
        .post(token_endpoint.clone())
        .header(ACCEPT, "application/json")
        .form(&form)
        .send()
        .await
        .map_err(network)?;
    match read_tokens(response).await {
        Ok(tokens) => Ok(Refresh::Fresh(tokens)),
        Err(TokenFailure::Rejected { error, .. }) if error == "invalid_grant" => {
            Ok(Refresh::Expired)
        }
        Err(TokenFailure::Rejected { error, description }) => Err(YtError::OAuth {
            message: format!("{error}: {description}"),
            help: help_for(&error),
        }
        .into()),
        Err(TokenFailure::Other(report)) => Err(report),
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct HubRefreshToken {
    pub id: String,
    pub created: i64,
    pub expires: Option<i64>,
    pub client_key: Option<String>,
}

pub async fn list_refresh_tokens(
    http: &reqwest::Client,
    hub: &Url,
    access: &SecretString,
) -> Result<Vec<HubRefreshToken>> {
    #[derive(Deserialize)]
    struct Listing {
        #[serde(default)]
        refreshtokens: Vec<Item>,
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Item {
        id: String,
        creation_time: Option<i64>,
        expires: Option<i64>,
        client: Option<ClientRef>,
    }
    #[derive(Deserialize)]
    struct ClientRef {
        key: Option<String>,
    }

    let mut url = hub
        .join("api/rest/users/me/refreshtokens")
        .map_err(|error| YtError::OAuth {
            message: error.to_string(),
            help: None,
        })?;
    url.query_pairs_mut()
        .append_pair("fields", "id,creationTime,expires,client(key)")
        .append_pair("$top", "100");
    let response = http
        .get(url)
        .header(ACCEPT, "application/json")
        .header(AUTHORIZATION, bearer(access))
        .send()
        .await
        .map_err(network)?;
    let status = response.status();
    let text = response.text().await.map_err(network)?;
    if !status.is_success() {
        return Err(YtError::OAuth {
            message: crate::api::error::unexpected(status, &text),
            help: Some("revoking sign-ins needs `yt auth login --revocable`".into()),
        }
        .into());
    }
    let listing: Listing = serde_json::from_str(&text).map_err(|error| YtError::OAuth {
        message: error.to_string(),
        help: None,
    })?;
    Ok(listing
        .refreshtokens
        .into_iter()
        .map(|item| HubRefreshToken {
            id: item.id,
            created: item.creation_time.unwrap_or(0),
            expires: item.expires,
            client_key: item.client.and_then(|client| client.key),
        })
        .collect())
}

pub async fn delete_refresh_token(
    http: &reqwest::Client,
    hub: &Url,
    access: &SecretString,
    id: &str,
) -> Result<()> {
    let url = hub
        .join(&format!("api/rest/users/me/refreshtokens/{id}"))
        .map_err(|error| YtError::OAuth {
            message: error.to_string(),
            help: None,
        })?;
    let response = http
        .delete(url)
        .header(AUTHORIZATION, bearer(access))
        .send()
        .await
        .map_err(network)?;
    let status = response.status();
    if status.is_success() {
        Ok(())
    } else {
        let text = response.text().await.unwrap_or_default();
        Err(YtError::OAuth {
            message: crate::api::error::unexpected(status, &text),
            help: None,
        }
        .into())
    }
}

/// The refresh token yt was issued at `created`: same client, closest in time, within two minutes.
pub fn find_issued<'a>(
    tokens: &'a [HubRefreshToken],
    client_id: &str,
    created: DateTime<Utc>,
) -> Option<&'a HubRefreshToken> {
    let target = created.timestamp_millis();
    tokens
        .iter()
        .filter(|token| token.client_key.as_deref() == Some(client_id))
        .filter(|token| (token.created - target).abs() <= 120_000)
        .min_by_key(|token| (token.created - target).abs())
}

/// Finds the Hub and the refresh token of the sign-in made at `created`; needs a revocable sign-in.
pub async fn issued_refresh_token(
    http: &reqwest::Client,
    url: &InstanceUrl,
    access: &SecretString,
    client_id: &str,
    created: DateTime<Utc>,
) -> Result<Option<(Url, HubRefreshToken)>> {
    let discovery = discover(http, url).await?;
    let tokens = list_refresh_tokens(http, &discovery.hub, access).await?;
    Ok(find_issued(&tokens, client_id, created)
        .cloned()
        .map(|token| (discovery.hub, token)))
}

enum TokenFailure {
    Rejected { error: String, description: String },
    Other(miette::Report),
}

async fn read_tokens(response: reqwest::Response) -> std::result::Result<Tokens, TokenFailure> {
    #[derive(Deserialize)]
    struct TokenBody {
        access_token: String,
        refresh_token: Option<String>,
        expires_in: Option<i64>,
    }
    #[derive(Deserialize)]
    struct TokenError {
        error: String,
        error_description: Option<String>,
    }

    let status = response.status();
    let text = response
        .text()
        .await
        .map_err(|error| TokenFailure::Other(network(error)))?;
    if status.is_success() {
        let body: TokenBody = serde_json::from_str(&text).map_err(|_| {
            TokenFailure::Other(
                YtError::OAuth {
                    message: "the token endpoint returned an unexpected response".into(),
                    help: None,
                }
                .into(),
            )
        })?;
        crate::redact::register(&body.access_token);
        if let Some(refresh_token) = &body.refresh_token {
            crate::redact::register(refresh_token);
        }
        return Ok(Tokens {
            access_token: SecretString::from(body.access_token),
            refresh_token: body.refresh_token.map(SecretString::from),
            // An absurd lifetime would overflow the expiry date; refreshing sooner is harmless.
            expires_in: body.expires_in.unwrap_or(3600).clamp(0, 24 * 60 * 60),
        });
    }
    match serde_json::from_str::<TokenError>(&text) {
        Ok(error) => Err(TokenFailure::Rejected {
            error: error.error,
            description: error.error_description.unwrap_or_default(),
        }),
        Err(_) => Err(TokenFailure::Other(
            YtError::OAuth {
                message: crate::api::error::unexpected(status, &text),
                help: None,
            }
            .into(),
        )),
    }
}

async fn hub_message(http: &reqwest::Client, hub: &Url, token: &str) -> Option<(String, String)> {
    #[derive(Deserialize)]
    struct Message {
        error: Option<String>,
        error_description: Option<String>,
    }
    let mut url = hub.join("api/rest/oauth/message").ok()?;
    url.query_pairs_mut().append_pair("token", token);
    let message: Message = http
        .get(url)
        .header(ACCEPT, "application/json")
        .send()
        .await
        .ok()?
        .json()
        .await
        .ok()?;
    Some((
        message.error.unwrap_or_else(|| "error".into()),
        message.error_description.unwrap_or_default(),
    ))
}

fn bearer(token: &SecretString) -> HeaderValue {
    let mut value = HeaderValue::from_str(&format!("Bearer {}", token.expose_secret()))
        .unwrap_or_else(|_| HeaderValue::from_static("Bearer invalid"));
    value.set_sensitive(true);
    value
}

fn network(error: reqwest::Error) -> miette::Report {
    YtError::Network(crate::redact::redact(&error.to_string())).into()
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use wiremock::matchers::{body_string_contains, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;

    fn http(redirects: bool) -> reqwest::Client {
        let config = crate::http::HttpConfig {
            timeout: std::time::Duration::from_secs(5),
            retries: 0,
            ca_file: None,
        };
        if redirects {
            crate::http::client(&config).unwrap()
        } else {
            crate::http::client_without_redirects(&config).unwrap()
        }
    }

    fn discovery(server: &MockServer) -> Discovery {
        let hub = Url::parse(&format!("{}/hub/", server.uri())).unwrap();
        Discovery {
            url: InstanceUrl::parse(&server.uri()).unwrap(),
            version: "2026.2".into(),
            service_id: "svc".into(),
            authorization_endpoint: hub.join("api/rest/oauth2/auth").unwrap(),
            token_endpoint: hub.join("api/rest/oauth2/token").unwrap(),
            hub,
            embedded_hub: true,
            cimd_supported: false,
        }
    }

    fn request<'a>() -> AuthorizeRequest<'a> {
        AuthorizeRequest {
            client_id: "yt-cli",
            redirect_uri: "http://127.0.0.1:5000/callback",
            scope: "svc",
            state: "st4te",
            challenge: "ch4llenge",
        }
    }

    #[tokio::test]
    async fn the_authorize_url_carries_pkce_and_offline_access() {
        let server = MockServer::start().await;
        let url = authorize_url(&discovery(&server), &request());
        let query: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
        assert_eq!(query["response_type"], "code");
        assert_eq!(query["client_id"], "yt-cli");
        assert_eq!(query["redirect_uri"], "http://127.0.0.1:5000/callback");
        assert_eq!(query["code_challenge_method"], "S256");
        assert_eq!(query["access_type"], "offline");
        assert_eq!(query["request_credentials"], "default");
        assert_eq!(scope("svc", true), "svc 0-0-0-0-0");
        assert_eq!(scope("svc", false), "svc");
    }

    #[tokio::test]
    async fn preflight_accepts_a_redirect_to_the_login_page() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/hub/api/rest/oauth2/auth"))
            .respond_with(
                ResponseTemplate::new(303).insert_header("location", "/hub/auth/login?flow=x"),
            )
            .mount(&server)
            .await;
        let discovery = discovery(&server);
        preflight(
            &http(false),
            &discovery,
            &authorize_url(&discovery, &request()),
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn preflight_reports_the_hub_reason_for_a_rejection() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/hub/api/rest/oauth2/auth"))
            .respond_with(ResponseTemplate::new(303).insert_header(
                "location",
                "/hub/auth/oauth/error?message_token=T1&client_id=yt-cli",
            ))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/hub/api/rest/oauth/message"))
            .and(query_param("token", "T1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "error": "invalid_redirect_uri",
                "error_description": "Unregistered Redirect URI http://localhost:5000/callback"
            })))
            .mount(&server)
            .await;
        let discovery = discovery(&server);
        let error = preflight(
            &http(false),
            &discovery,
            &authorize_url(&discovery, &request()),
        )
        .await
        .unwrap_err();
        assert!(
            error.to_string().contains("Unregistered Redirect URI"),
            "{error}"
        );
        let help = error
            .help()
            .map(|help| help.to_string())
            .unwrap_or_default();
        assert!(
            help.contains("http://127.0.0.1:<any port>/callback"),
            "{help}"
        );
    }

    #[tokio::test]
    async fn a_huge_lifetime_is_capped_at_a_day() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/hub/api/rest/oauth2/token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "access_token": "access-cap-0123456789",
                "expires_in": i64::MAX,
            })))
            .mount(&server)
            .await;
        let verifier = SecretString::from("v3rifier".to_string());
        let tokens = exchange_code(
            &http(true),
            &discovery(&server),
            "yt-cli",
            "c0de",
            "http://127.0.0.1:5000/callback",
            &verifier,
        )
        .await
        .unwrap();
        assert_eq!(tokens.expires_in, 24 * 60 * 60);
    }

    #[tokio::test]
    async fn exchanging_a_code_returns_tokens_and_registers_them_for_redaction() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/hub/api/rest/oauth2/token"))
            .and(body_string_contains("grant_type=authorization_code"))
            .and(body_string_contains("code_verifier=v3rifier"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "access_token": "access-0123456789",
                "refresh_token": "refresh-0123456789",
                "expires_in": 3600,
                "scope": "svc",
                "token_type": "Bearer"
            })))
            .mount(&server)
            .await;
        let verifier = SecretString::from("v3rifier".to_string());
        let tokens = exchange_code(
            &http(true),
            &discovery(&server),
            "yt-cli",
            "c0de",
            "http://127.0.0.1:5000/callback",
            &verifier,
        )
        .await
        .unwrap();
        assert_eq!(tokens.expires_in, 3600);
        assert!(tokens.refresh_token.is_some());
        assert_eq!(crate::redact::redact("access-0123456789"), "[REDACTED]");
    }

    #[tokio::test]
    async fn a_rejected_code_is_an_oauth_error() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(400).set_body_json(
                json!({"error": "invalid_grant", "error_description": "Code expired"}),
            ))
            .mount(&server)
            .await;
        let verifier = SecretString::from("v3rifier".to_string());
        let error = exchange_code(
            &http(true),
            &discovery(&server),
            "yt-cli",
            "c0de",
            "http://x",
            &verifier,
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("Code expired"), "{error}");
    }

    #[tokio::test]
    async fn an_invalid_grant_on_refresh_means_expired() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(body_string_contains("grant_type=refresh_token"))
            .respond_with(ResponseTemplate::new(400).set_body_json(
                json!({"error": "invalid_grant", "error_description": "Unknown refresh token"}),
            ))
            .mount(&server)
            .await;
        let token = SecretString::from("refresh-0123456789".to_string());
        let outcome = refresh(
            &http(true),
            &discovery(&server).token_endpoint,
            "yt-cli",
            &token,
            "svc",
        )
        .await
        .unwrap();
        assert!(matches!(outcome, Refresh::Expired));
    }

    #[tokio::test]
    async fn refresh_tokens_are_listed_and_deleted() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/hub/api/rest/users/me/refreshtokens"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "type": "RefreshTokenPage", "skip": 0, "top": 100, "total": 1,
                "refreshtokens": [{"id": "rt-1", "creationTime": 1_791_579_736_619_i64, "expires": 1_799_355_736_619_i64, "client": {"id": "uuid", "key": "yt-cli"}, "scope": ["svc"]}]
            })))
            .mount(&server)
            .await;
        Mock::given(method("DELETE"))
            .and(path("/hub/api/rest/users/me/refreshtokens/rt-1"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;
        let access = SecretString::from("access-0123456789".to_string());
        let hub = discovery(&server).hub;
        let tokens = list_refresh_tokens(&http(true), &hub, &access)
            .await
            .unwrap();
        assert_eq!(
            tokens,
            vec![HubRefreshToken {
                id: "rt-1".into(),
                created: 1_791_579_736_619,
                expires: Some(1_799_355_736_619),
                client_key: Some("yt-cli".into())
            }]
        );
        delete_refresh_token(&http(true), &hub, &access, "rt-1")
            .await
            .unwrap();
    }

    #[test]
    fn find_issued_picks_the_same_client_closest_in_time() {
        let created = DateTime::<Utc>::from_timestamp_millis(1_000_000_000).unwrap();
        let tokens = vec![
            HubRefreshToken {
                id: "other-client".into(),
                created: 1_000_000_100,
                expires: None,
                client_key: Some("other".into()),
            },
            HubRefreshToken {
                id: "far".into(),
                created: 1_000_500_000,
                expires: None,
                client_key: Some("yt-cli".into()),
            },
            HubRefreshToken {
                id: "near".into(),
                created: 1_000_001_500,
                expires: None,
                client_key: Some("yt-cli".into()),
            },
        ];
        assert_eq!(
            find_issued(&tokens, "yt-cli", created).map(|token| token.id.as_str()),
            Some("near")
        );
        assert_eq!(find_issued(&tokens[..2], "yt-cli", created), None);
    }

    #[tokio::test]
    async fn the_issued_refresh_token_is_found_through_discovery() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/config"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "version": "2026.2",
                "ring": {"url": "/hub", "serviceId": "svc", "hasEmbeddedHub": true}
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/hub/api/rest/users/me/refreshtokens"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "refreshtokens": [{"id": "rt-1", "creationTime": 1_000_000_000_i64, "expires": 8_776_000_000_i64, "client": {"key": "yt-cli"}}]
            })))
            .mount(&server)
            .await;
        let url = InstanceUrl::parse(&server.uri()).unwrap();
        let access = SecretString::from("access-0123456789".to_string());
        let created = DateTime::<Utc>::from_timestamp_millis(1_000_000_000).unwrap();
        let (hub, token) = issued_refresh_token(&http(true), &url, &access, "yt-cli", created)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(hub.path(), "/hub/");
        assert_eq!(token.expires, Some(8_776_000_000));
    }
}
