use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use secrecy::{ExposeSecret as _, SecretString};

use crate::api::Client;
use crate::auth::discovery::{Discovery, discover};
use crate::auth::loopback::{self, Callback};
use crate::auth::oauth::{self, AuthorizeRequest};
use crate::auth::pkce::{Pkce, random_token};
use crate::auth::token_source::StaticToken;
use crate::config::file::{AuthKind, ConfigStore, GlobalConfig, Profile};
use crate::credentials::{self, OauthCredential, StoredCredential};
use crate::env::Env;
use crate::error::{Result, YtError};
use crate::http::HttpConfig;
use crate::instance::InstanceUrl;
use crate::ui::{Choice, Ui};

/// The redirect port named when no listener runs (paste-back); any port is accepted on a registered loopback URI.
const PASTE_PORT: u16 = 8765;

/// yt's client ID where an instance allows automatic OAuth client registration (CIMD): the URL of its client metadata document, served from the `docs` branch. The URL must never change.
pub const CIMD_CLIENT_ID: &str = "https://vaivars.github.io/rust-yt-cli/oauth-client.json";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Method {
    Browser,
    Token,
}

#[derive(Default)]
pub struct LoginRequest {
    pub url: Option<String>,
    pub method: Option<Method>,
    pub client_id: Option<String>,
    pub profile: Option<String>,
    pub token: Option<SecretString>,
    pub paste: bool,
    pub revocable: bool,
}

pub struct LoginEnv<'a> {
    pub env: &'a Env,
    pub ui: &'a Ui,
    pub store: &'a ConfigStore,
    pub http: &'a HttpConfig,
    pub open_browser: &'a (dyn Fn(&str) -> bool + Send + Sync),
}

#[derive(Debug)]
pub struct LoginOutcome {
    pub profile: String,
    pub url: InstanceUrl,
    pub login: String,
}

#[derive(Clone, Copy)]
enum Next {
    ClientId,
    Token,
    Cancel,
}

/// Signs in: choose a method, find the instance, authenticate, then save the credential and profile.
pub async fn login(le: &LoginEnv<'_>, request: LoginRequest) -> Result<LoginOutcome> {
    let LoginRequest {
        url,
        method,
        client_id,
        profile,
        token,
        paste,
        revocable,
    } = request;
    let config = le.store.load()?;
    let mut method = match (token.is_some(), method) {
        (true, _) => Method::Token,
        (false, Some(method)) => method,
        (false, None) => {
            le.ui
                .select("How do you want to sign in?", method_choices(), "--method")
                .await?
        }
    };
    if method == Method::Browser && !le.ui.is_interactive() {
        return Err(YtError::Usage {
            message: "browser sign-in needs a terminal; use --method token --token-stdin, or set YOUTRACK_URL and YOUTRACK_TOKEN".into(),
        }
        .into());
    }
    let http = crate::http::client(le.http)?;
    let discovery = discover_with_prompt(le, &http, url).await?;
    if discovery.url.is_unencrypted() {
        log::warn!(
            "{} uses plain http: the sign-in and everything yt sends cross the network unencrypted",
            discovery.url
        );
    }
    check_requested(&config, profile.as_deref(), &discovery.url, None)?;
    let mut client_id = client_id
        .or_else(|| client_id_for(&config, &discovery.url))
        .or_else(|| discovery.cimd_supported.then(|| CIMD_CLIENT_ID.to_string()));
    if method == Method::Browser && client_id.is_none() {
        match ask_for_client(le, &discovery).await? {
            Some(id) => client_id = Some(id),
            None => method = Method::Token,
        }
    }
    let (login, credential) = match (method, client_id) {
        (Method::Browser, Some(client_id)) => {
            browser_login(le, &http, &discovery, &client_id, paste, revocable).await?
        }
        _ => token_login(le, &http, &discovery, token).await?,
    };
    check_requested(&config, profile.as_deref(), &discovery.url, Some(&login))?;
    let name = profile_name(le, &config, &discovery.url, &login, profile).await?;
    let account = discovery.url.account(&login);
    if revocable {
        revoke_previous(&http, &discovery, &account, &credential).await;
    }
    credentials::save(&account, &credential).await?;
    let previous = config.profiles.get(&name);
    let first_for_url = !config.profiles.iter().any(|(other, existing)| {
        other != &name && InstanceUrl::parse(&existing.url).ok().as_ref() == Some(&discovery.url)
    });
    let oauth = match &credential {
        StoredCredential::Oauth(oauth) => Some(oauth),
        StoredCredential::Token { .. } => None,
    };
    let profile = Profile {
        url: discovery.url.to_string(),
        login: Some(login.clone()),
        auth: if oauth.is_some() {
            AuthKind::Oauth
        } else {
            AuthKind::Token
        },
        project: previous.and_then(|existing| existing.project.clone()),
        default_for_url: first_for_url || previous.is_some_and(|existing| existing.default_for_url),
        oauth_client_id: oauth
            .map(|oauth| oauth.client_id.clone())
            .or_else(|| previous.and_then(|existing| existing.oauth_client_id.clone())),
        ca_file: le.http.ca_file.clone(),
        revocable: revocable && oauth.is_some(),
        apps: previous
            .map(|existing| existing.apps.clone())
            .unwrap_or_default(),
    };
    le.store.set_profile(&name, &profile)?;
    if config.default_profile.is_none() {
        le.store.set_default_profile(&name)?;
    }
    Ok(LoginOutcome {
        profile: name,
        url: discovery.url,
        login,
    })
}

fn method_choices() -> Vec<Choice<Method>> {
    vec![
        Choice {
            value: Method::Browser,
            label: "Browser sign-in (recommended)".into(),
            description: Some("approve access in your browser".into()),
        },
        Choice {
            value: Method::Token,
            label: "Permanent token".into(),
            description: Some("create a token in your YouTrack profile and paste it".into()),
        },
    ]
}

async fn discover_with_prompt(
    le: &LoginEnv<'_>,
    http: &reqwest::Client,
    url: Option<String>,
) -> Result<Discovery> {
    let mut given = url;
    loop {
        let input = match given.take() {
            Some(url) => url,
            None => {
                le.ui
                    .input(
                        "YouTrack URL",
                        Some("the address you open YouTrack at"),
                        None,
                        "<URL>",
                    )
                    .await?
            }
        };
        let result = match InstanceUrl::parse(&input) {
            Ok(url) => {
                le.ui
                    .with_spinner("Checking the instance", discover(http, &url))
                    .await
            }
            Err(error) => Err(error),
        };
        match result {
            Ok(discovery) => return Ok(discovery),
            Err(error) if le.ui.is_interactive() => crate::ui::eprint_line(&error.to_string()),
            Err(error) => return Err(error),
        }
    }
}

/// Explains what an administrator must set up; `Some(id)` to continue in the browser, `None` to use a token.
async fn ask_for_client(le: &LoginEnv<'_>, discovery: &Discovery) -> Result<Option<String>> {
    crate::ui::eprint_line(&client_setup(&discovery.url));
    let choices = vec![
        Choice {
            value: Next::ClientId,
            label: "Enter a client ID from your administrator".into(),
            description: None,
        },
        Choice {
            value: Next::Token,
            label: "Use a permanent token instead".into(),
            description: None,
        },
        Choice {
            value: Next::Cancel,
            label: "Cancel".into(),
            description: None,
        },
    ];
    match le
        .ui
        .select("How do you want to continue?", choices, "--client-id")
        .await?
    {
        Next::ClientId => Ok(Some(
            le.ui.input("Client ID", None, None, "--client-id").await?,
        )),
        Next::Token => Ok(None),
        Next::Cancel => Err(YtError::Cancelled.into()),
    }
}

/// What an administrator sets up so yt can sign in through the browser.
fn client_setup(url: &InstanceUrl) -> String {
    format!(
        "Browser sign-in needs an OAuth client for yt on {url}. An administrator sets it up under Administration › Access Management › OAuth Clients, either way:\n- turn on \"Allow automatic OAuth client registration via CIMD\", and yt registers itself, or\n- create a client: Authorization Code with PKCE, no client secret, redirect URI {} (any port works).\n",
        loopback::redirect_uri(PASTE_PORT)
    )
}

async fn browser_login(
    le: &LoginEnv<'_>,
    http: &reqwest::Client,
    discovery: &Discovery,
    client_id: &str,
    paste: bool,
    revocable: bool,
) -> Result<(String, StoredCredential)> {
    let pkce = Pkce::generate();
    let state = random_token(16);
    let scope = oauth::scope(&discovery.service_id, revocable);
    let paste = paste || !crate::browser::can_launch(le.env);
    let callback = if paste {
        None
    } else {
        Some(Callback::bind().await?)
    };
    let redirect_uri = callback.as_ref().map_or_else(
        || loopback::redirect_uri(PASTE_PORT),
        Callback::redirect_uri,
    );
    let authorize = oauth::authorize_url(
        discovery,
        &AuthorizeRequest {
            client_id,
            redirect_uri: &redirect_uri,
            scope: &scope,
            state: &state,
            challenge: &pkce.challenge,
        },
    );
    oauth::preflight(
        &crate::http::client_without_redirects(le.http)?,
        discovery,
        &authorize,
    )
    .await?;
    crate::ui::eprint_line(&format!("Open this address to sign in:\n{authorize}\n"));
    let code = match callback {
        Some(callback) => {
            (le.open_browser)(authorize.as_str());
            le.ui
                .with_spinner(
                    "Waiting for the browser",
                    callback.wait(&state, Duration::from_secs(300)),
                )
                .await?
        }
        None => {
            let pasted = le
                .ui
                .input(
                    "Address of the page your browser landed on",
                    Some("after you approve, that page fails to load; copy its address from the address bar"),
                    None,
                    "a terminal",
                )
                .await?;
            loopback::code_from_pasted(&pasted, &state)?
        }
    };
    let tokens = oauth::exchange_code(
        http,
        discovery,
        client_id,
        &code,
        &redirect_uri,
        &pkce.verifier,
    )
    .await?;
    let refresh_token = tokens.refresh_token.ok_or_else(|| YtError::OAuth {
        message: "Hub issued no refresh token".into(),
        help: None,
    })?;
    let access_token = tokens.access_token.expose_secret().to_string();
    let login = verify(
        le,
        http,
        discovery,
        SecretString::from(access_token.clone()),
    )
    .await?;
    let now = Utc::now();
    Ok((
        login,
        StoredCredential::Oauth(OauthCredential {
            access_token,
            refresh_token: refresh_token.expose_secret().to_string(),
            expires_at: now + chrono::Duration::seconds(tokens.expires_in),
            client_id: client_id.to_string(),
            token_endpoint: discovery.token_endpoint.to_string(),
            scope,
            created_at: now,
        }),
    ))
}

async fn token_login(
    le: &LoginEnv<'_>,
    http: &reqwest::Client,
    discovery: &Discovery,
    token: Option<SecretString>,
) -> Result<(String, StoredCredential)> {
    let token = match token {
        Some(token) => token,
        None => {
            if le.ui.is_interactive() {
                let page = discovery.token_page();
                crate::ui::eprint_line(&format!(
                    "Create a permanent token in your YouTrack profile:\n  {page}\n\n  1. Under Tokens, click New token.\n  2. Name it after this computer (e.g. yt on my laptop), keep the YouTrack scope, and click Create.\n  3. Copy the token.\n"
                ));
                (le.open_browser)(page.as_str());
            }
            le.ui
                .secret("Paste the token", None, "--token-stdin")
                .await?
        }
    };
    let token_text = token.expose_secret().to_string();
    let login = verify(le, http, discovery, token).await.map_err(|error| {
        if matches!(
            error.downcast_ref::<YtError>(),
            Some(YtError::CredentialRejected { .. })
        ) {
            YtError::TokenRejected.into()
        } else {
            error
        }
    })?;
    Ok((login, StoredCredential::Token { token: token_text }))
}

async fn verify(
    le: &LoginEnv<'_>,
    http: &reqwest::Client,
    discovery: &Discovery,
    token: SecretString,
) -> Result<String> {
    let source = Arc::new(StaticToken::new(token, discovery.url.to_string()));
    let client = Client::new(http.clone(), discovery.url.clone(), source, le.http.retries);
    Ok(crate::api::users::me(&client).await?.login)
}

async fn profile_name(
    le: &LoginEnv<'_>,
    config: &GlobalConfig,
    url: &InstanceUrl,
    login: &str,
    requested: Option<String>,
) -> Result<String> {
    if let Some(name) = requested {
        return Ok(name);
    }
    let existing = config.profiles.iter().find(|(_, profile)| {
        InstanceUrl::parse(&profile.url).ok().as_ref() == Some(url)
            && profile.login.as_deref() == Some(login)
    });
    if let Some((name, _)) = existing {
        return Ok(name.clone());
    }
    let hint = free_name(config, &url.profile_name_hint());
    if !le.ui.is_interactive() {
        return Ok(hint);
    }
    loop {
        let name = le
            .ui
            .input(
                "Profile name",
                Some("a short name to pick this account with --profile"),
                Some(&hint),
                "--profile",
            )
            .await?;
        if name.is_empty() || name.chars().any(char::is_whitespace) {
            crate::ui::eprint_line("Use one word without spaces.");
            continue;
        }
        if config.profiles.contains_key(&name) {
            let free = free_name(config, &name);
            crate::ui::eprint_line(&format!(
                "Profile '{name}' already exists for another account; try '{free}'."
            ));
            continue;
        }
        return Ok(name);
    }
}

/// A profile named with `--profile` keeps its account: same instance, and the same login once it is known.
fn check_requested(
    config: &GlobalConfig,
    requested: Option<&str>,
    url: &InstanceUrl,
    login: Option<&str>,
) -> Result<()> {
    let Some((name, existing)) = requested.and_then(|name| config.profiles.get_key_value(name))
    else {
        return Ok(());
    };
    let free = free_name(config, name);
    if InstanceUrl::parse(&existing.url).ok().as_ref() != Some(url) {
        return Err(YtError::Usage {
            message: format!(
                "profile '{name}' is for {}, not {url}; pick another --profile name, e.g. '{free}'",
                existing.url
            ),
        }
        .into());
    }
    if let (Some(login), Some(current)) = (login, existing.login.as_deref())
        && current != login
    {
        return Err(YtError::Usage {
            message: format!(
                "profile '{name}' is signed in as {current}, but you signed in as {login}; pick another --profile name, e.g. '{free}'"
            ),
        }
        .into());
    }
    Ok(())
}

fn free_name(config: &GlobalConfig, base: &str) -> String {
    if !config.profiles.contains_key(base) {
        return base.to_string();
    }
    (2..)
        .map(|n| format!("{base}-{n}"))
        .find(|name| !config.profiles.contains_key(name))
        .unwrap_or_else(|| base.to_string())
}

fn client_id_for(config: &GlobalConfig, url: &InstanceUrl) -> Option<String> {
    config
        .profiles
        .values()
        .filter(|profile| InstanceUrl::parse(&profile.url).ok().as_ref() == Some(url))
        .find_map(|profile| profile.oauth_client_id.clone())
}

/// Best effort: a leftover refresh token expires on its own after 90 days.
async fn revoke_previous(
    http: &reqwest::Client,
    discovery: &Discovery,
    account: &str,
    credential: &StoredCredential,
) {
    let StoredCredential::Oauth(new) = credential else {
        return;
    };
    let Ok(Some(StoredCredential::Oauth(old))) = credentials::load(account).await else {
        return;
    };
    let access = SecretString::from(new.access_token.clone());
    let result: Result<()> = async {
        let tokens = oauth::list_refresh_tokens(http, &discovery.hub, &access).await?;
        if let Some(token) = oauth::find_issued(&tokens, &old.client_id, old.created_at) {
            oauth::delete_refresh_token(http, &discovery.hub, &access, &token.id).await?;
        }
        Ok(())
    }
    .await;
    if let Err(error) = result {
        log::warn!("the previous sign-in was not revoked: {error}");
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Read as _, Write as _};

    use serde_json::json;
    use url::Url;
    use wiremock::matchers::{body_string_contains, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;

    struct Fixture {
        _dir: tempfile::TempDir,
        store: ConfigStore,
        http: HttpConfig,
        env: Env,
    }

    fn fixture() -> Fixture {
        keyring_core::set_default_store(keyring_core::mock::Store::new().unwrap());
        let dir = tempfile::tempdir().unwrap();
        let store = ConfigStore::new(dir.path().join("config.toml"));
        Fixture {
            _dir: dir,
            store,
            http: HttpConfig {
                timeout: Duration::from_secs(5),
                retries: 0,
                ca_file: None,
            },
            env: Env::from_vars([("DISPLAY", ":0")]),
        }
    }

    async fn youtrack(users_me: ResponseTemplate) -> MockServer {
        instance(users_me, false).await
    }

    async fn instance(users_me: ResponseTemplate, cimd: bool) -> MockServer {
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
            .and(path("/hub/.well-known/oauth-authorization-server"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "authorization_endpoint": format!("{}/hub/api/rest/oauth2/auth", server.uri()),
                "token_endpoint": format!("{}/hub/api/rest/oauth2/token", server.uri()),
                "client_id_metadata_document_supported": cimd
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/users/me"))
            .respond_with(users_me)
            .mount(&server)
            .await;
        server
    }

    fn jane() -> ResponseTemplate {
        ResponseTemplate::new(200)
            .set_body_json(json!({"id": "1-1", "login": "jane", "name": "Jane"}))
    }

    fn no_browser(_: &str) -> bool {
        false
    }

    #[tokio::test]
    async fn a_token_from_stdin_signs_in_without_prompts() {
        let server = youtrack(jane()).await;
        let f = fixture();
        let ui = Ui::new(false, false);
        let le = LoginEnv {
            env: &f.env,
            ui: &ui,
            store: &f.store,
            http: &f.http,
            open_browser: &no_browser,
        };
        let request = LoginRequest {
            url: Some(server.uri()),
            token: Some(SecretString::from(
                "perm-bG9naW4=.dGVzdA==.secret123456".to_string(),
            )),
            ..LoginRequest::default()
        };
        let outcome = login(&le, request).await.unwrap();
        assert_eq!(outcome.login, "jane");
        let config = f.store.load().unwrap();
        let profile = &config.profiles[&outcome.profile];
        assert_eq!(profile.auth, AuthKind::Token);
        assert!(profile.default_for_url);
        assert_eq!(
            config.default_profile.as_deref(),
            Some(outcome.profile.as_str())
        );
        let stored = credentials::load(&outcome.url.account("jane"))
            .await
            .unwrap();
        assert!(matches!(stored, Some(StoredCredential::Token { .. })));
    }

    #[tokio::test]
    async fn a_rejected_token_says_which_scope_is_needed() {
        let server =
            youtrack(ResponseTemplate::new(401).set_body_json(json!({"error": "Unauthorized"})))
                .await;
        let f = fixture();
        let ui = Ui::new(false, false);
        let le = LoginEnv {
            env: &f.env,
            ui: &ui,
            store: &f.store,
            http: &f.http,
            open_browser: &no_browser,
        };
        let request = LoginRequest {
            url: Some(server.uri()),
            token: Some(SecretString::from(
                "perm-YmFk.dGVzdA==.secret123456".to_string(),
            )),
            ..LoginRequest::default()
        };
        let error = login(&le, request).await.unwrap_err();
        assert!(matches!(
            error.downcast_ref::<YtError>(),
            Some(YtError::TokenRejected)
        ));
    }

    #[tokio::test]
    async fn browser_sign_in_needs_a_terminal() {
        let server = youtrack(jane()).await;
        let f = fixture();
        let ui = Ui::new(false, false);
        let le = LoginEnv {
            env: &f.env,
            ui: &ui,
            store: &f.store,
            http: &f.http,
            open_browser: &no_browser,
        };
        let request = LoginRequest {
            url: Some(server.uri()),
            method: Some(Method::Browser),
            ..LoginRequest::default()
        };
        assert_eq!(
            crate::error::exit_code(&login(&le, request).await.unwrap_err()),
            2
        );
    }

    #[tokio::test]
    async fn a_token_sign_in_without_a_terminal_asks_for_stdin() {
        let server = youtrack(jane()).await;
        let f = fixture();
        let ui = Ui::new(false, false);
        let opened = |_: &str| -> bool { panic!("no browser without a terminal") };
        let le = LoginEnv {
            env: &f.env,
            ui: &ui,
            store: &f.store,
            http: &f.http,
            open_browser: &opened,
        };
        let request = LoginRequest {
            url: Some(server.uri()),
            method: Some(Method::Token),
            ..LoginRequest::default()
        };
        let error = login(&le, request).await.unwrap_err();
        let help = error
            .help()
            .map(|help| help.to_string())
            .unwrap_or_default();
        assert_eq!(help, "pass --token-stdin");
    }

    /// Plays the browser: follows the authorize URL straight to the redirect, as an approving user would.
    fn approve(url: &str) -> bool {
        let url = Url::parse(url).unwrap();
        let query: std::collections::HashMap<String, String> =
            url.query_pairs().into_owned().collect();
        let redirect = Url::parse(&query["redirect_uri"]).unwrap();
        let state = query["state"].clone();
        std::thread::spawn(move || {
            let mut stream =
                std::net::TcpStream::connect(("127.0.0.1", redirect.port().unwrap())).unwrap();
            write!(
                stream,
                "GET /callback?code=c0de&state={state} HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n"
            )
            .unwrap();
            let mut response = String::new();
            let _ = stream.read_to_string(&mut response);
        });
        true
    }

    async fn oauth_endpoints(server: &MockServer) {
        Mock::given(method("GET"))
            .and(path("/hub/api/rest/oauth2/auth"))
            .respond_with(ResponseTemplate::new(303).insert_header("location", "/hub/auth/login"))
            .mount(server)
            .await;
        Mock::given(method("POST"))
            .and(path("/hub/api/rest/oauth2/token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "access_token": "access-0123456789",
                "refresh_token": "refresh-0123456789",
                "expires_in": 3600,
                "token_type": "Bearer"
            })))
            .mount(server)
            .await;
    }

    #[tokio::test]
    async fn browser_sign_in_stores_a_refreshable_credential() {
        let server = youtrack(jane()).await;
        oauth_endpoints(&server).await;
        let f = fixture();
        let ui = Ui::new(true, false);
        let le = LoginEnv {
            env: &f.env,
            ui: &ui,
            store: &f.store,
            http: &f.http,
            open_browser: &approve,
        };
        let request = LoginRequest {
            url: Some(server.uri()),
            method: Some(Method::Browser),
            client_id: Some("yt-cli".into()),
            profile: Some("acme".into()),
            ..LoginRequest::default()
        };
        let outcome = login(&le, request).await.unwrap();
        assert_eq!(outcome.profile, "acme");
        let profile = &f.store.load().unwrap().profiles["acme"];
        assert_eq!(profile.auth, AuthKind::Oauth);
        assert_eq!(profile.oauth_client_id.as_deref(), Some("yt-cli"));
        let Some(StoredCredential::Oauth(stored)) = credentials::load(&outcome.url.account("jane"))
            .await
            .unwrap()
        else {
            panic!("no oauth credential stored")
        };
        assert_eq!(stored.refresh_token, "refresh-0123456789");
        assert_eq!(stored.scope, "svc");
    }

    #[tokio::test]
    async fn a_revocable_sign_in_revokes_the_previous_one() {
        let server = youtrack(jane()).await;
        oauth_endpoints(&server).await;
        let f = fixture();
        let url = InstanceUrl::parse(&server.uri()).unwrap();
        let earlier = Utc::now() - chrono::Duration::days(1);
        credentials::save(
            &url.account("jane"),
            &StoredCredential::Oauth(OauthCredential {
                access_token: "old-access-0123456789".into(),
                refresh_token: "old-refresh-0123456789".into(),
                expires_at: earlier,
                client_id: "yt-cli".into(),
                token_endpoint: format!("{}/hub/api/rest/oauth2/token", server.uri()),
                scope: "svc 0-0-0-0-0".into(),
                created_at: earlier,
            }),
        )
        .await
        .unwrap();
        Mock::given(method("GET"))
            .and(path("/hub/api/rest/users/me/refreshtokens"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "refreshtokens": [{"id": "old-rt", "creationTime": earlier.timestamp_millis(), "client": {"key": "yt-cli"}}]
            })))
            .mount(&server)
            .await;
        Mock::given(method("DELETE"))
            .and(path("/hub/api/rest/users/me/refreshtokens/old-rt"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;
        let ui = Ui::new(true, false);
        let le = LoginEnv {
            env: &f.env,
            ui: &ui,
            store: &f.store,
            http: &f.http,
            open_browser: &approve,
        };
        let request = LoginRequest {
            url: Some(server.uri()),
            method: Some(Method::Browser),
            client_id: Some("yt-cli".into()),
            profile: Some("acme".into()),
            revocable: true,
            ..LoginRequest::default()
        };
        login(&le, request).await.unwrap();
        assert!(f.store.load().unwrap().profiles["acme"].revocable);
    }

    /// Expects exactly one code exchange, made with `client_id`.
    async fn oauth_endpoints_for(server: &MockServer, client_id: &str) {
        let encoded: String = url::form_urlencoded::byte_serialize(client_id.as_bytes()).collect();
        Mock::given(method("GET"))
            .and(path("/hub/api/rest/oauth2/auth"))
            .respond_with(ResponseTemplate::new(303).insert_header("location", "/hub/auth/login"))
            .mount(server)
            .await;
        Mock::given(method("POST"))
            .and(path("/hub/api/rest/oauth2/token"))
            .and(body_string_contains(format!("client_id={encoded}")))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "access_token": "access-0123456789",
                "refresh_token": "refresh-0123456789",
                "expires_in": 3600,
                "token_type": "Bearer"
            })))
            .expect(1)
            .mount(server)
            .await;
    }

    /// Signs in as profile `acme` through the browser and returns the client ID the authorize URL named.
    async fn browser_sign_in(f: &Fixture, server: &MockServer, client_id: Option<&str>) -> String {
        let seen = std::sync::Mutex::new(None);
        let open = |url: &str| {
            let query: std::collections::HashMap<String, String> = Url::parse(url)
                .unwrap()
                .query_pairs()
                .into_owned()
                .collect();
            *seen.lock().unwrap() = query.get("client_id").cloned();
            approve(url)
        };
        let ui = Ui::new(true, false);
        let le = LoginEnv {
            env: &f.env,
            ui: &ui,
            store: &f.store,
            http: &f.http,
            open_browser: &open,
        };
        let request = LoginRequest {
            url: Some(server.uri()),
            method: Some(Method::Browser),
            client_id: client_id.map(str::to_string),
            profile: Some("acme".into()),
            ..LoginRequest::default()
        };
        login(&le, request).await.unwrap();
        seen.into_inner()
            .unwrap()
            .expect("the browser was not opened")
    }

    #[tokio::test]
    async fn with_automatic_registration_browser_sign_in_needs_no_client() {
        let server = instance(jane(), true).await;
        oauth_endpoints_for(&server, CIMD_CLIENT_ID).await;
        let f = fixture();
        assert_eq!(browser_sign_in(&f, &server, None).await, CIMD_CLIENT_ID);
        let profile = &f.store.load().unwrap().profiles["acme"];
        assert_eq!(profile.oauth_client_id.as_deref(), Some(CIMD_CLIENT_ID));
    }

    #[tokio::test]
    async fn a_client_saved_for_the_instance_wins_over_automatic_registration() {
        let server = instance(jane(), true).await;
        oauth_endpoints_for(&server, "yt-cli").await;
        let f = fixture();
        let saved = Profile {
            oauth_client_id: Some("yt-cli".into()),
            ..existing(&server.uri(), "jane")
        };
        f.store.set_profile("acme", &saved).unwrap();
        assert_eq!(browser_sign_in(&f, &server, None).await, "yt-cli");
    }

    #[tokio::test]
    async fn the_client_id_flag_wins_over_automatic_registration() {
        let server = instance(jane(), true).await;
        oauth_endpoints_for(&server, "from-flag").await;
        let f = fixture();
        assert_eq!(
            browser_sign_in(&f, &server, Some("from-flag")).await,
            "from-flag"
        );
    }

    #[test]
    fn the_client_setup_help_offers_automatic_registration() {
        let text = client_setup(&InstanceUrl::parse("https://acme.youtrack.cloud").unwrap());
        assert!(
            text.contains("Allow automatic OAuth client registration via CIMD"),
            "{text}"
        );
        assert!(text.contains("http://127.0.0.1:8765/callback"), "{text}");
    }

    fn existing(url: &str, login: &str) -> Profile {
        Profile {
            url: url.into(),
            login: Some(login.into()),
            auth: AuthKind::Token,
            project: Some("OPS".into()),
            default_for_url: true,
            oauth_client_id: None,
            ca_file: None,
            revocable: false,
            apps: vec![],
        }
    }

    #[tokio::test]
    async fn a_requested_profile_of_another_account_is_left_alone() {
        let server = youtrack(jane()).await;
        let f = fixture();
        f.store
            .set_profile("acme", &existing(&server.uri(), "jane.admin"))
            .unwrap();
        let ui = Ui::new(false, false);
        let le = LoginEnv {
            env: &f.env,
            ui: &ui,
            store: &f.store,
            http: &f.http,
            open_browser: &no_browser,
        };
        let request = LoginRequest {
            url: Some(server.uri()),
            token: Some(SecretString::from(
                "perm-YWNtZQ==.dGVzdA==.secret123456".to_string(),
            )),
            profile: Some("acme".into()),
            ..LoginRequest::default()
        };
        let error = login(&le, request).await.unwrap_err();
        assert!(error.to_string().contains("jane.admin"), "{error}");
        assert_eq!(
            f.store.load().unwrap().profiles["acme"].login.as_deref(),
            Some("jane.admin")
        );
        let url = InstanceUrl::parse(&server.uri()).unwrap();
        assert!(
            credentials::load(&url.account("jane"))
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn a_requested_profile_of_another_instance_is_left_alone() {
        let server = youtrack(jane()).await;
        let f = fixture();
        f.store
            .set_profile("acme", &existing("https://other.youtrack.cloud", "jane"))
            .unwrap();
        let ui = Ui::new(false, false);
        let le = LoginEnv {
            env: &f.env,
            ui: &ui,
            store: &f.store,
            http: &f.http,
            open_browser: &no_browser,
        };
        let request = LoginRequest {
            url: Some(server.uri()),
            token: Some(SecretString::from(
                "perm-b3RoZXI=.dGVzdA==.secret123456".to_string(),
            )),
            profile: Some("acme".into()),
            ..LoginRequest::default()
        };
        let error = login(&le, request).await.unwrap_err();
        assert!(
            error.to_string().contains("other.youtrack.cloud"),
            "{error}"
        );
        assert_eq!(
            f.store.load().unwrap().profiles["acme"].url,
            "https://other.youtrack.cloud"
        );
    }
}
