use std::path::{Path, PathBuf};

use crate::env::Env;
use crate::error::{Result, YtError};

/// Where yt keeps its files: XDG locations on every OS, except caches on macOS.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dirs {
    pub config: PathBuf,
    pub cache: PathBuf,
    pub state: PathBuf,
}

#[cfg(target_os = "macos")]
const CACHE_FALLBACK: &str = "Library/Caches";
#[cfg(not(target_os = "macos"))]
const CACHE_FALLBACK: &str = ".cache";

impl Dirs {
    pub fn from_env(env: &Env) -> Result<Self> {
        let home = env.path("HOME");
        let base = |xdg: &str, fallback: &str| -> Result<PathBuf> {
            env.path(xdg)
                .or_else(|| home.as_ref().map(|home| home.join(fallback)))
                .ok_or_else(|| YtError::NoHome.into())
        };
        let config = match env.path("YT_CONFIG_DIR") {
            Some(path) => path,
            None => base("XDG_CONFIG_HOME", ".config")?.join("yt"),
        };
        let cache = match env.path("YT_CACHE_DIR") {
            Some(path) => path,
            None => base("XDG_CACHE_HOME", CACHE_FALLBACK)?.join("yt"),
        };
        let state = match env.path("YT_STATE_DIR") {
            Some(path) => path,
            None => base("XDG_STATE_HOME", ".local/state")?.join("yt"),
        };
        Ok(Self {
            config,
            cache,
            state,
        })
    }

    pub fn config_file(&self) -> PathBuf {
        self.config.join("config.toml")
    }
}

/// The cache directory of one profile, with the name made safe to use as a directory.
pub fn profile_cache(cache: &Path, profile: &str) -> PathBuf {
    let name: String = profile
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    cache.join(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_follow_xdg_under_home() {
        let dirs = Dirs::from_env(&Env::from_vars([("HOME", "/home/jane")])).unwrap();
        assert_eq!(dirs.config, PathBuf::from("/home/jane/.config/yt"));
        assert_eq!(dirs.state, PathBuf::from("/home/jane/.local/state/yt"));
        assert_eq!(
            dirs.config_file(),
            PathBuf::from("/home/jane/.config/yt/config.toml")
        );
        #[cfg(target_os = "macos")]
        assert_eq!(dirs.cache, PathBuf::from("/home/jane/Library/Caches/yt"));
        #[cfg(not(target_os = "macos"))]
        assert_eq!(dirs.cache, PathBuf::from("/home/jane/.cache/yt"));
    }

    #[test]
    fn xdg_variables_win_over_home() {
        let env = Env::from_vars([
            ("HOME", "/home/jane"),
            ("XDG_CONFIG_HOME", "/x/config"),
            ("XDG_CACHE_HOME", "/x/cache"),
            ("XDG_STATE_HOME", "/x/state"),
        ]);
        let dirs = Dirs::from_env(&env).unwrap();
        assert_eq!(dirs.config, PathBuf::from("/x/config/yt"));
        assert_eq!(dirs.cache, PathBuf::from("/x/cache/yt"));
        assert_eq!(dirs.state, PathBuf::from("/x/state/yt"));
    }

    #[test]
    fn yt_variables_win_over_everything() {
        let env = Env::from_vars([
            ("XDG_CONFIG_HOME", "/x/config"),
            ("YT_CONFIG_DIR", "/y/config"),
            ("YT_CACHE_DIR", "/y/cache"),
            ("YT_STATE_DIR", "/y/state"),
        ]);
        let dirs = Dirs::from_env(&env).unwrap();
        assert_eq!(dirs.config, PathBuf::from("/y/config"));
        assert_eq!(dirs.cache, PathBuf::from("/y/cache"));
        assert_eq!(dirs.state, PathBuf::from("/y/state"));
    }

    #[test]
    fn missing_home_is_an_error() {
        let error = Dirs::from_env(&Env::from_vars([("X", "y")])).unwrap_err();
        assert!(matches!(
            error.downcast_ref::<YtError>(),
            Some(YtError::NoHome)
        ));
    }
}
