use serde::Deserialize;
use serde_json::json;
use url::Url;

use crate::api::issues::UserRef;
use crate::api::{Client, Parent, Projection};
use crate::error::Result;
use crate::fields::usage;
use crate::instance::InstanceUrl;

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Comment {
    pub id: String,
    pub text: Option<String>,
    pub author: Option<UserRef>,
    #[serde(default)]
    pub created: i64,
    pub updated: Option<i64>,
}

impl Projection for Comment {
    const FIELDS: &'static str = "id,text,author(login,fullName),created,updated";
}

pub async fn list(client: &Client, parent: &Parent) -> Result<Vec<Comment>> {
    client.get_all(&format!("{}/comments", parent.path())).await
}

pub async fn get(client: &Client, parent: &Parent, comment: &str) -> Result<Comment> {
    check_id(comment, parent)?;
    client
        .get(&format!("{}/comments/{comment}", parent.path()), &[])
        .await
}

pub async fn add(client: &Client, parent: &Parent, text: &str) -> Result<Comment> {
    client
        .post(
            &format!("{}/comments", parent.path()),
            &json!({"text": text}),
        )
        .await
}

pub async fn edit(client: &Client, parent: &Parent, comment: &str, text: &str) -> Result<Comment> {
    check_id(comment, parent)?;
    client
        .post(
            &format!("{}/comments/{comment}", parent.path()),
            &json!({"text": text}),
        )
        .await
}

pub async fn delete(client: &Client, parent: &Parent, comment: &str) -> Result<()> {
    check_id(comment, parent)?;
    client
        .delete(&format!("{}/comments/{comment}", parent.path()))
        .await
}

/// The issue or article page scrolled to the comment.
pub fn url(instance: &InstanceUrl, parent: &Parent, comment: &str) -> Url {
    let mut url = parent.url(instance);
    url.set_fragment(Some(&format!("focus=Comments-{comment}.0-0")));
    url
}

/// YouTrack entity IDs (`4-17`); dot segments or other characters could steer the request to another path.
pub fn check_id(id: &str, parent: &Parent) -> Result<()> {
    if id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        && id.chars().any(|c| c.is_ascii_alphanumeric())
    {
        Ok(())
    } else {
        Err(usage(format!(
            "'{id}' is not a comment ID; `yt {} comment list` shows them",
            parent.noun()
        )))
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use wiremock::matchers::{body_json, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::api::test_client;

    fn issue() -> Parent {
        Parent::Issue("WEB-1".into())
    }

    fn comment(id: &str, text: &str) -> serde_json::Value {
        json!({"id": id, "text": text, "author": {"login": "jane", "fullName": "Jane"}, "created": 1_000, "updated": null})
    }

    #[tokio::test]
    async fn comments_are_listed_added_edited_and_deleted() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/issues/WEB-1/comments"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!([comment("4-1", "First")])),
            )
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/api/issues/WEB-1/comments"))
            .and(body_json(json!({"text": "Hello"})))
            .respond_with(ResponseTemplate::new(200).set_body_json(comment("4-2", "Hello")))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/api/issues/WEB-1/comments/4-2"))
            .and(body_json(json!({"text": "Fixed"})))
            .respond_with(ResponseTemplate::new(200).set_body_json(comment("4-2", "Fixed")))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("DELETE"))
            .and(path("/api/issues/WEB-1/comments/4-2"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;
        let client = test_client(&server.uri());
        let listed = list(&client, &issue()).await.unwrap();
        assert_eq!(listed[0].id, "4-1");
        assert_eq!(
            listed[0]
                .author
                .as_ref()
                .map(|author| author.login.as_str()),
            Some("jane")
        );
        assert_eq!(add(&client, &issue(), "Hello").await.unwrap().id, "4-2");
        assert_eq!(
            edit(&client, &issue(), "4-2", "Fixed")
                .await
                .unwrap()
                .text
                .as_deref(),
            Some("Fixed")
        );
        delete(&client, &issue(), "4-2").await.unwrap();
    }

    #[test]
    fn dot_segments_are_not_comment_ids() {
        for id in [".", "..", "", "../x", "4_1"] {
            assert!(check_id(id, &issue()).is_err(), "{id:?}");
        }
        assert!(check_id("4-17", &issue()).is_ok());
    }

    #[tokio::test]
    async fn comment_ids_are_checked_before_they_reach_a_path() {
        let client = test_client("https://acme.youtrack.cloud");
        assert!(delete(&client, &issue(), "../../admin").await.is_err());
    }

    #[test]
    fn comment_urls_focus_the_comment() {
        let instance = InstanceUrl::parse("https://acme.youtrack.cloud").unwrap();
        assert_eq!(
            url(&instance, &issue(), "4-2").as_str(),
            "https://acme.youtrack.cloud/issue/WEB-1#focus=Comments-4-2.0-0"
        );
    }

    #[test]
    fn article_comment_urls_focus_the_comment_on_the_article() {
        let instance = InstanceUrl::parse("https://acme.youtrack.cloud").unwrap();
        assert_eq!(
            url(&instance, &Parent::Article("KB-A-1".into()), "190-0").as_str(),
            "https://acme.youtrack.cloud/articles/KB-A-1#focus=Comments-190-0.0-0"
        );
    }

    #[tokio::test]
    async fn article_comments_use_the_article_path() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/articles/KB-A-1/comments"))
            .and(body_json(json!({"text": "Hi"})))
            .respond_with(ResponseTemplate::new(200).set_body_json(comment("190-0", "Hi")))
            .expect(1)
            .mount(&server)
            .await;
        let added = add(
            &test_client(&server.uri()),
            &Parent::Article("KB-A-1".into()),
            "Hi",
        )
        .await
        .unwrap();
        assert_eq!(added.id, "190-0");
    }
}
