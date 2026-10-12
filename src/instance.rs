use std::fmt;

use url::Url;

use crate::error::{Result, YtError};

/// The address a user opens YouTrack at, including any context path (`https://host/youtrack`).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct InstanceUrl {
    base: Url,
}

impl InstanceUrl {
    pub fn parse(input: &str) -> Result<Self> {
        let input = input.trim();
        let invalid = |reason: &str| -> miette::Report {
            YtError::Usage {
                message: format!("'{input}' is not a valid YouTrack URL: {reason}"),
            }
            .into()
        };
        let candidate = if input.contains("://") {
            input.to_string()
        } else {
            format!("https://{input}")
        };
        let mut url = Url::parse(&candidate).map_err(|error| invalid(&error.to_string()))?;
        if !matches!(url.scheme(), "https" | "http") {
            return Err(invalid("use an http or https address"));
        }
        if url.host_str().is_none() {
            return Err(invalid("there is no host"));
        }
        url.set_query(None);
        url.set_fragment(None);
        let _ = url.set_username("");
        let _ = url.set_password(None);
        let trimmed = url.path().trim_end_matches('/');
        let path = trimmed.strip_suffix("/api").unwrap_or(trimmed).to_string();
        url.set_path(&format!("{path}/"));
        Ok(Self { base: url })
    }

    pub fn host(&self) -> &str {
        self.base.host_str().unwrap_or_default()
    }

    /// The keyring account for `login` on this instance, e.g. `jane@acme.youtrack.cloud`.
    pub fn account(&self, login: &str) -> String {
        let address = self.to_string();
        let without_scheme = address
            .split_once("://")
            .map_or(address.as_str(), |(_, rest)| rest);
        format!("{login}@{without_scheme}")
    }

    pub fn api(&self) -> Url {
        self.join("api/")
    }

    pub fn origin(&self) -> Url {
        self.join("/")
    }

    pub fn join(&self, relative: &str) -> Url {
        self.base
            .join(relative)
            .unwrap_or_else(|_| self.base.clone())
    }

    pub fn issue_url(&self, id: &str) -> Url {
        self.join(&format!("issue/{id}"))
    }

    pub fn project_url(&self, short_name: &str) -> Url {
        self.join(&format!("projects/{short_name}"))
    }

    pub fn profile_name_hint(&self) -> String {
        self.host()
            .split('.')
            .next()
            .unwrap_or("default")
            .to_string()
    }

    pub fn article_url(&self, id: &str) -> Url {
        self.join(&format!("articles/{id}"))
    }

    pub fn board_url(&self, id: &str) -> Url {
        self.join(&format!("agiles/{id}/current"))
    }

    /// Accepts `WEB-42` or this instance's issue URL, e.g. `https://acme.youtrack.cloud/issue/WEB-42/slug`.
    pub fn issue_id(&self, input: &str) -> Result<String> {
        self.readable_id(
            input,
            "issue",
            "an issue ID like WEB-42 or an issue URL",
            is_issue_id,
        )
    }

    /// Accepts `KB-A-12` or this instance's article URL, e.g. `https://acme.youtrack.cloud/articles/KB-A-12/slug`.
    pub fn article_id(&self, input: &str) -> Result<String> {
        let id = self.readable_id(
            input,
            "articles",
            "an article ID like KB-A-12 or an article URL",
            is_article_id,
        )?;
        // YouTrack matches the project part in any case but finds the article only with an uppercase `A`.
        let (prefix, number) = id.rsplit_once('-').unwrap_or((&id, ""));
        let project = &prefix[..prefix.len() - 2];
        Ok(format!("{project}-A-{number}"))
    }

    fn readable_id(
        &self,
        input: &str,
        segment: &str,
        expected: &str,
        valid: fn(&str) -> bool,
    ) -> Result<String> {
        let input = input.trim();
        let not_an_id = || -> miette::Report {
            YtError::Usage {
                message: format!("'{input}' is not {expected}"),
            }
            .into()
        };
        let id = if input.contains("://") {
            let url = Url::parse(input).map_err(|_| not_an_id())?;
            if url.host_str() != self.base.host_str() {
                return Err(YtError::Usage {
                    message: format!("{input} belongs to a different instance than {self}"),
                }
                .into());
            }
            let rest = url
                .path()
                .strip_prefix(self.base.path())
                .unwrap_or(url.path());
            let mut segments = rest.split('/').filter(|part| !part.is_empty());
            match (segments.next(), segments.next()) {
                (Some(found), Some(id)) if found == segment => id.to_string(),
                _ => return Err(not_an_id()),
            }
        } else {
            input.to_string()
        };
        if valid(&id) { Ok(id) } else { Err(not_an_id()) }
    }
}

fn is_issue_id(id: &str) -> bool {
    let Some((project, number)) = id.rsplit_once('-') else {
        return false;
    };
    project
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic())
        && project
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_')
        && !number.is_empty()
        && number.chars().all(|c| c.is_ascii_digit())
}

/// `KB-A-12`: a project, `A` for article, a number.
fn is_article_id(id: &str) -> bool {
    id.rsplit_once('-')
        .and_then(|(prefix, number)| {
            let project = prefix
                .strip_suffix("-A")
                .or_else(|| prefix.strip_suffix("-a"))?;
            Some(is_issue_id(&format!("{project}-{number}")))
        })
        .unwrap_or(false)
}

impl fmt::Display for InstanceUrl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.base.as_str().trim_end_matches('/'))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalises_what_users_type() {
        for input in [
            "acme.youtrack.cloud",
            "https://acme.youtrack.cloud",
            "https://acme.youtrack.cloud/",
            "https://acme.youtrack.cloud/api",
            "https://acme.youtrack.cloud/api/",
            " https://acme.youtrack.cloud?x=1#y ",
        ] {
            assert_eq!(
                InstanceUrl::parse(input).unwrap().to_string(),
                "https://acme.youtrack.cloud",
                "{input}"
            );
        }
    }

    #[test]
    fn keeps_a_context_path() {
        let url = InstanceUrl::parse("https://youtrack.example.com/youtrack/api").unwrap();
        assert_eq!(url.to_string(), "https://youtrack.example.com/youtrack");
        assert_eq!(
            url.api().as_str(),
            "https://youtrack.example.com/youtrack/api/"
        );
        assert_eq!(url.origin().as_str(), "https://youtrack.example.com/");
        assert_eq!(
            url.issue_url("WEB-1").as_str(),
            "https://youtrack.example.com/youtrack/issue/WEB-1"
        );
        assert_eq!(
            url.join("users/me?tab=account-security").as_str(),
            "https://youtrack.example.com/youtrack/users/me?tab=account-security"
        );
        assert_eq!(url.account("jane"), "jane@youtrack.example.com/youtrack");
    }

    #[test]
    fn account_and_name_hint_for_a_cloud_instance() {
        let url = InstanceUrl::parse("acme.youtrack.cloud").unwrap();
        assert_eq!(url.account("jane"), "jane@acme.youtrack.cloud");
        assert_eq!(url.profile_name_hint(), "acme");
    }

    #[test]
    fn rejects_other_schemes() {
        assert!(InstanceUrl::parse("ftp://acme.youtrack.cloud").is_err());
    }

    #[test]
    fn issue_ids_and_issue_urls() {
        let url = InstanceUrl::parse("https://host.example/youtrack").unwrap();
        assert_eq!(url.issue_id("WEB-42").unwrap(), "WEB-42");
        assert_eq!(
            url.issue_id("https://host.example/youtrack/issue/WEB-42/login-fails")
                .unwrap(),
            "WEB-42"
        );
        assert!(
            url.issue_id("https://other.example/youtrack/issue/WEB-42")
                .is_err()
        );
        assert!(url.issue_id("WEB").is_err());
        assert!(url.issue_id("42").is_err());
    }

    #[test]
    fn article_ids_and_article_urls() {
        let url = InstanceUrl::parse("https://host.example/youtrack").unwrap();
        assert_eq!(url.article_id("KB-A-12").unwrap(), "KB-A-12");
        assert_eq!(
            url.article_id("https://host.example/youtrack/articles/KB-A-12/getting-started")
                .unwrap(),
            "KB-A-12"
        );
        assert_eq!(url.article_id("kb-a-12").unwrap(), "kb-A-12");
        assert!(url.article_id("KB-12").is_err());
        assert!(
            url.article_id("https://host.example/youtrack/issue/KB-A-12")
                .is_err()
        );
        assert!(
            url.article_id("https://other.example/youtrack/articles/KB-A-12")
                .is_err()
        );
        assert_eq!(
            url.article_url("KB-A-12").as_str(),
            "https://host.example/youtrack/articles/KB-A-12"
        );
    }
}
