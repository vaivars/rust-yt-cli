use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use futures::future::BoxFuture;
use futures::{Stream, TryStreamExt as _};
use reqwest::header::{ACCEPT, AUTHORIZATION, HeaderValue};
use reqwest::{Method, StatusCode};
use secrecy::{ExposeSecret as _, SecretString};
use serde::de::DeserializeOwned;
use serde_json::Value;
use url::Url;

use crate::api::error::{api_error, unexpected};
use crate::api::projection::Projection;
use crate::error::{Result, YtError};
use crate::http::{is_transient, retry_delay};
use crate::instance::InstanceUrl;

/// Uploads and downloads take as long as the file needs; `http_timeout` would cut them off.
const TRANSFER_TIMEOUT: Duration = Duration::from_secs(60 * 60);
pub(crate) const LIST_PAGE: usize = 500;

/// Supplies the bearer credential for requests.
pub trait TokenSource: Send + Sync {
    fn bearer(&self) -> BoxFuture<'_, Result<SecretString>>;
    /// Called once after a 401; `Ok(true)` means a fresh credential is ready and the request is retried.
    fn refresh_after_unauthorized(&self) -> BoxFuture<'_, Result<bool>>;
    /// The profile name shown in errors.
    fn label(&self) -> &str;
}

pub struct Client {
    http: reqwest::Client,
    instance: InstanceUrl,
    api: Url,
    auth: Arc<dyn TokenSource>,
    retries: u32,
    written: AtomicBool,
}

struct Response {
    status: StatusCode,
    body: String,
}

#[derive(Clone, Copy)]
struct Page {
    skip: usize,
    size: usize,
    done: bool,
}

impl Client {
    pub fn new(
        http: reqwest::Client,
        instance: InstanceUrl,
        auth: Arc<dyn TokenSource>,
        retries: u32,
    ) -> Self {
        let api = instance.api();
        Self {
            http,
            instance,
            api,
            auth,
            retries,
            written: AtomicBool::new(false),
        }
    }

    /// Whether this client sent a change the server may have applied (a non-GET request it answered, or one that timed out).
    pub fn has_written(&self) -> bool {
        self.written.load(Ordering::SeqCst)
    }

    pub fn instance(&self) -> &InstanceUrl {
        &self.instance
    }

    /// GETs `path` (relative to `/api/`) with `query`, adding `T::FIELDS`.
    pub async fn get<T: Projection>(&self, path: &str, query: &[(&str, &str)]) -> Result<T> {
        let url = self.url(path, query, Some(T::FIELDS))?;
        let response = self.execute(Method::GET, url.clone(), None, true).await?;
        let body = self.success(&Method::GET, &url, response)?;
        decode(&url, &body)
    }

    /// POSTs `body` to `path` (relative to `/api/`) and decodes the answer, asking for `T::FIELDS`. A write: never retried.
    pub async fn post<T: Projection>(&self, path: &str, body: &Value) -> Result<T> {
        let url = self.url(path, &[], Some(T::FIELDS))?;
        let response = self
            .execute(Method::POST, url.clone(), Some(body), false)
            .await?;
        let text = self.success(&Method::POST, &url, response)?;
        decode(&url, &text)
    }

    /// A POST that only reads (`commands/assist`): retried like a GET and not counted as a write.
    pub async fn post_query<T: Projection>(&self, path: &str, body: &Value) -> Result<T> {
        let url = self.url(path, &[], Some(T::FIELDS))?;
        let response = self
            .execute(Method::POST, url.clone(), Some(body), true)
            .await?;
        let text = self.success(&Method::POST, &url, response)?;
        decode(&url, &text)
    }

    /// POSTs `body` to `path` and ignores the answer.
    pub async fn post_unit(&self, path: &str, body: &Value) -> Result<()> {
        let url = self.url(path, &[], None)?;
        let response = self
            .execute(Method::POST, url.clone(), Some(body), false)
            .await?;
        self.success(&Method::POST, &url, response).map(drop)
    }

    pub async fn delete(&self, path: &str) -> Result<()> {
        let url = self.url(path, &[], None)?;
        let response = self
            .execute(Method::DELETE, url.clone(), None, false)
            .await?;
        self.success(&Method::DELETE, &url, response).map(drop)
    }

    /// Uploads files in one multipart POST to `path` and decodes the answer. A write: never retried.
    pub async fn upload<T: Projection>(
        &self,
        path: &str,
        files: &[(String, Vec<u8>)],
    ) -> Result<T> {
        let url = self.url(path, &[], Some(T::FIELDS))?;
        let response = self
            .send_once(Method::POST, url.clone(), |request| {
                let mut form = reqwest::multipart::Form::new();
                for (name, data) in files {
                    form = form.part(
                        "file",
                        reqwest::multipart::Part::bytes(data.clone()).file_name(name.clone()),
                    );
                }
                request.multipart(form)
            })
            .await?;
        let status = response.status();
        let body = response.text().await.map_err(network)?;
        let text = self.success(&Method::POST, &url, Response { status, body })?;
        decode(&url, &text)
    }

    /// GETs a file URL YouTrack handed out (relative to the instance) and returns the response for streaming.
    pub async fn download(&self, url: &str) -> Result<reqwest::Response> {
        let url = self.instance.join(url);
        let response = self
            .send_once(Method::GET, url.clone(), |request| request)
            .await?;
        let status = response.status();
        if status.is_success() {
            return Ok(response);
        }
        let body = response.text().await.unwrap_or_default();
        Err(self.failure(&Method::GET, &url, status, &body))
    }

    /// One request with the credential, refreshed once after a 401 and never retried; for non-JSON bodies.
    async fn send_once(
        &self,
        method: Method,
        url: Url,
        build: impl Fn(reqwest::RequestBuilder) -> reqwest::RequestBuilder,
    ) -> Result<reqwest::Response> {
        self.check_origin(&url)?;
        let mut refreshed = false;
        loop {
            let request = self
                .http
                .request(method.clone(), url.clone())
                .header(AUTHORIZATION, self.authorization().await?);
            let response = match build(request).timeout(TRANSFER_TIMEOUT).send().await {
                Ok(response) => response,
                Err(error) => {
                    if method != Method::GET && !error.is_connect() {
                        self.written.store(true, Ordering::SeqCst);
                    }
                    return Err(network(error));
                }
            };
            if response.status() == StatusCode::UNAUTHORIZED && !refreshed {
                refreshed = true;
                if self.auth.refresh_after_unauthorized().await? {
                    continue;
                }
            }
            if method != Method::GET && response.status() != StatusCode::UNAUTHORIZED {
                self.written.store(true, Ordering::SeqCst);
            }
            return Ok(response);
        }
    }

    fn check_origin(&self, url: &Url) -> Result<()> {
        if url.origin() == self.api.origin() {
            return Ok(());
        }
        Err(YtError::Usage {
            message: format!(
                "refusing to send the credential to {}: it is not {}",
                url.origin().ascii_serialization(),
                self.instance
            ),
        }
        .into())
    }

    async fn authorization(&self) -> Result<HeaderValue> {
        let token = self.auth.bearer().await?;
        let mut value = HeaderValue::from_str(&format!("Bearer {}", token.expose_secret()))
            .map_err(|_| YtError::Usage {
                message: "the credential contains characters that cannot be sent".into(),
            })?;
        value.set_sensitive(true);
        Ok(value)
    }

    /// Sends a request as given and returns YouTrack's JSON text, `None` for an empty body. Only GET is retried.
    pub async fn raw(
        &self,
        method: Method,
        path_and_query: &str,
        body: Option<&Value>,
    ) -> Result<Option<String>> {
        let relative = path_and_query.trim_start_matches('/');
        let relative = relative.strip_prefix("api/").unwrap_or(relative);
        let url = self.api.join(relative).map_err(|error| YtError::Usage {
            message: format!("invalid API path '{path_and_query}': {error}"),
        })?;
        let idempotent = method == Method::GET;
        let response = self
            .execute(method.clone(), url.clone(), body, idempotent)
            .await?;
        let text = self.success(&method, &url, response)?;
        if text.trim().is_empty() {
            return Ok(None);
        }
        if serde_json::from_str::<serde::de::IgnoredAny>(&text).is_err() {
            return Err(YtError::Api {
                method: method.to_string(),
                path: url.path().to_string(),
                status: 200,
                code: None,
                message: unexpected(StatusCode::OK, &text),
                help: None,
            }
            .into());
        }
        Ok(Some(text))
    }

    /// Every item of a collection, read in pages of `LIST_PAGE`.
    pub async fn get_all<T>(&self, path: &str) -> Result<Vec<T>>
    where
        T: Projection + Send + 'static,
    {
        self.paginate(path.to_string(), Vec::new(), LIST_PAGE)
            .try_collect()
            .await
    }

    /// Pages through a collection with `$skip`/`$top`, shrinking the page if the instance caps its size.
    pub fn paginate<T>(
        &self,
        path: String,
        query: Vec<(String, String)>,
        page_size: usize,
    ) -> impl Stream<Item = Result<T>> + Send + '_
    where
        T: Projection + Send + 'static,
    {
        let start = Page {
            skip: 0,
            size: page_size.max(1),
            done: false,
        };
        futures::stream::try_unfold(start, move |page| {
            let path = path.clone();
            let query = query.clone();
            async move {
                self.next_page::<T>(&path, &query, page).await.map(|next| {
                    next.map(|(items, page)| {
                        (
                            futures::stream::iter(items.into_iter().map(Ok::<T, miette::Report>)),
                            page,
                        )
                    })
                })
            }
        })
        .try_flatten()
    }

    async fn next_page<T: Projection>(
        &self,
        path: &str,
        query: &[(String, String)],
        mut page: Page,
    ) -> Result<Option<(Vec<T>, Page)>> {
        if page.done {
            return Ok(None);
        }
        loop {
            let skip = page.skip.to_string();
            let top = page.size.to_string();
            let mut pairs: Vec<(&str, &str)> = query
                .iter()
                .map(|(key, value)| (key.as_str(), value.as_str()))
                .collect();
            pairs.push(("$skip", &skip));
            pairs.push(("$top", &top));
            let url = self.url(path, &pairs, Some(T::FIELDS))?;
            let response = self.execute(Method::GET, url.clone(), None, true).await?;
            if response.status == StatusCode::BAD_REQUEST
                && let Some(limit) = export_limit(&response.body).filter(|limit| *limit < page.size)
            {
                page.size = limit;
                continue;
            }
            let body = self.success(&Method::GET, &url, response)?;
            let items: Vec<T> = decode(&url, &body)?;
            page.skip += items.len();
            page.done = items.len() < page.size;
            return Ok(Some((items, page)));
        }
    }

    fn url(&self, path: &str, query: &[(&str, &str)], fields: Option<&str>) -> Result<Url> {
        let mut url = self.api.join(path).map_err(|error| YtError::Usage {
            message: format!("invalid API path '{path}': {error}"),
        })?;
        if !query.is_empty() || fields.is_some() {
            let mut pairs = url.query_pairs_mut();
            for (key, value) in query {
                pairs.append_pair(key, value);
            }
            if let Some(fields) = fields {
                pairs.append_pair("fields", fields);
            }
        }
        Ok(url)
    }

    async fn execute(
        &self,
        method: Method,
        url: Url,
        body: Option<&Value>,
        idempotent: bool,
    ) -> Result<Response> {
        self.check_origin(&url)?;
        let mut attempt = 0;
        let mut refreshed = false;
        loop {
            let authorization = self.authorization().await?;
            let mut request = self
                .http
                .request(method.clone(), url.clone())
                .header(AUTHORIZATION, authorization)
                .header(ACCEPT, "application/json");
            if let Some(body) = body {
                request = request.json(body);
            }
            let started = Instant::now();
            match request.send().await {
                Ok(response) => {
                    let status = response.status();
                    log::debug!(
                        "{method} {} -> {} in {} ms",
                        url.path(),
                        status.as_u16(),
                        started.elapsed().as_millis()
                    );
                    log::trace!("query: {}", url.query().unwrap_or(""));
                    if status == StatusCode::UNAUTHORIZED && !refreshed {
                        refreshed = true;
                        if self.auth.refresh_after_unauthorized().await? {
                            continue;
                        }
                    }
                    if !idempotent && status != StatusCode::UNAUTHORIZED {
                        self.written.store(true, Ordering::SeqCst);
                    }
                    if idempotent && is_transient(status) && attempt < self.retries {
                        tokio::time::sleep(retry_delay(attempt, Some(response.headers()))).await;
                        attempt += 1;
                        continue;
                    }
                    let body = response.text().await.map_err(network)?;
                    return Ok(Response { status, body });
                }
                Err(error)
                    if idempotent
                        && attempt < self.retries
                        && (error.is_connect() || error.is_timeout()) =>
                {
                    log::debug!("{method} {} failed, retrying: {error}", url.path());
                    tokio::time::sleep(retry_delay(attempt, None)).await;
                    attempt += 1;
                }
                Err(error) => {
                    if !idempotent && !error.is_connect() {
                        self.written.store(true, Ordering::SeqCst);
                    }
                    return Err(network(error));
                }
            }
        }
    }

    fn success(&self, method: &Method, url: &Url, response: Response) -> Result<String> {
        if response.status.is_success() {
            return Ok(response.body);
        }
        Err(self.failure(method, url, response.status, &response.body))
    }

    fn failure(
        &self,
        method: &Method,
        url: &Url,
        status: StatusCode,
        body: &str,
    ) -> miette::Report {
        if status == StatusCode::UNAUTHORIZED {
            return YtError::CredentialRejected {
                profile: self.auth.label().to_string(),
            }
            .into();
        }
        api_error(method, url.path(), status, body)
    }
}

fn decode<T: DeserializeOwned>(url: &Url, body: &str) -> Result<T> {
    let mut deserializer = serde_json::Deserializer::from_str(body);
    serde_path_to_error::deserialize(&mut deserializer).map_err(|error| {
        let message = if body.trim_start().starts_with('<') {
            unexpected(StatusCode::OK, body)
        } else {
            format!(
                "could not read the response at {}: {}",
                error.path(),
                error.inner()
            )
        };
        YtError::Api {
            method: "GET".into(),
            path: url.path().to_string(),
            status: 200,
            code: None,
            message,
            help: None,
        }
        .into()
    })
}

fn export_limit(body: &str) -> Option<usize> {
    let rest = body.split("greater than limit").nth(1)?;
    rest.trim_start()
        .chars()
        .take_while(char::is_ascii_digit)
        .collect::<String>()
        .parse()
        .ok()
}

fn network(error: reqwest::Error) -> miette::Report {
    YtError::Network(crate::http::describe(&error)).into()
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use serde::Deserialize;
    use serde_json::json;
    use wiremock::matchers::{body_json, header, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::http::HttpConfig;

    const TOKEN: &str = "perm-dGVzdA==.Y2xpZW50.secret123456";

    struct Fixed {
        refreshes: AtomicUsize,
    }

    impl TokenSource for Fixed {
        fn bearer(&self) -> BoxFuture<'_, Result<SecretString>> {
            Box::pin(async { Ok(SecretString::from(TOKEN.to_string())) })
        }
        fn refresh_after_unauthorized(&self) -> BoxFuture<'_, Result<bool>> {
            self.refreshes.fetch_add(1, Ordering::SeqCst);
            Box::pin(async { Ok(false) })
        }
        fn label(&self) -> &str {
            "acme"
        }
    }

    #[derive(Debug, Deserialize, PartialEq)]
    #[serde(rename_all = "camelCase")]
    struct Item {
        id_readable: String,
    }

    impl Projection for Item {
        const FIELDS: &'static str = "idReadable";
    }

    fn client_for(server: &MockServer, auth: Arc<Fixed>) -> Client {
        let http = crate::http::client(&HttpConfig {
            timeout: Duration::from_secs(5),
            retries: 3,
            ca_file: None,
        })
        .unwrap();
        Client::new(http, InstanceUrl::parse(&server.uri()).unwrap(), auth, 3)
    }

    fn fixed() -> Arc<Fixed> {
        Arc::new(Fixed {
            refreshes: AtomicUsize::new(0),
        })
    }

    fn item(id: &str) -> serde_json::Value {
        json!({"idReadable": id, "$type": "Issue"})
    }

    #[tokio::test]
    async fn get_sends_fields_and_the_token() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/issues/WEB-1"))
            .and(query_param("fields", "idReadable"))
            .and(header("authorization", format!("Bearer {TOKEN}").as_str()))
            .respond_with(ResponseTemplate::new(200).set_body_json(item("WEB-1")))
            .expect(1)
            .mount(&server)
            .await;
        let found: Item = client_for(&server, fixed())
            .get("issues/WEB-1", &[])
            .await
            .unwrap();
        assert_eq!(
            found,
            Item {
                id_readable: "WEB-1".into()
            }
        );
    }

    #[tokio::test]
    async fn a_stopped_redirect_says_why() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(|request: &wiremock::Request| {
                ResponseTemplate::new(302)
                    .insert_header("location", format!("{}/next", request.url.path()))
            })
            .mount(&server)
            .await;
        let result: Result<Item> = client_for(&server, fixed()).get("issues/WEB-1", &[]).await;
        let error = result.unwrap_err().to_string();
        assert!(error.contains("too many redirects"), "{error}");
    }

    #[tokio::test]
    async fn unauthorized_asks_for_a_refresh_once_and_never_shows_the_token() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(401).set_body_json(
                json!({"error": "Unauthorized", "error_description": "Invalid token"}),
            ))
            .mount(&server)
            .await;
        let auth = fixed();
        let error = client_for(&server, auth.clone())
            .get::<Item>("issues/WEB-1", &[])
            .await
            .unwrap_err();
        assert_eq!(crate::error::exit_code(&error), 4);
        assert_eq!(auth.refreshes.load(Ordering::SeqCst), 1);
        assert!(!format!("{error:?}").contains(TOKEN));
    }

    #[tokio::test]
    async fn a_rejected_download_asks_to_sign_in_again() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&server)
            .await;
        let auth = fixed();
        let error = client_for(&server, auth.clone())
            .download("/api/files/8-1?sign=x")
            .await
            .unwrap_err();
        assert_eq!(crate::error::exit_code(&error), 4);
        assert_eq!(auth.refreshes.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn reads_are_retried_on_503() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(503).insert_header("retry-after", "0"))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_json(item("WEB-1")))
            .mount(&server)
            .await;
        let found: Item = client_for(&server, fixed())
            .get("issues/WEB-1", &[])
            .await
            .unwrap();
        assert_eq!(found.id_readable, "WEB-1");
    }

    #[tokio::test]
    async fn writes_are_never_retried() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(503).insert_header("retry-after", "0"))
            .expect(1)
            .mount(&server)
            .await;
        let result = client_for(&server, fixed())
            .raw(Method::POST, "commands", Some(&json!({"query": "x"})))
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn html_error_pages_are_reported_as_such() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(
                ResponseTemplate::new(502)
                    .set_body_raw("<html><body>Bad gateway</body></html>", "text/html"),
            )
            .mount(&server)
            .await;
        let auth = fixed();
        let http = crate::http::client(&HttpConfig {
            timeout: Duration::from_secs(5),
            retries: 0,
            ca_file: None,
        })
        .unwrap();
        let client = Client::new(http, InstanceUrl::parse(&server.uri()).unwrap(), auth, 0);
        let error = client.get::<Item>("issues/WEB-1", &[]).await.unwrap_err();
        assert!(
            error
                .to_string()
                .contains("unexpected HTML response (HTTP 502"),
            "{error}"
        );
    }

    #[tokio::test]
    async fn html_with_status_200_is_not_a_decode_panic() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(
                ResponseTemplate::new(200).set_body_raw("<html>Sign in</html>", "text/html"),
            )
            .mount(&server)
            .await;
        let error = client_for(&server, fixed())
            .get::<Item>("issues/WEB-1", &[])
            .await
            .unwrap_err();
        assert!(
            error.to_string().contains("unexpected HTML response"),
            "{error}"
        );
    }

    #[tokio::test]
    async fn youtrack_errors_are_shown_verbatim() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(400).set_body_json(json!({
                "error": "invalid_query",
                "error_description": "Can't parse search query",
                "error_children": [{"error": "The value \"NOPE\" isn't used for the project field."}]
            })))
            .mount(&server)
            .await;
        let error = client_for(&server, fixed())
            .get::<Item>("issues", &[])
            .await
            .unwrap_err();
        let text = error.to_string();
        assert!(
            text.contains("Can't parse search query")
                && text.contains("isn't used for the project field"),
            "{text}"
        );
        assert_eq!(crate::error::exit_code(&error), 1);
    }

    #[tokio::test]
    async fn decode_errors_name_the_json_path() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"idReadable": 5})))
            .mount(&server)
            .await;
        let error = client_for(&server, fixed())
            .get::<Item>("issues/WEB-1", &[])
            .await
            .unwrap_err();
        assert!(error.to_string().contains("idReadable"), "{error}");
    }

    #[tokio::test]
    async fn paginate_walks_pages_until_a_short_one() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/issues"))
            .and(query_param("$skip", "0"))
            .and(query_param("$top", "2"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!([item("WEB-1"), item("WEB-2")])),
            )
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/issues"))
            .and(query_param("$skip", "2"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([item("WEB-3")])))
            .expect(1)
            .mount(&server)
            .await;
        let client = client_for(&server, fixed());
        let items: Vec<Item> = client
            .paginate("issues".into(), vec![], 2)
            .try_collect()
            .await
            .unwrap();
        assert_eq!(items.len(), 3);
    }

    #[tokio::test]
    async fn paginate_shrinks_to_the_export_limit() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(query_param("$top", "100"))
            .respond_with(ResponseTemplate::new(400).set_body_json(json!({
                "error": "Bad Request",
                "error_description": "Requested size 100 is greater than limit 50"
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(query_param("$top", "50"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([item("WEB-1")])))
            .mount(&server)
            .await;
        let client = client_for(&server, fixed());
        let items: Vec<Item> = client
            .paginate("issues".into(), vec![], 100)
            .try_collect()
            .await
            .unwrap();
        assert_eq!(items.len(), 1);
    }

    #[tokio::test]
    async fn raw_accepts_paths_with_or_without_the_api_prefix() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/admin/projects"))
            .and(query_param("$top", "1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
            .expect(2)
            .mount(&server)
            .await;
        let client = client_for(&server, fixed());
        assert_eq!(
            client
                .raw(Method::GET, "/api/admin/projects?$top=1", None)
                .await
                .unwrap()
                .as_deref(),
            Some("[]")
        );
        assert_eq!(
            client
                .raw(Method::GET, "admin/projects?$top=1", None)
                .await
                .unwrap()
                .as_deref(),
            Some("[]")
        );
    }

    #[tokio::test]
    async fn the_token_never_leaves_the_instance() {
        let instance = MockServer::start().await;
        let other = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
            .mount(&other)
            .await;
        let client = client_for(&instance, fixed());
        let other_host = other.address().to_string();
        for path in [
            format!("{}/steal", other.uri()),
            format!("\\\\{other_host}/steal"),
        ] {
            let error = client.raw(Method::GET, &path, None).await.unwrap_err();
            assert_eq!(crate::error::exit_code(&error), 2, "{path}: {error}");
        }
        assert!(other.received_requests().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn only_changes_the_server_accepted_count_as_writes() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/api/rejected"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/api/accepted"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
            .mount(&server)
            .await;
        let client = client_for(&server, fixed());
        client.raw(Method::GET, "issues", None).await.unwrap();
        assert!(!client.has_written());
        client
            .raw(Method::POST, "rejected", Some(&json!({})))
            .await
            .unwrap_err();
        assert!(!client.has_written());
        client
            .raw(Method::POST, "accepted", Some(&json!({})))
            .await
            .unwrap();
        assert!(client.has_written());
    }

    #[tokio::test]
    async fn post_sends_the_body_and_decodes_the_answer() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/issues"))
            .and(query_param("fields", "idReadable"))
            .and(body_json(json!({"summary": "x"})))
            .respond_with(ResponseTemplate::new(200).set_body_json(item("WEB-9")))
            .expect(1)
            .mount(&server)
            .await;
        let client = client_for(&server, fixed());
        let created: Item = client
            .post("issues", &json!({"summary": "x"}))
            .await
            .unwrap();
        assert_eq!(created.id_readable, "WEB-9");
        assert!(client.has_written());
    }

    #[tokio::test]
    async fn deletes_and_unit_posts_are_never_retried() {
        let server = MockServer::start().await;
        Mock::given(method("DELETE"))
            .respond_with(ResponseTemplate::new(503))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(503))
            .expect(1)
            .mount(&server)
            .await;
        let client = client_for(&server, fixed());
        assert!(client.delete("issues/WEB-1").await.is_err());
        assert!(client.post_unit("issues/WEB-1", &json!({})).await.is_err());
    }

    #[tokio::test]
    async fn a_read_only_post_is_not_a_write() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/commands/assist"))
            .respond_with(ResponseTemplate::new(200).set_body_json(item("WEB-1")))
            .mount(&server)
            .await;
        let client = client_for(&server, fixed());
        let _: Item = client
            .post_query("commands/assist", &json!({}))
            .await
            .unwrap();
        assert!(!client.has_written());
    }
}
