use serde::Deserialize;
use url::Url;

use crate::api::{Client, Projection};
use crate::error::Result;
use crate::instance::InstanceUrl;

/// What comments and attachments belong to, by readable ID.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Parent {
    Issue(String),
    Article(String),
}

impl Parent {
    pub fn id(&self) -> &str {
        match self {
            Self::Issue(id) | Self::Article(id) => id,
        }
    }

    /// The REST path of the issue or article: `issues/WEB-1`, `articles/KB-A-1`.
    pub fn path(&self) -> String {
        match self {
            Self::Issue(id) => format!("issues/{id}"),
            Self::Article(id) => format!("articles/{id}"),
        }
    }

    pub fn url(&self, instance: &InstanceUrl) -> Url {
        match self {
            Self::Issue(id) => instance.issue_url(id),
            Self::Article(id) => instance.article_url(id),
        }
    }

    pub fn noun(&self) -> &'static str {
        match self {
            Self::Issue(_) => "issue",
            Self::Article(_) => "article",
        }
    }
}

#[derive(Deserialize)]
struct Found {}

impl Projection for Found {
    const FIELDS: &'static str = "id";
}

/// Fails unless the issue or article exists; checked before opening an editor so no writing is lost.
pub async fn exists(client: &Client, parent: &Parent) -> Result<()> {
    client.get::<Found>(&parent.path(), &[]).await.map(drop)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parents_name_their_paths_and_pages() {
        let instance = InstanceUrl::parse("https://acme.youtrack.cloud").unwrap();
        let issue = Parent::Issue("WEB-1".into());
        let article = Parent::Article("KB-A-1".into());
        assert_eq!(issue.path(), "issues/WEB-1");
        assert_eq!(article.path(), "articles/KB-A-1");
        assert_eq!(article.id(), "KB-A-1");
        assert_eq!(issue.noun(), "issue");
        assert_eq!(
            article.url(&instance).as_str(),
            "https://acme.youtrack.cloud/articles/KB-A-1"
        );
    }
}
