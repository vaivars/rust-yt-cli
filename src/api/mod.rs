pub mod agiles;
pub mod articles;
pub mod attachments;
pub mod client;
pub mod commands;
pub mod comments;
pub mod custom_fields;
pub mod error;
pub mod issues;
pub mod links;
pub mod parent;
pub mod projection;
pub mod projects;
pub mod tags;
pub mod users;
pub mod work_items;

pub use client::{Client, TokenSource};
pub use parent::Parent;
pub use projection::Projection;

#[cfg(test)]
pub(crate) fn test_client(uri: &str) -> Client {
    let http = crate::http::client(&crate::http::HttpConfig {
        timeout: std::time::Duration::from_secs(5),
        retries: 0,
        ca_file: None,
    })
    .unwrap();
    let token = std::sync::Arc::new(crate::auth::token_source::StaticToken::new(
        secrecy::SecretString::from("perm-dGVzdA==.dGVzdA==.secret123456".to_string()),
        "acme",
    ));
    Client::new(
        http,
        crate::instance::InstanceUrl::parse(uri).unwrap(),
        token,
        0,
    )
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};
    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::api::client::LIST_PAGE;

    async fn two_pages(server: &MockServer, endpoint: &str, item: Value) {
        Mock::given(method("GET"))
            .and(path(endpoint))
            .and(query_param("$skip", "0"))
            .respond_with(ResponseTemplate::new(200).set_body_json(vec![item.clone(); LIST_PAGE]))
            .mount(server)
            .await;
        Mock::given(method("GET"))
            .and(path(endpoint))
            .and(query_param("$skip", LIST_PAGE.to_string()))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([item])))
            .mount(server)
            .await;
    }

    #[tokio::test]
    async fn lists_read_every_page() {
        let server = MockServer::start().await;
        let issue = "/api/issues/WEB-1";
        two_pages(&server, &format!("{issue}/comments"), json!({"id": "4-1"})).await;
        two_pages(
            &server,
            &format!("{issue}/attachments"),
            json!({"id": "8-1", "name": "a.txt"}),
        )
        .await;
        two_pages(
            &server,
            &format!("{issue}/links"),
            json!({"direction": "OUTWARD", "linkType": {"name": "Relates"}}),
        )
        .await;
        two_pages(&server, "/api/tags", json!({"name": "ui"})).await;
        two_pages(&server, "/api/admin/projects", json!({"shortName": "WEB"})).await;
        let client = test_client(&server.uri());
        let all = LIST_PAGE + 1;
        assert_eq!(
            comments::list(&client, &Parent::Issue("WEB-1".into()))
                .await
                .unwrap()
                .len(),
            all
        );
        assert_eq!(
            attachments::list(&client, &Parent::Issue("WEB-1".into()))
                .await
                .unwrap()
                .len(),
            all
        );
        assert_eq!(links::list(&client, "WEB-1").await.unwrap().len(), all);
        assert_eq!(tags::visible(&client).await.unwrap().len(), all);
        assert_eq!(projects::short_names(&client).await.unwrap().len(), all);
    }
}
