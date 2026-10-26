use secrecy::SecretString;

use crate::config::file::{GlobalConfig, Profile};
use crate::config::repo::RepoConfig;
use crate::env::Env;
use crate::error::{Result, YtError};
use crate::instance::InstanceUrl;

#[derive(Debug)]
pub enum ProfileChoice {
    /// `YOUTRACK_URL` is set: a temporary profile authenticated with `YOUTRACK_TOKEN`.
    Env {
        url: InstanceUrl,
        token: SecretString,
    },
    Named {
        name: String,
        profile: Profile,
    },
}

impl ProfileChoice {
    pub fn label(&self) -> &str {
        match self {
            Self::Env { .. } => "YOUTRACK_URL",
            Self::Named { name, .. } => name,
        }
    }

    pub fn url(&self) -> Result<InstanceUrl> {
        match self {
            Self::Env { url, .. } => Ok(url.clone()),
            Self::Named { profile, .. } => InstanceUrl::parse(&profile.url),
        }
    }
}

/// Picks the profile for this invocation: environment, flag, repo URL, default, the only one.
pub fn resolve(
    flag: Option<&str>,
    env: &Env,
    repo: Option<&RepoConfig>,
    config: &GlobalConfig,
) -> Result<ProfileChoice> {
    if let Some(url) = env.get("YOUTRACK_URL") {
        let token = env.get("YOUTRACK_TOKEN").ok_or_else(|| YtError::Usage {
            message: "YOUTRACK_URL is set but YOUTRACK_TOKEN is not".into(),
        })?;
        crate::redact::register(token);
        return Ok(ProfileChoice::Env {
            url: InstanceUrl::parse(url)?,
            token: SecretString::from(token.to_string()),
        });
    }
    if let Some(name) = flag {
        return named(name, config);
    }
    if let Some(repo) = repo
        && let Some(url) = &repo.url
    {
        let matches: Vec<(&String, &Profile)> = config
            .profiles
            .iter()
            .filter(|(_, profile)| InstanceUrl::parse(&profile.url).ok().as_ref() == Some(url))
            .collect();
        return match matches.as_slice() {
            [] => Err(YtError::NoProfileForUrl {
                url: url.to_string(),
                path: repo.path.display().to_string(),
            }
            .into()),
            [(name, _)] => named(name, config),
            several => match several.iter().find(|(_, profile)| profile.default_for_url) {
                Some((name, _)) => named(name, config),
                None => Err(YtError::AmbiguousProfile {
                    url: url.to_string(),
                    count: several.len(),
                }
                .into()),
            },
        };
    }
    if let Some(name) = &config.default_profile {
        return named(name, config);
    }
    match config.profiles.iter().collect::<Vec<_>>().as_slice() {
        [(name, _)] => named(name, config),
        _ => Err(YtError::NoProfile.into()),
    }
}

pub fn project(
    flag: Option<&str>,
    repo: Option<&RepoConfig>,
    choice: &ProfileChoice,
) -> Option<String> {
    let from_profile = match choice {
        ProfileChoice::Named { profile, .. } => profile.project.clone(),
        ProfileChoice::Env { .. } => None,
    };
    flag.map(str::to_string)
        .or_else(|| repo.and_then(|repo| repo.project.clone()))
        .or(from_profile)
}

pub fn unknown_profile(name: &str, config: &GlobalConfig) -> miette::Report {
    let suggestion = config
        .profiles
        .keys()
        .map(|candidate| (strsim::jaro_winkler(name, candidate), candidate))
        .filter(|(score, _)| *score > 0.8)
        .max_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, candidate)| format!("did you mean '{candidate}'?"));
    YtError::UnknownProfile {
        name: name.to_string(),
        help: suggestion.or_else(|| Some("run `yt profile list` to see the profiles".into())),
    }
    .into()
}

fn named(name: &str, config: &GlobalConfig) -> Result<ProfileChoice> {
    let profile = config
        .profiles
        .get(name)
        .ok_or_else(|| unknown_profile(name, config))?;
    Ok(ProfileChoice::Named {
        name: name.to_string(),
        profile: profile.clone(),
    })
}

/// Other profiles signed in as the same account as `name` (same instance and login); they share its keyring entry.
pub fn sharing_account<'a>(config: &'a GlobalConfig, name: &str) -> Vec<&'a str> {
    let Some(profile) = config.profiles.get(name) else {
        return Vec::new();
    };
    let Some(login) = profile.login.as_deref() else {
        return Vec::new();
    };
    let url = InstanceUrl::parse(&profile.url).ok();
    config
        .profiles
        .iter()
        .filter(|(other, candidate)| {
            other.as_str() != name
                && candidate.login.as_deref() == Some(login)
                && url.is_some()
                && InstanceUrl::parse(&candidate.url).ok() == url
        })
        .map(|(other, _)| other.as_str())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::file::AuthKind;

    fn profile(url: &str, default_for_url: bool) -> Profile {
        Profile {
            url: url.into(),
            login: Some("jane".into()),
            auth: AuthKind::Token,
            project: Some("PRO".into()),
            default_for_url,
            oauth_client_id: None,
            ca_file: None,
            revocable: false,
            apps: vec![],
        }
    }

    fn config() -> GlobalConfig {
        GlobalConfig {
            default_profile: Some("home".into()),
            profiles: [
                (
                    "home".to_string(),
                    profile("https://home.youtrack.cloud", false),
                ),
                (
                    "work".to_string(),
                    profile("https://acme.youtrack.cloud", true),
                ),
                (
                    "bot".to_string(),
                    profile("https://acme.youtrack.cloud", false),
                ),
            ]
            .into(),
        }
    }

    fn name(choice: ProfileChoice) -> String {
        choice.label().to_string()
    }

    fn repo(url: &str) -> RepoConfig {
        RepoConfig {
            path: "/r/.yt.toml".into(),
            url: Some(InstanceUrl::parse(url).unwrap()),
            project: Some("WEB".into()),
        }
    }

    #[test]
    fn environment_wins() {
        let env = Env::from_vars([
            ("YOUTRACK_URL", "https://ci.youtrack.cloud"),
            ("YOUTRACK_TOKEN", "perm-x"),
        ]);
        let choice = resolve(Some("home"), &env, None, &config()).unwrap();
        assert!(
            matches!(choice, ProfileChoice::Env { ref url, .. } if url.to_string() == "https://ci.youtrack.cloud")
        );
    }

    #[test]
    fn a_url_without_a_token_is_a_usage_error() {
        let env = Env::from_vars([("YOUTRACK_URL", "https://ci.youtrack.cloud")]);
        let error = resolve(None, &env, None, &config()).unwrap_err();
        assert_eq!(crate::error::exit_code(&error), 2);
    }

    #[test]
    fn the_flag_wins_over_repo_and_default() {
        let choice = resolve(
            Some("bot"),
            &Env::default(),
            Some(&repo("https://home.youtrack.cloud")),
            &config(),
        )
        .unwrap();
        assert_eq!(name(choice), "bot");
    }

    #[test]
    fn the_repo_url_picks_the_default_for_that_url() {
        let choice = resolve(
            None,
            &Env::default(),
            Some(&repo("https://acme.youtrack.cloud/")),
            &config(),
        )
        .unwrap();
        assert_eq!(name(choice), "work");
    }

    #[test]
    fn a_repo_url_without_a_profile_is_an_error() {
        let error = resolve(
            None,
            &Env::default(),
            Some(&repo("https://other.youtrack.cloud")),
            &config(),
        )
        .unwrap_err();
        assert!(matches!(
            error.downcast_ref::<YtError>(),
            Some(YtError::NoProfileForUrl { .. })
        ));
        let help = error
            .help()
            .map(|help| help.to_string())
            .unwrap_or_default();
        assert!(
            help.starts_with("/r/.yt.toml names this instance")
                && help.contains("yt auth login https://other.youtrack.cloud"),
            "{help}"
        );
    }

    #[test]
    fn several_profiles_without_a_default_for_the_url_are_ambiguous() {
        let mut config = config();
        config.profiles.get_mut("work").unwrap().default_for_url = false;
        let error = resolve(
            None,
            &Env::default(),
            Some(&repo("https://acme.youtrack.cloud")),
            &config,
        )
        .unwrap_err();
        assert!(matches!(
            error.downcast_ref::<YtError>(),
            Some(YtError::AmbiguousProfile { count: 2, .. })
        ));
    }

    #[test]
    fn then_the_default_profile_then_the_only_one() {
        assert_eq!(
            name(resolve(None, &Env::default(), None, &config()).unwrap()),
            "home"
        );
        let only = GlobalConfig {
            default_profile: None,
            profiles: [(
                "solo".to_string(),
                profile("https://x.youtrack.cloud", false),
            )]
            .into(),
        };
        assert_eq!(
            name(resolve(None, &Env::default(), None, &only).unwrap()),
            "solo"
        );
        let error = resolve(None, &Env::default(), None, &GlobalConfig::default()).unwrap_err();
        assert!(matches!(
            error.downcast_ref::<YtError>(),
            Some(YtError::NoProfile)
        ));
    }

    #[test]
    fn unknown_names_get_a_suggestion() {
        let error = resolve(Some("wrok"), &Env::default(), None, &config()).unwrap_err();
        assert!(format!("{error:?}").contains("did you mean 'work'?"));
    }

    #[test]
    fn project_order() {
        let choice = resolve(Some("home"), &Env::default(), None, &config()).unwrap();
        let repo = repo("https://home.youtrack.cloud");
        assert_eq!(
            project(Some("FLAG"), Some(&repo), &choice).as_deref(),
            Some("FLAG")
        );
        assert_eq!(project(None, Some(&repo), &choice).as_deref(), Some("WEB"));
        assert_eq!(project(None, None, &choice).as_deref(), Some("PRO"));
    }

    #[test]
    fn profiles_signed_in_as_the_same_account_share_it() {
        let mut bob = profile("https://acme.youtrack.cloud", false);
        bob.login = Some("bob".into());
        let config = GlobalConfig {
            default_profile: None,
            profiles: [
                (
                    "a".to_string(),
                    profile("https://acme.youtrack.cloud/", true),
                ),
                ("b".to_string(), profile("acme.youtrack.cloud", false)),
                ("c".to_string(), bob),
                (
                    "d".to_string(),
                    profile("https://other.youtrack.cloud", true),
                ),
            ]
            .into(),
        };
        assert_eq!(sharing_account(&config, "b"), vec!["a"]);
        assert!(sharing_account(&config, "c").is_empty());
        assert!(sharing_account(&config, "missing").is_empty());
    }
}
