use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use miette::IntoDiagnostic as _;
use serde::Deserialize;
use toml_edit::{DocumentMut, Item, Table, value};

use crate::error::{Result, YtError};
use crate::instance::InstanceUrl;

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum AuthKind {
    Oauth,
    Token,
}

impl AuthKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Oauth => "oauth",
            Self::Token => "token",
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct Profile {
    pub url: String,
    pub login: Option<String>,
    pub auth: AuthKind,
    pub project: Option<String>,
    #[serde(default)]
    pub default_for_url: bool,
    pub oauth_client_id: Option<String>,
    pub ca_file: Option<PathBuf>,
    #[serde(default)]
    pub revocable: bool,
    /// Ids of the YouTrack apps `yt app` runs for this profile.
    #[serde(default)]
    pub apps: Vec<String>,
}

#[derive(Debug, Default, Deserialize)]
pub struct GlobalConfig {
    pub default_profile: Option<String>,
    #[serde(default)]
    pub profiles: BTreeMap<String, Profile>,
}

/// Reads the global config and writes changes without losing the user's comments or formatting.
pub struct ConfigStore {
    path: PathBuf,
}

impl ConfigStore {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn load(&self) -> Result<GlobalConfig> {
        match std::fs::read_to_string(&self.path) {
            Ok(text) => toml::from_str(&text).map_err(|error| self.invalid(error.to_string())),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                Ok(GlobalConfig::default())
            }
            Err(error) => Err(self.invalid(error.to_string())),
        }
    }

    pub fn set_profile(&self, name: &str, profile: &Profile) -> Result<()> {
        self.edit(|doc| {
            let table = self.profile_table(doc, name)?;
            table["url"] = value(profile.url.as_str());
            set_optional(table, "login", profile.login.as_deref());
            table["auth"] = value(profile.auth.as_str());
            set_optional(table, "project", profile.project.as_deref());
            set_flag(table, "default_for_url", profile.default_for_url);
            set_optional(table, "oauth_client_id", profile.oauth_client_id.as_deref());
            set_optional(
                table,
                "ca_file",
                profile.ca_file.as_deref().and_then(Path::to_str),
            );
            set_flag(table, "revocable", profile.revocable);
            Ok(())
        })
    }

    /// Sets the profile's `apps`, removing the key when there are none.
    pub fn set_apps(&self, name: &str, apps: &[String]) -> Result<()> {
        self.edit(|doc| {
            let table = self.profile_table(doc, name)?;
            if apps.is_empty() {
                table.remove("apps");
            } else {
                table["apps"] = value(
                    apps.iter()
                        .map(String::as_str)
                        .collect::<toml_edit::Array>(),
                );
            }
            Ok(())
        })
    }

    pub fn remove_profile(&self, name: &str) -> Result<()> {
        self.edit(|doc| {
            if let Some(profiles) = doc.get_mut("profiles").and_then(Item::as_table_mut) {
                profiles.remove(name);
            }
            if doc.get("default_profile").and_then(Item::as_str) == Some(name) {
                doc.remove("default_profile");
            }
            Ok(())
        })
    }

    /// Makes `name` the default profile, and the default among profiles for the same URL.
    pub fn set_default_profile(&self, name: &str) -> Result<()> {
        let config = self.load()?;
        let target = config
            .profiles
            .get(name)
            .ok_or_else(|| self.invalid(format!("there is no profile '{name}'")))?;
        let url = InstanceUrl::parse(&target.url).ok();
        let same_url: Vec<String> = config
            .profiles
            .iter()
            .filter(|(other, profile)| {
                *other != name && InstanceUrl::parse(&profile.url).ok() == url
            })
            .map(|(other, _)| other.clone())
            .collect();
        self.edit(|doc| {
            doc["default_profile"] = value(name);
            for other in &same_url {
                self.profile_table(doc, other)?.remove("default_for_url");
            }
            self.profile_table(doc, name)?["default_for_url"] = value(true);
            Ok(())
        })
    }

    pub fn set_setting(&self, key: &str, setting: toml_edit::Value) -> Result<()> {
        self.edit(|doc| {
            let settings = doc
                .entry("settings")
                .or_insert(Item::Table(Table::new()))
                .as_table_mut()
                .ok_or_else(|| self.invalid("[settings] is not a table".into()))?;
            settings[key] = Item::Value(setting);
            Ok(())
        })
    }

    fn profile_table<'d>(&self, doc: &'d mut DocumentMut, name: &str) -> Result<&'d mut Table> {
        let profiles = doc
            .entry("profiles")
            .or_insert(Item::Table(Table::new()))
            .as_table_mut()
            .ok_or_else(|| self.invalid("[profiles] is not a table".into()))?;
        profiles.set_implicit(true);
        profiles
            .entry(name)
            .or_insert(Item::Table(Table::new()))
            .as_table_mut()
            .ok_or_else(|| self.invalid(format!("[profiles.{name}] is not a table")))
    }

    fn edit(&self, change: impl FnOnce(&mut DocumentMut) -> Result<()>) -> Result<()> {
        let text = match std::fs::read_to_string(&self.path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(error) => return Err(self.invalid(error.to_string())),
        };
        let mut doc: DocumentMut = text
            .parse()
            .map_err(|error: toml_edit::TomlError| self.invalid(error.to_string()))?;
        change(&mut doc)?;
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).into_diagnostic()?;
        }
        // Rename onto the link's target so a symlinked config (dotfile managers) stays a link.
        let target = std::fs::canonicalize(&self.path).unwrap_or_else(|_| self.path.clone());
        let name = target
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let temporary = target.with_file_name(format!(".{name}.{}.tmp", std::process::id()));
        std::fs::write(&temporary, doc.to_string()).into_diagnostic()?;
        std::fs::rename(&temporary, &target).into_diagnostic()
    }

    fn invalid(&self, message: String) -> miette::Report {
        YtError::Config {
            path: self.path.display().to_string(),
            message,
        }
        .into()
    }
}

fn set_optional(table: &mut Table, key: &str, setting: Option<&str>) {
    match setting {
        Some(setting) => table[key] = value(setting),
        None => {
            table.remove(key);
        }
    }
}

fn set_flag(table: &mut Table, key: &str, flag: bool) {
    if flag {
        table[key] = value(true);
    } else {
        table.remove(key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store_with(text: &str) -> (tempfile::TempDir, ConfigStore) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("yt").join("config.toml");
        if !text.is_empty() {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, text).unwrap();
        }
        (dir, ConfigStore::new(path))
    }

    fn profile(url: &str) -> Profile {
        Profile {
            url: url.into(),
            login: Some("jane".into()),
            auth: AuthKind::Oauth,
            project: None,
            default_for_url: false,
            oauth_client_id: Some("yt-cli".into()),
            ca_file: None,
            revocable: false,
            apps: vec![],
        }
    }

    #[test]
    fn a_missing_file_is_an_empty_config() {
        let (_dir, store) = store_with("");
        let config = store.load().unwrap();
        assert!(config.profiles.is_empty() && config.default_profile.is_none());
    }

    #[test]
    fn set_profile_creates_the_file_and_round_trips() {
        let (_dir, store) = store_with("");
        store
            .set_profile("acme", &profile("https://acme.youtrack.cloud"))
            .unwrap();
        let config = store.load().unwrap();
        assert_eq!(
            config.profiles["acme"],
            profile("https://acme.youtrack.cloud")
        );
    }

    #[test]
    fn writes_keep_comments_and_unknown_keys() {
        let (_dir, store) = store_with(
            "# my yt config\n[profiles.acme] # work\nurl = \"https://acme.youtrack.cloud\"\nauth = \"token\"\ncolor_hint = \"keep me\"\n",
        );
        store
            .set_profile("acme", &profile("https://acme.youtrack.cloud"))
            .unwrap();
        let text = std::fs::read_to_string(store.path()).unwrap();
        assert!(text.starts_with("# my yt config\n"), "{text}");
        assert!(text.contains("# work"), "{text}");
        assert!(text.contains("color_hint = \"keep me\""), "{text}");
        assert!(text.contains("auth = \"oauth\""), "{text}");
    }

    #[test]
    fn apps_are_set_and_removed_without_touching_the_rest() {
        let (_dir, store) = store_with(
            "# mine\n[profiles.acme] # work\nurl = \"https://acme.youtrack.cloud\"\nauth = \"token\"\n",
        );
        store
            .set_apps("acme", &["hidden-fields".into(), "other".into()])
            .unwrap();
        let text = std::fs::read_to_string(store.path()).unwrap();
        assert!(
            text.contains("apps = [\"hidden-fields\", \"other\"]"),
            "{text}"
        );
        assert!(text.contains("# mine") && text.contains("# work"), "{text}");
        assert_eq!(
            store.load().unwrap().profiles["acme"].apps,
            ["hidden-fields", "other"]
        );
        store
            .set_profile("acme", &profile("https://acme.youtrack.cloud"))
            .unwrap();
        assert_eq!(
            store.load().unwrap().profiles["acme"].apps,
            ["hidden-fields", "other"]
        );
        store.set_apps("acme", &[]).unwrap();
        let text = std::fs::read_to_string(store.path()).unwrap();
        assert!(!text.contains("apps"), "{text}");
    }

    #[test]
    fn default_profile_and_default_for_url() {
        let (_dir, store) = store_with("");
        store
            .set_profile("a", &profile("https://acme.youtrack.cloud"))
            .unwrap();
        store
            .set_profile("b", &profile("https://acme.youtrack.cloud/"))
            .unwrap();
        store.set_default_profile("a").unwrap();
        store.set_default_profile("b").unwrap();
        let config = store.load().unwrap();
        assert_eq!(config.default_profile.as_deref(), Some("b"));
        assert!(config.profiles["b"].default_for_url);
        assert!(!config.profiles["a"].default_for_url);
    }

    #[test]
    fn remove_profile_clears_the_default() {
        let (_dir, store) = store_with("");
        store
            .set_profile("a", &profile("https://acme.youtrack.cloud"))
            .unwrap();
        store.set_default_profile("a").unwrap();
        store.remove_profile("a").unwrap();
        let config = store.load().unwrap();
        assert!(config.profiles.is_empty() && config.default_profile.is_none());
    }

    #[test]
    fn set_setting_writes_the_settings_table() {
        let (_dir, store) = store_with("");
        store.set_setting("http_retries", 5.into()).unwrap();
        let text = std::fs::read_to_string(store.path()).unwrap();
        assert!(
            text.contains("[settings]") && text.contains("http_retries = 5"),
            "{text}"
        );
    }

    #[test]
    fn invalid_toml_is_a_config_error() {
        let (_dir, store) = store_with("[profiles\n");
        let error = store.load().unwrap_err();
        assert!(matches!(
            error.downcast_ref::<YtError>(),
            Some(YtError::Config { .. })
        ));
    }

    #[test]
    fn writes_go_through_a_symlinked_config() {
        let dir = tempfile::tempdir().unwrap();
        let dotfiles = dir.path().join("dotfiles").join("yt.toml");
        std::fs::create_dir_all(dotfiles.parent().unwrap()).unwrap();
        std::fs::write(&dotfiles, "# managed by dotfiles\n").unwrap();
        let link = dir.path().join("config.toml");
        std::os::unix::fs::symlink(&dotfiles, &link).unwrap();
        ConfigStore::new(link.clone())
            .set_profile("acme", &profile("https://acme.youtrack.cloud"))
            .unwrap();
        assert!(link.symlink_metadata().unwrap().file_type().is_symlink());
        let text = std::fs::read_to_string(&dotfiles).unwrap();
        assert!(
            text.contains("# managed by dotfiles") && text.contains("[profiles.acme]"),
            "{text}"
        );
    }
}
