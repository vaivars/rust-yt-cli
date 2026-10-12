pub mod apply;
pub mod cache;
pub mod change;
pub mod plan;
pub mod resolve;
pub mod schema;

use crate::api::Client;
use crate::error::{Result, YtError};
use crate::fields::cache::SchemaCache;
use crate::fields::change::{Change, Op};
use crate::fields::resolve::Resolved;
use crate::fields::schema::ProjectSchema;

/// Case-insensitive comparison for names and values (Unicode, so `Stāvoklis` matches `stāvoklis`).
pub fn same(a: &str, b: &str) -> bool {
    a.to_lowercase() == b.to_lowercase()
}

/// Up to ten values, then how many more.
pub fn choices(values: &[String]) -> String {
    let mut text = values
        .iter()
        .take(10)
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join(", ");
    if values.len() > 10 {
        text.push_str(&format!(" and {} more", values.len() - 10));
    }
    text
}

/// The candidate closest to `value`, when it is close enough to be a likely typo.
pub fn closest<'a>(value: &str, candidates: impl IntoIterator<Item = &'a str>) -> Option<&'a str> {
    let value = value.to_lowercase();
    candidates
        .into_iter()
        .map(|candidate| {
            (
                strsim::jaro_winkler(&value, &candidate.to_lowercase()),
                candidate,
            )
        })
        .filter(|(score, _)| *score > 0.8)
        .max_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, candidate)| candidate)
}

/// `1w 2d 3h 30m`: any of these units, each once, in this order; `None` for anything else.
pub fn period(value: &str) -> Option<String> {
    let units = ['w', 'd', 'h', 'm'];
    let mut next = 0;
    let mut parts = Vec::new();
    for token in value.split_whitespace() {
        let (index, unit) = token.char_indices().last()?;
        let number = &token[..index];
        let position = units.iter().position(|candidate| *candidate == unit)?;
        if position < next || number.is_empty() || !number.chars().all(|c| c.is_ascii_digit()) {
            return None;
        }
        next = position + 1;
        parts.push(token);
    }
    (!parts.is_empty()).then(|| parts.join(" "))
}

pub(crate) fn usage(message: impl Into<String>) -> miette::Report {
    YtError::Usage {
        message: message.into(),
    }
    .into()
}

/// Resolves changes for issues of `project` with the cached schema, refetching it once when a name or value
/// is not in the cached copy (it may predate a change in YouTrack).
pub async fn resolve_for(
    client: &Client,
    cache: &SchemaCache,
    project: &str,
    changes: &[Change],
    validate: bool,
) -> Result<(ProjectSchema, Vec<Resolved>)> {
    let me = if mentions_me(changes) {
        Some(crate::api::users::me(client).await?.login)
    } else {
        None
    };
    if let Some(schema) = cache.load(project)
        && let Ok(resolved) = resolve::resolve(&schema, changes, me.as_deref(), validate)
    {
        return Ok((schema, resolved));
    }
    let schema = cache.refresh(client, project).await?;
    let resolved = resolve::resolve(&schema, changes, me.as_deref(), validate)?;
    Ok((schema, resolved))
}

fn mentions_me(changes: &[Change]) -> bool {
    changes.iter().any(|change| match &change.op {
        Op::Set(values) => values.iter().any(|value| value.eq_ignore_ascii_case("me")),
        Op::Add(value) | Op::Remove(value) => value.eq_ignore_ascii_case("me"),
        Op::Clear => false,
    })
}

#[cfg(test)]
mod tests {
    use crate::api::test_client;
    use crate::fields::cache::SchemaCache;
    use crate::fields::change::{Change, Op};
    use crate::fields::resolve::ResolvedOp;
    use crate::fields::schema::{ProjectSchema, SchemaField};
    use serde_json::json;
    use std::time::Duration;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;

    #[test]
    fn names_match_case_insensitively_in_any_script() {
        assert!(same("Stāvoklis", "STĀVOKLIS"));
        assert!(!same("State", "Stage"));
    }

    #[test]
    fn periods_take_each_unit_once_in_order() {
        assert_eq!(period("1w 2d 3h 30m").as_deref(), Some("1w 2d 3h 30m"));
        assert_eq!(period(" 90m ").as_deref(), Some("90m"));
        assert_eq!(period("2d 1w"), None);
        assert_eq!(period("90"), None);
        assert_eq!(period(""), None);
    }

    #[test]
    fn long_choice_lists_are_cut() {
        let values: Vec<String> = (1..=12).map(|n| n.to_string()).collect();
        assert_eq!(choices(&values), "1, 2, 3, 4, 5, 6, 7, 8, 9, 10 and 2 more");
    }

    #[test]
    fn the_closest_candidate_is_suggested_only_when_close() {
        assert_eq!(closest("Majr", ["Normal", "Major"]), Some("Major"));
        assert_eq!(closest("xyz", ["Normal", "Major"]), None);
    }

    fn priority(values: &[&str]) -> ProjectSchema {
        ProjectSchema {
            project: "WEB".into(),
            fields: vec![SchemaField {
                name: "Priority".into(),
                aliases: vec![],
                type_id: "enum".into(),
                multi: false,
                can_be_empty: false,
                has_default: true,
                values: values.iter().map(|value| value.to_string()).collect(),
            }],
        }
    }

    fn server_fields(values: &[&str]) -> serde_json::Value {
        let values: Vec<_> = values
            .iter()
            .map(|name| json!({"name": name, "archived": false}))
            .collect();
        json!([{"canBeEmpty": false, "defaultValues": [{"name": "Normal"}],
                "field": {"name": "Priority", "fieldType": {"id": "enum[1]", "isMultiValue": false}},
                "bundle": {"values": values}}])
    }

    fn set_priority(value: &str) -> Vec<Change> {
        vec![Change {
            name: "Priority".into(),
            op: Op::Set(vec![value.into()]),
        }]
    }

    #[tokio::test]
    async fn a_fresh_cached_schema_needs_no_request() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(500))
            .expect(0)
            .mount(&server)
            .await;
        let dir = tempfile::tempdir().unwrap();
        let cache = SchemaCache::new(dir.path(), "acme", Duration::from_secs(60));
        cache.store(&priority(&["Normal", "Major"]));
        let (_, resolved) = resolve_for(
            &test_client(&server.uri()),
            &cache,
            "WEB",
            &set_priority("major"),
            true,
        )
        .await
        .unwrap();
        assert_eq!(resolved[0].op, ResolvedOp::Set(vec!["Major".into()]));
    }

    #[tokio::test]
    async fn a_value_missing_from_the_cache_refetches_once() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/admin/projects/WEB/customFields"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(server_fields(&["Normal", "Urgent"])),
            )
            .expect(1)
            .mount(&server)
            .await;
        let dir = tempfile::tempdir().unwrap();
        let cache = SchemaCache::new(dir.path(), "acme", Duration::from_secs(60));
        cache.store(&priority(&["Normal"]));
        let (schema, resolved) = resolve_for(
            &test_client(&server.uri()),
            &cache,
            "WEB",
            &set_priority("Urgent"),
            true,
        )
        .await
        .unwrap();
        assert_eq!(resolved[0].op, ResolvedOp::Set(vec!["Urgent".into()]));
        assert_eq!(cache.load("WEB"), Some(schema));
    }

    #[tokio::test]
    async fn me_is_looked_up_only_when_used() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/users/me"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({"id": "1-1", "login": "jane"})),
            )
            .expect(1)
            .mount(&server)
            .await;
        let dir = tempfile::tempdir().unwrap();
        let cache = SchemaCache::new(dir.path(), "acme", Duration::from_secs(60));
        cache.store(&ProjectSchema {
            project: "WEB".into(),
            fields: vec![SchemaField {
                name: "Assignee".into(),
                aliases: vec![],
                type_id: "user".into(),
                multi: false,
                can_be_empty: true,
                has_default: false,
                values: vec!["jane".into()],
            }],
        });
        let changes = vec![Change {
            name: "Assignee".into(),
            op: Op::Set(vec!["me".into()]),
        }];
        let (_, resolved) = resolve_for(&test_client(&server.uri()), &cache, "WEB", &changes, true)
            .await
            .unwrap();
        assert_eq!(resolved[0].op, ResolvedOp::Set(vec!["jane".into()]));
    }
}
