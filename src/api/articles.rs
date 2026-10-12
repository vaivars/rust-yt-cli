use futures::Stream;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::api::issues::{ProjectRef, UserRef};
use crate::api::{Client, Projection};
use crate::error::Result;

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArticleRef {
    pub id_readable: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArticleRow {
    pub id_readable: String,
    pub summary: Option<String>,
    pub project: Option<ProjectRef>,
    pub parent_article: Option<ArticleRef>,
    #[serde(default)]
    pub child_articles: Vec<ArticleRef>,
    #[serde(default)]
    pub updated: i64,
}

impl Projection for ArticleRow {
    const FIELDS: &'static str = "idReadable,summary,project(shortName,name),parentArticle(idReadable),childArticles(idReadable),updated";
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Article {
    pub id_readable: String,
    pub summary: Option<String>,
    pub project: Option<ProjectRef>,
    pub parent_article: Option<ArticleRef>,
    #[serde(default)]
    pub child_articles: Vec<ArticleRef>,
    #[serde(default)]
    pub updated: i64,
    pub content: Option<String>,
    pub reporter: Option<UserRef>,
    #[serde(default)]
    pub created: i64,
}

impl Projection for Article {
    const FIELDS: &'static str = "idReadable,summary,project(shortName,name),parentArticle(idReadable),childArticles(idReadable),updated,content,reporter(login,fullName),created";
}

impl Article {
    pub fn row(&self) -> ArticleRow {
        ArticleRow {
            id_readable: self.id_readable.clone(),
            summary: self.summary.clone(),
            project: self.project.clone(),
            parent_article: self.parent_article.clone(),
            child_articles: self.child_articles.clone(),
            updated: self.updated,
        }
    }
}

/// Articles matching a YouTrack query, most recently updated first (YouTrack's order).
pub fn search<'a>(
    client: &'a Client,
    query: &str,
    page_size: usize,
) -> impl Stream<Item = Result<ArticleRow>> + Send + 'a {
    let mut parameters = Vec::new();
    if !query.is_empty() {
        parameters.push(("query".to_string(), query.to_string()));
    }
    client.paginate("articles".to_string(), parameters, page_size)
}

pub async fn get(client: &Client, id: &str) -> Result<Article> {
    client.get(&format!("articles/{id}"), &[]).await
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Created {
    id_readable: String,
}

impl Projection for Created {
    const FIELDS: &'static str = "idReadable";
}

/// Creates an article and returns its readable ID.
pub async fn create(
    client: &Client,
    project: &str,
    summary: &str,
    content: Option<&str>,
    parent: Option<&str>,
) -> Result<String> {
    let mut body = json!({
        "project": {"shortName": project},
        "summary": summary,
    });
    if let Some(content) = content {
        body["content"] = json!(content);
    }
    if let Some(parent) = parent {
        body["parentArticle"] = json!({"idReadable": parent});
    }
    let created: Created = client.post("articles", &body).await?;
    Ok(created.id_readable)
}

/// The fields of `POST articles/{id}`; `parent: Some(None)` moves the article to the top level.
pub fn update_body(
    summary: Option<&str>,
    content: Option<&str>,
    parent: Option<Option<&str>>,
) -> Value {
    let mut body = Map::new();
    if let Some(summary) = summary {
        body.insert("summary".into(), json!(summary));
    }
    if let Some(content) = content {
        body.insert("content".into(), json!(content));
    }
    match parent {
        Some(Some(parent)) => {
            body.insert("parentArticle".into(), json!({"idReadable": parent}));
        }
        Some(None) => {
            body.insert("parentArticle".into(), Value::Null);
        }
        None => {}
    }
    Value::Object(body)
}

pub async fn update(client: &Client, id: &str, body: &Value) -> Result<()> {
    client.post_unit(&format!("articles/{id}"), body).await
}

pub async fn delete(client: &Client, id: &str) -> Result<()> {
    client.delete(&format!("articles/{id}")).await
}

#[cfg(test)]
mod tests {
    use futures::TryStreamExt as _;
    use serde_json::json;
    use wiremock::matchers::{body_json, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::api::test_client;

    #[test]
    fn update_bodies_carry_only_what_changes() {
        assert_eq!(
            update_body(Some("Title"), None, None),
            json!({"summary": "Title"})
        );
        assert_eq!(
            update_body(None, Some("Text"), Some(None)),
            json!({"content": "Text", "parentArticle": null})
        );
        assert_eq!(
            update_body(None, None, Some(Some("KB-A-1"))),
            json!({"parentArticle": {"idReadable": "KB-A-1"}})
        );
    }

    #[tokio::test]
    async fn articles_are_searched_created_read_updated_and_deleted() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/articles"))
            .and(query_param("query", "project: {KB}"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([
                {"idReadable": "KB-A-2", "summary": "Child", "project": {"shortName": "KB", "name": "Knowledge"}, "parentArticle": {"idReadable": "KB-A-1"}, "childArticles": [], "updated": 2}
            ])))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/api/articles"))
            .and(body_json(json!({
                "project": {"shortName": "KB"},
                "summary": "Child",
                "content": "Text",
                "parentArticle": {"idReadable": "KB-A-1"}
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"idReadable": "KB-A-2"})))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/articles/KB-A-2"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "idReadable": "KB-A-2", "summary": "Child", "project": {"shortName": "KB", "name": "Knowledge"},
                "parentArticle": {"idReadable": "KB-A-1"}, "childArticles": [], "updated": 2,
                "content": "Text", "reporter": {"login": "jane", "fullName": "Jane"}, "created": 1
            })))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/api/articles/KB-A-2"))
            .and(body_json(json!({"summary": "Renamed"})))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("DELETE"))
            .and(path("/api/articles/KB-A-2"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;
        let client = test_client(&server.uri());
        let rows: Vec<ArticleRow> = search(&client, "project: {KB}", 50)
            .try_collect()
            .await
            .unwrap();
        assert_eq!(
            rows[0].parent_article.as_ref().unwrap().id_readable,
            "KB-A-1"
        );
        let id = create(&client, "KB", "Child", Some("Text"), Some("KB-A-1"))
            .await
            .unwrap();
        assert_eq!(id, "KB-A-2");
        let article = get(&client, "KB-A-2").await.unwrap();
        assert_eq!(article.content.as_deref(), Some("Text"));
        assert_eq!(article.row().id_readable, "KB-A-2");
        update(&client, "KB-A-2", &update_body(Some("Renamed"), None, None))
            .await
            .unwrap();
        delete(&client, "KB-A-2").await.unwrap();
    }
}
