use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

use usage_rs::config::{EnvLayer, FileLayer, FileScope, Layers, Resolved, resolve};

use crate::env::Env;
use crate::error::{Result, YtError};

/// Settings that change how yt behaves. Each comes from the `[settings]` table of the global config, overridable by its environment variable.
#[derive(usage_rs::Config, Clone, Debug, PartialEq)]
#[usage(file(path = "yt/config.toml", xdg = "config", format = "toml"))]
pub struct Settings {
    /// How long a request may take
    #[usage(env = "YT_HTTP_TIMEOUT", default = "30s", ty = "duration")]
    pub http_timeout: String,
    /// How often to retry a read on a network error or HTTP 408, 429 or 5xx
    #[usage(env = "YT_HTTP_RETRIES", default = 3)]
    pub http_retries: u64,
    /// When to use color: auto, always or never
    #[usage(env = "YT_COLOR", default = "auto", choices("auto", "always", "never"))]
    pub color: String,
    /// How long cached project data stays valid
    #[usage(env = "YT_CACHE_TTL", default = "24h", ty = "duration")]
    pub cache_ttl: String,
}

pub const KEYS: [&str; 4] = ["cache_ttl", "color", "http_retries", "http_timeout"];

impl Settings {
    pub fn http_timeout(&self) -> Result<Duration> {
        parse_duration(&self.http_timeout)
            .map_err(|message| invalid_setting("http_timeout", message))
    }

    pub fn cache_ttl(&self) -> Result<Duration> {
        parse_duration(&self.cache_ttl).map_err(|message| invalid_setting("cache_ttl", message))
    }
}

#[derive(Debug)]
pub struct ResolvedSettings {
    pub settings: Settings,
    resolved: Resolved,
}

impl ResolvedSettings {
    pub fn resolve(env: &Env, config_file: &Path) -> Result<Self> {
        let env_layer = EnvLayer::new(
            env.iter()
                .map(|(key, value)| (key.to_string(), value.to_string())),
        );
        let file_layer = config_file
            .is_file()
            .then(|| FileLayer::at(config_file.to_path_buf(), FileScope::Global).under("settings"));
        let mut layers = Layers::new().then(&env_layer);
        if let Some(file_layer) = &file_layer {
            layers = layers.then(file_layer);
        }
        let invalid = |message: String| YtError::Config {
            path: config_file.display().to_string(),
            message,
        };
        let resolved = resolve(Settings::SETTINGS_REGISTRY, layers)
            .map_err(|error| invalid(error.to_string()))?;
        let settings = Settings::read(&resolved).map_err(|error| invalid(format!("{error:?}")))?;
        settings.http_timeout()?;
        settings.cache_ttl()?;
        Ok(Self { settings, resolved })
    }

    /// One `key = value` line per setting.
    pub fn list(&self) -> Vec<String> {
        usage_rs::config::explain::list(&self.resolved)
    }

    /// A setting's value and where it came from.
    pub fn explain(&self, key: &str) -> Result<String> {
        usage_rs::config::explain::explain(&self.resolved, key).ok_or_else(|| unknown_setting(key))
    }

    pub fn values(&self) -> BTreeMap<String, String> {
        KEYS.iter()
            .filter_map(|key| {
                self.resolved
                    .get_key(key)
                    .map(|value| (key.to_string(), value.to_string()))
            })
            .collect()
    }
}

/// `1500ms`, `30s` (or `30`), `5m`, `24h` or `2d`.
pub fn parse_duration(text: &str) -> std::result::Result<Duration, String> {
    let text = text.trim();
    let split = text
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(text.len());
    let (number, unit) = text.split_at(split);
    let value: u64 = number
        .parse()
        .map_err(|_| format!("'{text}' is not a duration like 30s, 5m or 24h"))?;
    let seconds = match unit {
        "ms" => return Ok(Duration::from_millis(value)),
        "" | "s" => value,
        "m" => value * 60,
        "h" => value * 3_600,
        "d" => value * 86_400,
        _ => {
            return Err(format!(
                "'{text}' has an unknown unit; use ms, s, m, h or d"
            ));
        }
    };
    Ok(Duration::from_secs(seconds))
}

pub fn unknown_setting(key: &str) -> miette::Report {
    YtError::Usage {
        message: format!(
            "unknown setting '{key}'; the settings are {}",
            KEYS.join(", ")
        ),
    }
    .into()
}

fn invalid_setting(key: &str, message: String) -> miette::Report {
    YtError::Config {
        path: format!("setting {key}"),
        message,
    }
    .into()
}

/// Checks `raw` for `key` and returns the TOML value to store.
pub fn setting_value(key: &str, raw: &str) -> Result<toml_edit::Value> {
    let usage = |message: String| -> miette::Report { YtError::Usage { message }.into() };
    match key {
        "http_timeout" | "cache_ttl" => {
            parse_duration(raw).map_err(usage)?;
            Ok(raw.into())
        }
        "http_retries" => raw
            .parse::<i64>()
            .ok()
            .filter(|count| *count >= 0)
            .map(Into::into)
            .ok_or_else(|| usage("http_retries must be a whole number".into())),
        "color" if matches!(raw, "auto" | "always" | "never") => Ok(raw.into()),
        "color" => Err(usage("color must be auto, always or never".into())),
        _ => Err(unknown_setting(key)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resolve_with(env: &[(&str, &str)], file: Option<&str>) -> Result<ResolvedSettings> {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        if let Some(text) = file {
            std::fs::write(&path, text).unwrap();
        }
        ResolvedSettings::resolve(&Env::from_vars(env.iter().copied()), &path)
    }

    #[test]
    fn defaults() {
        let resolved = resolve_with(&[], None).unwrap();
        assert_eq!(
            resolved.settings.http_timeout().unwrap(),
            Duration::from_secs(30)
        );
        assert_eq!(resolved.settings.http_retries, 3);
        assert_eq!(resolved.settings.color, "auto");
        assert_eq!(
            resolved.settings.cache_ttl().unwrap(),
            Duration::from_secs(86_400)
        );
    }

    #[test]
    fn the_settings_table_of_the_config_file_is_read() {
        let resolved = resolve_with(
            &[],
            Some("[settings]\nhttp_retries = 5\n[profiles.acme]\nurl = \"x\"\n"),
        )
        .unwrap();
        assert_eq!(resolved.settings.http_retries, 5);
    }

    #[test]
    fn environment_wins_over_the_file() {
        let resolved = resolve_with(
            &[("YT_HTTP_RETRIES", "7")],
            Some("[settings]\nhttp_retries = 5\n"),
        )
        .unwrap();
        assert_eq!(resolved.settings.http_retries, 7);
    }

    #[test]
    fn a_bad_duration_is_a_config_error() {
        let error = resolve_with(&[("YT_HTTP_TIMEOUT", "soon")], None).unwrap_err();
        assert!(error.to_string().contains("http_timeout"), "{error}");
    }

    #[test]
    fn list_has_one_line_per_setting() {
        let lines = resolve_with(&[], None).unwrap().list();
        assert_eq!(lines.len(), KEYS.len());
        for (line, key) in lines.iter().zip(KEYS) {
            assert!(line.starts_with(key), "{line}");
        }
    }

    #[test]
    fn explain_names_the_source() {
        let resolved = resolve_with(&[("YT_HTTP_RETRIES", "7")], None).unwrap();
        let text = resolved.explain("http_retries").unwrap();
        assert!(text.contains("YT_HTTP_RETRIES"), "{text}");
        assert!(resolved.explain("nope").is_err());
    }

    #[test]
    fn durations() {
        assert_eq!(
            parse_duration("1500ms").unwrap(),
            Duration::from_millis(1500)
        );
        assert_eq!(parse_duration("30s").unwrap(), Duration::from_secs(30));
        assert_eq!(parse_duration("30").unwrap(), Duration::from_secs(30));
        assert_eq!(parse_duration("5m").unwrap(), Duration::from_secs(300));
        assert_eq!(parse_duration("24h").unwrap(), Duration::from_secs(86_400));
        assert_eq!(parse_duration("2d").unwrap(), Duration::from_secs(172_800));
        assert!(parse_duration("soon").is_err());
        assert!(parse_duration("5w").is_err());
    }

    #[test]
    fn setting_values_are_validated() {
        assert_eq!(
            setting_value("http_retries", "5").unwrap().as_integer(),
            Some(5)
        );
        assert_eq!(
            setting_value("http_timeout", "10s").unwrap().as_str(),
            Some("10s")
        );
        assert_eq!(
            setting_value("color", "never").unwrap().as_str(),
            Some("never")
        );
        assert!(setting_value("http_retries", "-1").is_err());
        assert!(setting_value("color", "sometimes").is_err());
        assert!(setting_value("cache_ttl", "forever").is_err());
        assert!(setting_value("nope", "1").is_err());
    }
}
