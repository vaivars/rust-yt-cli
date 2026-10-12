use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use secrecy::SecretString;
use usage_rs::spec::ValueEnum as _;
use yt::YtError;
use yt::api::{Client, TokenSource};
use yt::auth::token_source::{OAuthToken, StaticToken};
use yt::config::file::{ConfigStore, GlobalConfig, Profile};
use yt::config::profile::ProfileChoice;
use yt::config::repo::RepoConfig;
use yt::config::settings::ResolvedSettings;
use yt::credentials::StoredCredential;
use yt::dirs::Dirs;
use yt::env::Env;
use yt::fields::cache::SchemaCache;
use yt::http::HttpConfig;
use yt::instance::InstanceUrl;
use yt::output::{Format, Output};
use yt::ui::Ui;

use super::Globals;

pub(crate) struct Ctx {
    pub(crate) env: Env,
    pub(crate) dirs: Dirs,
    pub(crate) output: Output,
    pub(crate) ui: Ui,
    settings: OnceLock<ResolvedSettings>,
    pub(crate) profile_flag: Option<String>,
    quiet: bool,
    cwd: PathBuf,
    config: OnceLock<GlobalConfig>,
    repo: OnceLock<Option<RepoConfig>>,
    client: tokio::sync::OnceCell<Client>,
}

impl Ctx {
    pub(crate) fn new(globals: &Globals, env: Env) -> yt::Result<Self> {
        let dirs = Dirs::from_env(&env)?;
        let format = match globals.format {
            Some(format) => format,
            None => format_from(&env)?,
        };
        let ui = Ui::detect(
            &env,
            globals.no_input || env.is_true("YT_NO_INPUT"),
            globals.yes,
            globals.verbose == 0 && !globals.quiet,
        );
        let profile_flag = globals
            .profile
            .clone()
            .or_else(|| env.get("YT_PROFILE").map(str::to_string));
        Ok(Self {
            env,
            dirs,
            output: Output::stdout(format),
            ui,
            settings: OnceLock::new(),
            profile_flag,
            quiet: globals.quiet,
            cwd: std::env::current_dir().unwrap_or_default(),
            config: OnceLock::new(),
            repo: OnceLock::new(),
            client: tokio::sync::OnceCell::new(),
        })
    }

    pub(crate) fn settings(&self) -> yt::Result<&ResolvedSettings> {
        cached(&self.settings, || {
            ResolvedSettings::resolve(&self.env, &self.dirs.config_file())
        })
    }

    pub(crate) fn config_store(&self) -> ConfigStore {
        ConfigStore::new(self.dirs.config_file())
    }

    /// A message for the person at the terminal; never on stdout.
    pub(crate) fn note(&self, message: &str) {
        if !self.quiet {
            yt::ui::eprint_line(message);
        }
    }

    pub(crate) fn global_config(&self) -> yt::Result<&GlobalConfig> {
        cached(&self.config, || self.config_store().load())
    }

    pub(crate) fn repo(&self) -> yt::Result<Option<&RepoConfig>> {
        cached(&self.repo, || yt::config::repo::find(&self.cwd)).map(Option::as_ref)
    }

    pub(crate) fn profile(&self) -> yt::Result<ProfileChoice> {
        yt::config::profile::resolve(
            self.profile_flag.as_deref(),
            &self.env,
            self.repo()?,
            self.global_config()?,
        )
    }

    pub(crate) fn instance(&self) -> yt::Result<InstanceUrl> {
        self.profile()?.url()
    }

    /// Whether this command already sent a change the server may have applied.
    pub(crate) fn has_written(&self) -> bool {
        self.client.get().is_some_and(Client::has_written)
    }

    pub(crate) async fn client(&self) -> yt::Result<&Client> {
        self.client.get_or_try_init(|| self.build_client()).await
    }

    async fn build_client(&self) -> yt::Result<Client> {
        let settings = &self.settings()?.settings;
        match self.profile()? {
            ProfileChoice::Env { url, token } => {
                let config = HttpConfig::from_settings(settings, None)?;
                let auth = Arc::new(StaticToken::new(token, "YOUTRACK_URL"));
                Ok(Client::new(
                    yt::http::client(&config)?,
                    url,
                    auth,
                    config.retries,
                ))
            }
            ProfileChoice::Named { name, profile } => {
                let url = InstanceUrl::parse(&profile.url)?;
                let config = HttpConfig::from_settings(settings, profile.ca_file.clone())?;
                let http = yt::http::client(&config)?;
                let auth: Arc<dyn TokenSource> = match self.env.get("YOUTRACK_TOKEN") {
                    Some(token) => Arc::new(StaticToken::new(
                        SecretString::from(token.to_string()),
                        name,
                    )),
                    None => self.stored_token(&name, &profile, &url, &http).await?,
                };
                Ok(Client::new(http, url, auth, config.retries))
            }
        }
    }

    async fn stored_token(
        &self,
        name: &str,
        profile: &Profile,
        url: &InstanceUrl,
        http: &reqwest::Client,
    ) -> yt::Result<Arc<dyn TokenSource>> {
        let not_signed_in = || -> miette::Report {
            YtError::NotSignedIn {
                profile: name.to_string(),
            }
            .into()
        };
        let login = profile.login.as_deref().ok_or_else(not_signed_in)?;
        yt::credentials::init_os_store()?;
        let account = url.account(login);
        match yt::credentials::load(&account).await? {
            None => Err(not_signed_in()),
            Some(StoredCredential::Token { token }) => {
                Ok(Arc::new(StaticToken::new(SecretString::from(token), name)))
            }
            Some(StoredCredential::Oauth(credential)) => Ok(Arc::new(OAuthToken::new(
                http.clone(),
                account,
                name,
                &self.dirs.state,
                credential,
            ))),
        }
    }

    /// The schema cache of the active profile.
    pub(crate) fn schema_cache(&self) -> yt::Result<SchemaCache> {
        let ttl = self.settings()?.settings.cache_ttl()?;
        Ok(SchemaCache::for_profile(
            &self.dirs.cache,
            self.profile()?.label(),
            &self.instance()?,
            ttl,
        ))
    }
}

fn format_from(env: &Env) -> yt::Result<Format> {
    match env.get("YT_FORMAT") {
        None => Ok(Format::default()),
        Some(value) => Format::from_choice(value).ok_or_else(|| {
            YtError::Usage {
                message: format!("YT_FORMAT must be table, json or ndjson, not '{value}'"),
            }
            .into()
        }),
    }
}

fn cached<T>(cell: &OnceLock<T>, init: impl FnOnce() -> yt::Result<T>) -> yt::Result<&T> {
    if let Some(value) = cell.get() {
        return Ok(value);
    }
    let value = init()?;
    Ok(cell.get_or_init(|| value))
}
