use reqwest::header::ACCEPT;
use serde::Deserialize;
use url::Url;

use crate::error::{Result, YtError};
use crate::instance::InstanceUrl;

/// What an instance says about itself without credentials.
#[derive(Clone, Debug)]
pub struct Discovery {
    pub url: InstanceUrl,
    pub version: String,
    pub service_id: String,
    pub hub: Url,
    pub embedded_hub: bool,
    pub authorization_endpoint: Url,
    pub token_endpoint: Url,
    pub cimd_supported: bool,
}

#[derive(Deserialize)]
struct FrontendConfig {
    version: Option<String>,
    ring: Option<Ring>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Ring {
    url: String,
    service_id: String,
    #[serde(default)]
    has_embedded_hub: bool,
}

#[derive(Deserialize)]
struct Metadata {
    authorization_endpoint: Url,
    token_endpoint: Url,
    #[serde(default)]
    client_id_metadata_document_supported: bool,
}

impl Discovery {
    /// Where the user creates a permanent token.
    pub fn token_page(&self) -> Url {
        if self.embedded_hub {
            self.url.join("users/me?tab=account-security")
        } else {
            self.hub
                .join("users/me?tab=authentication")
                .unwrap_or_else(|_| self.hub.clone())
        }
    }
}

pub async fn discover(http: &reqwest::Client, url: &InstanceUrl) -> Result<Discovery> {
    let fail = |reason: String| -> miette::Report {
        YtError::Discovery {
            url: url.to_string(),
            reason,
        }
        .into()
    };
    let mut config_url = url
        .api()
        .join("config")
        .map_err(|error| fail(error.to_string()))?;
    config_url.set_query(Some("fields=version,ring(url,serviceId,hasEmbeddedHub)"));
    let response = http
        .get(config_url)
        .header(ACCEPT, "application/json")
        .send()
        .await
        .map_err(|error| fail(crate::http::describe(&error)))?;
    if !response.status().is_success() {
        return Err(fail(format!(
            "/api/config answered HTTP {}",
            response.status()
        )));
    }
    let config: FrontendConfig = response
        .json()
        .await
        .map_err(|_| fail("/api/config did not return a YouTrack configuration".into()))?;
    let ring = config
        .ring
        .ok_or_else(|| fail("the instance reported no Hub".into()))?;
    let hub = hub_base(url, &ring.url).map_err(fail)?;
    let (authorization_endpoint, token_endpoint, cimd_supported) =
        match fetch_metadata(http, &hub).await {
            Some(metadata) => (
                metadata.authorization_endpoint,
                metadata.token_endpoint,
                metadata.client_id_metadata_document_supported,
            ),
            None => (
                hub.join("api/rest/oauth2/auth")
                    .map_err(|error| fail(error.to_string()))?,
                hub.join("api/rest/oauth2/token")
                    .map_err(|error| fail(error.to_string()))?,
                false,
            ),
        };
    if let Some(reason) = insecure_endpoint(
        url,
        &[
            ("Hub", &hub),
            ("authorization endpoint", &authorization_endpoint),
            ("token endpoint", &token_endpoint),
        ],
    ) {
        return Err(fail(reason));
    }
    Ok(Discovery {
        url: url.clone(),
        version: config.version.unwrap_or_default(),
        service_id: ring.service_id,
        hub,
        embedded_hub: ring.has_embedded_hub,
        authorization_endpoint,
        token_endpoint,
        cimd_supported,
    })
}

/// Sign-in codes and tokens must not leave an `https` instance over plain `http`.
fn insecure_endpoint(url: &InstanceUrl, endpoints: &[(&str, &Url)]) -> Option<String> {
    if url.origin().scheme() != "https" {
        return None;
    }
    endpoints
        .iter()
        .find(|(_, endpoint)| endpoint.scheme() != "https")
        .map(|(name, endpoint)| {
            format!("the instance uses https, but its {name} {endpoint} does not")
        })
}

/// `ring.url` is absolute for an external Hub and an origin-relative path (`/hub`) for a built-in one.
pub fn hub_base(instance: &InstanceUrl, ring_url: &str) -> std::result::Result<Url, String> {
    let mut hub = instance
        .origin()
        .join(ring_url)
        .map_err(|error| format!("invalid Hub URL '{ring_url}': {error}"))?;
    if !hub.path().ends_with('/') {
        let path = format!("{}/", hub.path());
        hub.set_path(&path);
    }
    Ok(hub)
}

async fn fetch_metadata(http: &reqwest::Client, hub: &Url) -> Option<Metadata> {
    let url = hub.join(".well-known/oauth-authorization-server").ok()?;
    let response = http
        .get(url)
        .header(ACCEPT, "application/json")
        .send()
        .await
        .ok()?;
    if !response.status().is_success() {
        return None;
    }
    response.json().await.ok()
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;

    fn http() -> reqwest::Client {
        crate::http::client(&crate::http::HttpConfig {
            timeout: std::time::Duration::from_secs(5),
            retries: 0,
            ca_file: None,
        })
        .unwrap()
    }

    #[tokio::test]
    async fn built_in_hub_resolves_against_the_origin_not_the_context_path() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/youtrack/api/config"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "version": "2026.2",
                "ring": {"url": "/hub", "serviceId": "svc-1", "hasEmbeddedHub": true}
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/hub/.well-known/oauth-authorization-server"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "authorization_endpoint": format!("{}/hub/api/rest/oauth2/auth", server.uri()),
                "token_endpoint": format!("{}/hub/api/rest/oauth2/token", server.uri()),
                "client_id_metadata_document_supported": true
            })))
            .mount(&server)
            .await;
        let url = InstanceUrl::parse(&format!("{}/youtrack", server.uri())).unwrap();
        let discovery = discover(&http(), &url).await.unwrap();
        assert_eq!(discovery.hub.as_str(), format!("{}/hub/", server.uri()));
        assert_eq!(discovery.service_id, "svc-1");
        assert!(discovery.cimd_supported);
        assert_eq!(
            discovery.token_endpoint.path(),
            "/hub/api/rest/oauth2/token"
        );
        assert_eq!(
            discovery.token_page().as_str(),
            format!("{}/youtrack/users/me?tab=account-security", server.uri())
        );
    }

    #[tokio::test]
    async fn an_external_hub_without_metadata_uses_the_default_endpoints() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/config"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "version": "2026.1",
                "ring": {"url": format!("{}/external-hub", server.uri()), "serviceId": "svc-2", "hasEmbeddedHub": false}
            })))
            .mount(&server)
            .await;
        let url = InstanceUrl::parse(&server.uri()).unwrap();
        let discovery = discover(&http(), &url).await.unwrap();
        assert_eq!(discovery.hub.path(), "/external-hub/");
        assert_eq!(
            discovery.authorization_endpoint.path(),
            "/external-hub/api/rest/oauth2/auth"
        );
        assert!(!discovery.cimd_supported);
        assert_eq!(discovery.token_page().path(), "/external-hub/users/me");
    }

    #[test]
    fn an_https_instance_refuses_http_hub_endpoints() {
        let url = InstanceUrl::parse("https://acme.example").unwrap();
        let secure = Url::parse("https://hub.example/hub/").unwrap();
        let plain = Url::parse("http://acme.example/hub/api/rest/oauth2/token").unwrap();
        assert_eq!(
            insecure_endpoint(&url, &[("Hub", &secure), ("token endpoint", &secure)]),
            None
        );
        assert_eq!(
            insecure_endpoint(&url, &[("Hub", &secure), ("token endpoint", &plain)]).as_deref(),
            Some(
                "the instance uses https, but its token endpoint http://acme.example/hub/api/rest/oauth2/token does not"
            )
        );
    }

    #[test]
    fn an_http_instance_may_use_http_hub_endpoints() {
        let url = InstanceUrl::parse("http://youtrack.lan").unwrap();
        let plain = Url::parse("http://youtrack.lan/hub/").unwrap();
        assert_eq!(insecure_endpoint(&url, &[("Hub", &plain)]), None);
    }

    #[tokio::test]
    async fn something_that_is_not_youtrack_is_a_discovery_error() {
        let server = MockServer::start().await;
        let url = InstanceUrl::parse(&server.uri()).unwrap();
        let error = discover(&http(), &url).await.unwrap_err();
        assert!(matches!(
            error.downcast_ref::<YtError>(),
            Some(YtError::Discovery { .. })
        ));
    }
}
