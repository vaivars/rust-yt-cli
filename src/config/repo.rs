use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::error::{Result, YtError};
use crate::instance::InstanceUrl;

/// A repository's `.yt.toml` or `.config/yt.toml`: pins the instance and default project, never credentials.
#[derive(Clone, Debug)]
pub struct RepoConfig {
    pub path: PathBuf,
    pub url: Option<InstanceUrl>,
    pub project: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RepoFile {
    url: Option<String>,
    project: Option<String>,
}

/// The nearest repo config, searching from `start` up to the filesystem root.
pub fn find(start: &Path) -> Result<Option<RepoConfig>> {
    for dir in start.ancestors() {
        let dotted = dir.join(".yt.toml");
        let nested = dir.join(".config").join("yt.toml");
        match (dotted.is_file(), nested.is_file()) {
            (true, true) => {
                return Err(YtError::Config {
                    path: dir.display().to_string(),
                    message: "both .yt.toml and .config/yt.toml exist; keep one".into(),
                }
                .into());
            }
            (true, false) => return load(dotted).map(Some),
            (false, true) => return load(nested).map(Some),
            (false, false) => {}
        }
    }
    Ok(None)
}

fn load(path: PathBuf) -> Result<RepoConfig> {
    let invalid = |message: String| -> miette::Report {
        YtError::Config {
            path: path.display().to_string(),
            message,
        }
        .into()
    };
    let text = std::fs::read_to_string(&path).map_err(|error| invalid(error.to_string()))?;
    let file: RepoFile = toml::from_str(&text).map_err(|error| invalid(error.to_string()))?;
    if let Some(project) = &file.project {
        crate::fields::schema::check_short_name(project)
            .map_err(|error| invalid(error.to_string()))?;
    }
    let url = file.url.as_deref().map(InstanceUrl::parse).transpose()?;
    Ok(RepoConfig {
        path,
        url,
        project: file.project,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nearest_file_wins_and_both_forms_are_found() {
        let root = tempfile::tempdir().unwrap();
        let nested = root.path().join("a/b");
        std::fs::create_dir_all(nested.join(".config")).unwrap();
        std::fs::write(
            root.path().join(".yt.toml"),
            "url = \"https://outer.youtrack.cloud\"\n",
        )
        .unwrap();
        std::fs::write(
            nested.join(".config/yt.toml"),
            "url = \"https://inner.youtrack.cloud\"\nproject = \"WEB\"\n",
        )
        .unwrap();

        let found = find(&nested).unwrap().unwrap();
        assert_eq!(
            found.url.unwrap().to_string(),
            "https://inner.youtrack.cloud"
        );
        assert_eq!(found.project.as_deref(), Some("WEB"));

        let found = find(&root.path().join("a")).unwrap().unwrap();
        assert_eq!(
            found.url.unwrap().to_string(),
            "https://outer.youtrack.cloud"
        );
    }

    #[test]
    fn both_files_in_one_directory_is_an_error() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join(".config")).unwrap();
        std::fs::write(root.path().join(".yt.toml"), "").unwrap();
        std::fs::write(root.path().join(".config/yt.toml"), "").unwrap();
        assert!(
            find(root.path())
                .unwrap_err()
                .to_string()
                .contains("keep one")
        );
    }

    #[test]
    fn the_project_must_be_a_short_name() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join(".yt.toml"), "project = \"/tmp/x\"\n").unwrap();
        let error = find(root.path()).unwrap_err().to_string();
        assert!(error.contains("not a project short name"), "{error}");
    }

    #[test]
    fn profile_names_are_not_allowed_in_shared_files() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join(".yt.toml"), "profile = \"acme\"\n").unwrap();
        assert!(find(root.path()).is_err());
    }
}
