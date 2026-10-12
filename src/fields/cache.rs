use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::api::Client;
use crate::error::Result;
use crate::fields::schema::{self, ProjectSchema};
use crate::instance::InstanceUrl;

/// Project schemas cached under `<cache>/<profile>/schema/<project>.json` for `cache_ttl`.
pub struct SchemaCache {
    dir: PathBuf,
    ttl: Duration,
}

impl SchemaCache {
    pub fn new(cache_dir: &Path, profile: &str, ttl: Duration) -> Self {
        let profile: String = profile
            .chars()
            .map(|c| {
                if c.is_alphanumeric() || c == '-' || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        Self {
            dir: cache_dir.join(profile).join("schema"),
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

    /// None for anything but a project short name, so a name can never point outside the cache.
    fn path(&self, project: &str) -> Option<PathBuf> {
        schema::check_short_name(project).ok()?;
        Some(self.dir.join(format!("{project}.json")))
    }

    /// The cached schema while it is younger than the TTL.
    pub fn load(&self, project: &str) -> Option<ProjectSchema> {
        let path = self.path(project)?;
        let age = std::fs::metadata(&path)
            .ok()?
            .modified()
            .ok()?
            .elapsed()
            .ok()?;
        if age >= self.ttl {
            return None;
        }
        serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
    }

    /// Best effort: a schema that cannot be cached is fetched again next time.
    pub fn store(&self, schema: &ProjectSchema) {
        let Some(path) = self.path(&schema.project) else {
            return;
        };
        let write = || -> std::io::Result<()> {
            std::fs::create_dir_all(&self.dir)?;
            std::fs::write(path, serde_json::to_vec(schema)?)
        };
        if let Err(error) = write() {
            log::warn!("could not cache the fields of {}: {error}", schema.project);
        }
    }

    /// Fetches the schema and caches it.
    pub async fn refresh(&self, client: &Client, project: &str) -> Result<ProjectSchema> {
        let schema = schema::fetch(client, project).await?;
        self.store(&schema);
        Ok(schema)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::api::test_client;

    fn schema() -> ProjectSchema {
        ProjectSchema {
            project: "WEB".into(),
            fields: vec![],
        }
    }

    #[test]
    fn stored_schemas_load_until_they_expire() {
        let dir = tempfile::tempdir().unwrap();
        let cache = SchemaCache::new(dir.path(), "acme", Duration::from_secs(3600));
        assert_eq!(cache.load("WEB"), None);
        cache.store(&schema());
        assert_eq!(cache.load("WEB"), Some(schema()));
        assert!(dir.path().join("acme/schema/WEB.json").exists());
        let expired = SchemaCache::new(dir.path(), "acme", Duration::ZERO);
        assert_eq!(expired.load("WEB"), None);
    }

    #[test]
    fn each_instance_gets_its_own_directory() {
        let dir = tempfile::tempdir().unwrap();
        let first = crate::instance::InstanceUrl::parse("https://acme.youtrack.cloud").unwrap();
        let second =
            crate::instance::InstanceUrl::parse("https://other.example.com/youtrack").unwrap();
        SchemaCache::for_profile(dir.path(), "YOUTRACK_URL", &first, Duration::from_secs(60))
            .store(&schema());
        let other =
            SchemaCache::for_profile(dir.path(), "YOUTRACK_URL", &second, Duration::from_secs(60));
        assert_eq!(other.load("WEB"), None);
        other.store(&schema());
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 2);
    }

    #[test]
    fn only_project_short_names_reach_the_disk() {
        let dir = tempfile::tempdir().unwrap();
        let cache = SchemaCache::new(dir.path(), "acme", Duration::from_secs(3600));
        cache.store(&schema());
        let outside = dir.path().join("planted");
        std::fs::copy(
            dir.path().join("acme/schema/WEB.json"),
            outside.with_extension("json"),
        )
        .unwrap();
        assert_eq!(cache.load(outside.to_str().unwrap()), None);
        assert_eq!(cache.load("../../planted"), None);
        cache.store(&ProjectSchema {
            project: "../escaped".into(),
            fields: vec![],
        });
        assert!(!dir.path().join("acme/escaped.json").exists());
    }

    #[test]
    fn profile_names_become_safe_directory_names() {
        let dir = tempfile::tempdir().unwrap();
        SchemaCache::new(dir.path(), "a/b", Duration::from_secs(60)).store(&schema());
        assert!(dir.path().join("a_b/schema/WEB.json").exists());
    }

    #[tokio::test]
    async fn refresh_fetches_and_stores() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/admin/projects/WEB/customFields"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/issues"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
            .mount(&server)
            .await;
        let dir = tempfile::tempdir().unwrap();
        let cache = SchemaCache::new(dir.path(), "acme", Duration::from_secs(60));
        let fetched = cache
            .refresh(&test_client(&server.uri()), "WEB")
            .await
            .unwrap();
        assert_eq!(fetched, schema());
        assert_eq!(cache.load("WEB"), Some(schema()));
    }
}
