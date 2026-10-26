pub mod cache;
pub mod openapi;
pub mod registry;
pub mod request;

use reqwest::Method;

use crate::api::Client;
use crate::apps::cache::AppCache;
use crate::apps::openapi::AppApi;
use crate::apps::registry::AppEntry;
use crate::error::{Result, YtError};

/// Why a request to an app suggests its cached description is out of date.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stale {
    /// The app answered `403 disabled`: an administrator switched the endpoint off.
    Disabled,
    /// YouTrack has no handler for the path: the app is gone, changed, or not attached to the project.
    NoHandler,
}

pub fn stale(report: &miette::Report) -> Option<Stale> {
    match report.downcast_ref::<YtError>()? {
        YtError::Api {
            status: 403,
            code: Some(code),
            ..
        } if code == "disabled" => Some(Stale::Disabled),
        YtError::Api {
            status: 404,
            message,
            ..
        } if message.starts_with("HTTP handler not found") => Some(Stale::NoHandler),
        _ => None,
    }
}

/// The app's description as text; a 404 means the app is not installed.
pub async fn fetch_document(client: &Client, app: &AppEntry) -> Result<String> {
    let path = format!("extensionEndpoints/{}/meta/openapi.json", app.id);
    match client.raw(Method::GET, &path, None).await {
        Ok(Some(text)) => Ok(text),
        Ok(None) => Err(YtError::AppDocument {
            id: app.id.to_string(),
            message: "it is empty".into(),
        }
        .into()),
        Err(error)
            if matches!(
                error.downcast_ref::<YtError>(),
                Some(YtError::Api { status: 404, .. })
            ) =>
        {
            Err(YtError::AppNotInstalled {
                id: app.id.to_string(),
                url: client.instance().to_string(),
                help: Some(format!(
                    "an administrator can install it from {}; an installed version may also be older than the meta/openapi.json convention",
                    app.homepage
                )),
            }
            .into())
        }
        Err(error) => Err(error),
    }
}

/// The app's commands: from the cache while it is fresh, unless `refresh`, otherwise fetched and cached.
pub async fn load(
    client: &Client,
    cache: &AppCache,
    app: &AppEntry,
    refresh: bool,
    taken: &[&str],
) -> Result<AppApi> {
    if !refresh && let Some(text) = cache.load(app.id) {
        match AppApi::parse(app.id, &text, taken) {
            Ok(api) => return Ok(api),
            Err(error) => log::debug!("ignoring the cached description of {}: {error}", app.id),
        }
    }
    let text = match fetch_document(client, app).await {
        Ok(text) => text,
        Err(error) => {
            if matches!(
                error.downcast_ref::<YtError>(),
                Some(YtError::AppNotInstalled { .. })
            ) {
                cache.forget(app.id);
            }
            return Err(error);
        }
    };
    let api = AppApi::parse(app.id, &text, taken)?;
    cache.store(app.id, &text);
    Ok(api)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::api::test_client;

    const META: &str = "/api/extensionEndpoints/hidden-fields/meta/openapi.json";

    fn app() -> &'static AppEntry {
        registry::find("hidden-fields").unwrap()
    }

    fn description() -> &'static str {
        include_str!("../../tests/fixtures/apps/hidden-fields.openapi.json")
    }

    #[tokio::test]
    async fn a_description_is_fetched_once_and_then_cached() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(META))
            .respond_with(
                ResponseTemplate::new(200).set_body_raw(description(), "application/json"),
            )
            .expect(1)
            .mount(&server)
            .await;
        let dir = tempfile::tempdir().unwrap();
        let cache = AppCache::new(dir.path(), "acme", Duration::from_secs(60));
        let client = test_client(&server.uri());
        let first = load(&client, &cache, app(), false, &[]).await.unwrap();
        let second = load(&client, &cache, app(), false, &[]).await.unwrap();
        assert_eq!(first, second);
        assert_eq!(first.operations.len(), 2);
    }

    #[tokio::test]
    async fn a_missing_app_is_not_installed_and_leaves_no_cache() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(META))
            .respond_with(ResponseTemplate::new(404).set_body_raw(
                r#"{"error":"Not Found","error_description":"HTTP handler not found by path hidden-fields/meta/openapi.json"}"#,
                "application/json",
            ))
            .mount(&server)
            .await;
        let dir = tempfile::tempdir().unwrap();
        let cache = AppCache::new(dir.path(), "acme", Duration::from_secs(60));
        cache.store("hidden-fields", description());
        let error = load(&test_client(&server.uri()), &cache, app(), true, &[])
            .await
            .unwrap_err();
        assert!(
            matches!(
                error.downcast_ref::<YtError>(),
                Some(YtError::AppNotInstalled { .. })
            ),
            "{error:?}"
        );
        assert_eq!(cache.load_stale("hidden-fields"), None);
    }

    #[test]
    fn stale_answers_are_recognised() {
        let api = |status: u16, body: &str| {
            crate::api::error::api_error(
                &Method::GET,
                "/api/x",
                reqwest::StatusCode::from_u16(status).unwrap(),
                body,
            )
        };
        assert_eq!(
            stale(&api(
                403,
                r#"{"error":"disabled","error_description":"off"}"#
            )),
            Some(Stale::Disabled)
        );
        assert_eq!(
            stale(&api(
                404,
                r#"{"error":"Not Found","error_description":"HTTP handler not found by path hidden-fields/api/issue"}"#
            )),
            Some(Stale::NoHandler)
        );
        assert_eq!(
            stale(&api(
                404,
                r#"{"error":"not_found","error_description":"Issue \"X-1\" was not found"}"#
            )),
            None
        );
        assert_eq!(
            stale(&api(
                403,
                r#"{"error":"forbidden","error_description":"no"}"#
            )),
            None
        );
    }
}
