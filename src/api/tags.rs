use serde::Deserialize;
use serde_json::json;

use crate::api::{Client, Projection};
use crate::error::Result;
use crate::fields::{choices, closest, same, usage};

#[derive(Deserialize)]
struct Tag {
    name: String,
}

impl Projection for Tag {
    const FIELDS: &'static str = "name";
}

/// Names of the tags the user can see.
pub async fn visible(client: &Client) -> Result<Vec<String>> {
    let tags: Vec<Tag> = client.get_all("tags").await?;
    Ok(tags.into_iter().map(|tag| tag.name).collect())
}

/// Creates a tag owned by the user.
pub async fn create(client: &Client, name: &str) -> Result<()> {
    client.post_unit("tags", &json!({"name": name})).await
}

/// The instance's spelling of a tag name; `hint` ends the error for an unknown name.
pub fn resolve(tags: &[String], name: &str, hint: &str) -> Result<String> {
    if let Some(tag) = tags.iter().find(|tag| same(tag, name)) {
        return Ok(tag.clone());
    }
    let mut message = format!("there is no tag '{name}'");
    if let Some(close) = closest(name, tags.iter().map(String::as_str)) {
        message.push_str(&format!(" (did you mean '{close}'?)"));
    } else if !tags.is_empty() {
        message.push_str(&format!("; tags: {}", choices(tags)));
    }
    message.push_str(&format!("; {hint}"));
    Err(usage(message))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tags() -> Vec<String> {
        vec!["Needs review".into(), "ui".into()]
    }

    #[test]
    fn tags_take_the_instance_spelling() {
        assert_eq!(
            resolve(&tags(), "needs REVIEW", "").unwrap(),
            "Needs review"
        );
    }

    #[test]
    fn unknown_tags_suggest_the_closest() {
        let error = resolve(&tags(), "Neds review", "pass --no-validate to create it").unwrap_err();
        assert!(
            error.to_string().contains("did you mean 'Needs review'"),
            "{error}"
        );
        assert!(error.to_string().contains("--no-validate"), "{error}");
    }
}
