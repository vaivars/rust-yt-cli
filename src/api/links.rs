use serde::Deserialize;

use crate::api::custom_fields::IssueCustomField;
use crate::api::{Client, Projection};
use crate::error::Result;
use crate::fields::{choices, closest, same, usage};

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkType {
    pub name: String,
    pub source_to_target: Option<String>,
    pub target_to_source: Option<String>,
    #[serde(default)]
    pub directed: bool,
}

impl Projection for LinkType {
    const FIELDS: &'static str = "name,sourceToTarget,targetToSource,directed";
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueLink {
    pub direction: String,
    pub link_type: LinkType,
    #[serde(default)]
    pub issues: Vec<LinkedIssue>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkedIssue {
    pub id_readable: String,
    pub summary: Option<String>,
    #[serde(default)]
    pub custom_fields: Vec<IssueCustomField>,
}

impl Projection for IssueLink {
    const FIELDS: &'static str = "direction,linkType(name,sourceToTarget,targetToSource,directed),issues(idReadable,summary,customFields(name,$type,projectCustomField(field(fieldType(id))),value(name,login,fullName,isResolved,minutes,presentation,text,$type)))";
}

impl IssueLink {
    pub fn inward(&self) -> bool {
        self.direction == "INWARD"
    }

    /// How the link reads from this issue (`depends on`, `is required for`).
    pub fn word(&self) -> &str {
        let word = if self.inward() {
            &self.link_type.target_to_source
        } else {
            &self.link_type.source_to_target
        };
        word.as_deref().unwrap_or(&self.link_type.name)
    }
}

pub async fn types(client: &Client) -> Result<Vec<LinkType>> {
    client.get("issueLinkTypes", &[("$top", "500")]).await
}

pub async fn list(client: &Client, issue: &str) -> Result<Vec<IssueLink>> {
    client.get_all(&format!("issues/{issue}/links")).await
}

/// The instance's spelling of a link's direction words.
pub fn resolve_word(types: &[LinkType], word: &str) -> Result<String> {
    let mut words: Vec<String> = Vec::new();
    for link_type in types {
        for candidate in [&link_type.source_to_target, &link_type.target_to_source]
            .into_iter()
            .flatten()
        {
            if !words.contains(candidate) {
                words.push(candidate.clone());
            }
        }
    }
    if let Some(found) = words.iter().find(|candidate| same(candidate, word)) {
        return Ok(found.clone());
    }
    let mut message = format!("'{word}' is not a link type; use {}", choices(&words));
    if let Some(close) = closest(word, words.iter().map(String::as_str)) {
        message.push_str(&format!(" (did you mean '{close}'?)"));
    }
    Err(usage(message))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn link_types() -> Vec<LinkType> {
        vec![
            LinkType {
                name: "Depend".into(),
                source_to_target: Some("depends on".into()),
                target_to_source: Some("is required for".into()),
                directed: true,
            },
            LinkType {
                name: "Relates".into(),
                source_to_target: Some("relates to".into()),
                target_to_source: Some("relates to".into()),
                directed: false,
            },
        ]
    }

    #[test]
    fn direction_words_take_the_instance_spelling() {
        assert_eq!(
            resolve_word(&link_types(), "Depends On").unwrap(),
            "depends on"
        );
        assert_eq!(
            resolve_word(&link_types(), "is required for").unwrap(),
            "is required for"
        );
        let error = resolve_word(&link_types(), "depend on").unwrap_err();
        assert!(
            error.to_string().contains("did you mean 'depends on'"),
            "{error}"
        );
    }

    #[test]
    fn a_link_reads_in_its_direction() {
        let link = IssueLink {
            direction: "INWARD".into(),
            link_type: link_types().remove(0),
            issues: vec![],
        };
        assert_eq!(link.word(), "is required for");
        assert!(link.inward());
    }
}
