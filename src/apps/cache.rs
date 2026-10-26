use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::apps::registry::is_kebab;
use crate::instance::InstanceUrl;

/// App descriptions cached as fetched under `<cache>/<profile>/apps/<id>.json` for `cache_ttl`.
pub struct AppCache {
    dir: PathBuf,
    ttl: Duration,
}

impl AppCache {
    pub fn new(cache_dir: &Path, profile: &str, ttl: Duration) -> Self {
        Self {
            dir: crate::dirs::profile_cache(cache_dir, profile).join("apps"),
            ttl,
        }
    }

    /// The cache of one profile on one instance: profiles from `YOUTRACK_URL` share a name, not an instance.
    pub fn for_profile(
        cache_dir: &Path,
        profile: &str,
        instance: &InstanceUrl,
        ttl: Duration,
    ) -> Self {
        Self::new(cache_dir, &instance.account(profile), ttl)
    }

    fn path(&self, id: &str) -> Option<PathBuf> {
        is_kebab(id).then(|| self.dir.join(format!("{id}.json")))
    }

    /// The cached description while it is younger than the TTL.
    pub fn load(&self, id: &str) -> Option<String> {
        let path = self.path(id)?;
        let age = std::fs::metadata(&path)
            .ok()?
            .modified()
            .ok()?
            .elapsed()
            .ok()?;
        if age >= self.ttl {
            return None;
        }
        std::fs::read_to_string(path).ok()
    }

    /// The cached description however old it is, for completion, which never fetches.
    pub fn load_stale(&self, id: &str) -> Option<String> {
        std::fs::read_to_string(self.path(id)?).ok()
    }

    /// Best effort: a description that cannot be cached is fetched again next time.
    pub fn store(&self, id: &str, text: &str) {
        let Some(path) = self.path(id) else {
            return;
        };
        let write = || -> std::io::Result<()> {
            std::fs::create_dir_all(&self.dir)?;
            std::fs::write(path, text)
        };
        if let Err(error) = write() {
            log::warn!("could not cache the description of {id}: {error}");
        }
    }

    pub fn forget(&self, id: &str) {
        if let Some(path) = self.path(id)
            && let Err(error) = std::fs::remove_file(path)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            log::warn!("could not remove the cached description of {id}: {error}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descriptions_load_until_they_expire() {
        let dir = tempfile::tempdir().unwrap();
        let cache = AppCache::new(dir.path(), "acme", Duration::from_secs(3600));
        assert_eq!(cache.load("hidden-fields"), None);
        cache.store("hidden-fields", "{}");
        assert_eq!(cache.load("hidden-fields").as_deref(), Some("{}"));
        assert!(dir.path().join("acme/apps/hidden-fields.json").exists());
        let expired = AppCache::new(dir.path(), "acme", Duration::ZERO);
        assert_eq!(expired.load("hidden-fields"), None);
        assert_eq!(expired.load_stale("hidden-fields").as_deref(), Some("{}"));
        cache.forget("hidden-fields");
        assert_eq!(cache.load_stale("hidden-fields"), None);
        cache.forget("hidden-fields");
    }

    #[test]
    fn only_app_ids_reach_the_disk() {
        let dir = tempfile::tempdir().unwrap();
        let cache = AppCache::new(dir.path(), "acme", Duration::from_secs(3600));
        cache.store("../escaped", "{}");
        assert!(!dir.path().join("acme/escaped.json").exists());
        assert_eq!(cache.load_stale("../escaped"), None);
    }

    #[test]
    fn each_instance_gets_its_own_directory() {
        let dir = tempfile::tempdir().unwrap();
        let first = InstanceUrl::parse("https://acme.youtrack.cloud").unwrap();
        let second = InstanceUrl::parse("https://other.example.com/youtrack").unwrap();
        let ttl = Duration::from_secs(60);
        AppCache::for_profile(dir.path(), "YOUTRACK_URL", &first, ttl).store("hidden-fields", "{}");
        let other = AppCache::for_profile(dir.path(), "YOUTRACK_URL", &second, ttl);
        assert_eq!(other.load("hidden-fields"), None);
    }
}
