use std::path::PathBuf;

use miette::IntoDiagnostic as _;
use serde::Deserialize;

use crate::api::issues::UserRef;
use crate::api::{Client, Parent, Projection};
use crate::error::{Result, YtError};
use crate::fields::{choices, usage};

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Attachment {
    pub id: String,
    pub name: String,
    pub size: Option<i64>,
    pub mime_type: Option<String>,
    pub author: Option<UserRef>,
    pub created: Option<i64>,
    pub url: Option<String>,
}

impl Projection for Attachment {
    const FIELDS: &'static str = "id,name,size,mimeType,author(login,fullName),created,url";
}

pub async fn list(client: &Client, parent: &Parent) -> Result<Vec<Attachment>> {
    client
        .get_all(&format!("{}/attachments", parent.path()))
        .await
}

/// Uploads files to an issue or article in one request.
pub async fn upload(
    client: &Client,
    parent: &Parent,
    files: &[PathBuf],
) -> Result<Vec<Attachment>> {
    let paths = files.to_vec();
    let parts = tokio::task::spawn_blocking(move || -> Result<Vec<(String, Vec<u8>)>> {
        paths
            .iter()
            .map(|path| {
                let data = std::fs::read(path)
                    .map_err(|error| usage(format!("cannot read {}: {error}", path.display())))?;
                let name = path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "file".into());
                Ok((name, data))
            })
            .collect()
    })
    .await
    .into_diagnostic()??;
    client
        .upload(&format!("{}/attachments", parent.path()), &parts)
        .await
}

/// The attachment's content, byte for byte.
pub async fn download(client: &Client, attachment: &Attachment) -> Result<Vec<u8>> {
    let url = attachment.url.as_deref().ok_or_else(|| {
        usage(format!(
            "YouTrack gave no download address for {}",
            attachment.name
        ))
    })?;
    let mut response = client.download(url).await?;
    let mut data = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| YtError::Network(crate::redact::redact(&error.to_string())))?
    {
        data.extend_from_slice(&chunk);
    }
    Ok(data)
}

pub async fn delete(client: &Client, parent: &Parent, attachment: &str) -> Result<()> {
    if !attachment
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | '_'))
    {
        return Err(usage(format!("'{attachment}' is not an attachment ID")));
    }
    client
        .delete(&format!("{}/attachments/{attachment}", parent.path()))
        .await
}

/// An attachment by ID, or by a name only one attachment has.
pub fn find<'a>(
    attachments: &'a [Attachment],
    name_or_id: &str,
    parent: &Parent,
) -> Result<&'a Attachment> {
    if let Some(found) = attachments
        .iter()
        .find(|attachment| attachment.id == name_or_id)
    {
        return Ok(found);
    }
    let named: Vec<&Attachment> = attachments
        .iter()
        .filter(|attachment| attachment.name == name_or_id)
        .collect();
    match named.as_slice() {
        [one] => Ok(one),
        [] => {
            let names: Vec<String> = attachments
                .iter()
                .map(|attachment| attachment.name.clone())
                .collect();
            Err(usage(if names.is_empty() {
                format!(
                    "there is no attachment '{name_or_id}'; the {} has none",
                    parent.noun()
                )
            } else {
                format!(
                    "there is no attachment '{name_or_id}'; attachments: {}",
                    choices(&names)
                )
            }))
        }
        several => Err(usage(format!(
            "{} attachments are named '{name_or_id}'; pick one by ID: {}",
            several.len(),
            several
                .iter()
                .map(|attachment| attachment.id.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ))),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use wiremock::matchers::{body_string_contains, header_regex, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::api::test_client;

    fn attachment(id: &str, name: &str) -> Attachment {
        Attachment {
            id: id.into(),
            name: name.into(),
            size: Some(5),
            mime_type: None,
            author: None,
            created: None,
            url: Some(format!("/api/files/{id}?sign=x")),
        }
    }

    #[tokio::test]
    async fn files_upload_in_one_multipart_request() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/issues/WEB-1/attachments"))
            .and(header_regex("content-type", "^multipart/form-data"))
            .and(body_string_contains("hello attachment"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!([{"id": "1-1", "name": "log.txt", "size": 16}])),
            )
            .expect(1)
            .mount(&server)
            .await;
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("log.txt");
        std::fs::write(&file, "hello attachment").unwrap();
        let client = test_client(&server.uri());
        let uploaded = upload(&client, &Parent::Issue("WEB-1".into()), &[file])
            .await
            .unwrap();
        assert_eq!(uploaded[0].name, "log.txt");
        assert!(client.has_written());
    }

    #[tokio::test]
    async fn a_missing_file_names_its_path() {
        let client = test_client("https://acme.youtrack.cloud");
        let error = upload(
            &client,
            &Parent::Issue("WEB-1".into()),
            &[PathBuf::from("/nonexistent/x.bin")],
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("/nonexistent/x.bin"), "{error}");
    }

    #[tokio::test]
    async fn downloads_keep_every_byte() {
        let server = MockServer::start().await;
        let bytes = vec![0_u8, 159, 146, 150, 255, 10, 13];
        Mock::given(method("GET"))
            .and(path("/api/files/1-1"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(bytes.clone()))
            .mount(&server)
            .await;
        let data = download(&test_client(&server.uri()), &attachment("1-1", "data.bin"))
            .await
            .unwrap();
        assert_eq!(data, bytes);
    }

    #[tokio::test]
    async fn a_slow_download_outlasts_the_request_timeout() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/files/1-1"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_bytes(vec![1_u8, 2, 3])
                    .set_delay(std::time::Duration::from_millis(1500)),
            )
            .mount(&server)
            .await;
        let http = crate::http::client(&crate::http::HttpConfig {
            timeout: std::time::Duration::from_millis(500),
            retries: 0,
            ca_file: None,
        })
        .unwrap();
        let token = std::sync::Arc::new(crate::auth::token_source::StaticToken::new(
            secrecy::SecretString::from("perm-c2xvdw==.dGVzdA==.secret123456".to_string()),
            "acme",
        ));
        let client = Client::new(
            http,
            crate::instance::InstanceUrl::parse(&server.uri()).unwrap(),
            token,
            0,
        );
        assert_eq!(
            download(&client, &attachment("1-1", "data.bin"))
                .await
                .unwrap(),
            [1, 2, 3]
        );
    }

    #[test]
    fn attachments_are_found_by_id_or_unique_name() {
        let attachments = vec![
            attachment("1-1", "a.txt"),
            attachment("1-2", "b.txt"),
            attachment("1-3", "b.txt"),
        ];
        assert_eq!(
            find(&attachments, "a.txt", &Parent::Issue("WEB-1".into()))
                .unwrap()
                .id,
            "1-1"
        );
        assert_eq!(
            find(&attachments, "1-3", &Parent::Issue("WEB-1".into()))
                .unwrap()
                .id,
            "1-3"
        );
        let error = find(&attachments, "b.txt", &Parent::Issue("WEB-1".into())).unwrap_err();
        assert!(error.to_string().contains("1-2, 1-3"), "{error}");
        assert!(find(&attachments, "c.txt", &Parent::Issue("WEB-1".into())).is_err());
    }
}
