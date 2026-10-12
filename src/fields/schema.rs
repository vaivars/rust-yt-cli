use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::api::{Client, Projection};
use crate::error::Result;
use crate::fields::{same, usage};

/// A project's custom fields as yt needs them for validation and typed writes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProjectSchema {
    pub project: String,
    pub fields: Vec<SchemaField>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SchemaField {
    pub name: String,
    pub aliases: Vec<String>,
    /// `fieldType.id` without its multiplicity suffix: `enum`, `state`, `user`, `date and time`, …
    pub type_id: String,
    pub multi: bool,
    pub can_be_empty: bool,
    pub has_default: bool,
    /// Allowed values: non-archived bundle values, or the logins a user field accepts.
    pub values: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Bundle,
    State,
    User,
    Date,
    DateTime,
    Period,
    Integer,
    Float,
    String,
    Text,
    Other,
}

impl SchemaField {
    /// A field the schema does not know, written only through commands (`--no-validate`).
    pub fn unknown(name: &str) -> Self {
        Self {
            name: name.to_string(),
            aliases: Vec::new(),
            type_id: String::new(),
            multi: false,
            can_be_empty: true,
            has_default: true,
            values: Vec::new(),
        }
    }

    pub fn kind(&self) -> Kind {
        match self.type_id.as_str() {
            "enum" | "version" | "build" | "ownedField" | "group" => Kind::Bundle,
            "state" => Kind::State,
            "user" => Kind::User,
            "date" => Kind::Date,
            "date and time" => Kind::DateTime,
            "period" => Kind::Period,
            "integer" => Kind::Integer,
            "float" => Kind::Float,
            "string" => Kind::String,
            "text" => Kind::Text,
            _ => Kind::Other,
        }
    }

    /// The `$type` of this field's value in a typed REST write.
    pub fn write_type(&self) -> Option<&'static str> {
        let (single, multi) = match self.type_id.as_str() {
            "enum" => (
                "SingleEnumIssueCustomField",
                Some("MultiEnumIssueCustomField"),
            ),
            "state" => ("StateIssueCustomField", None),
            "user" => (
                "SingleUserIssueCustomField",
                Some("MultiUserIssueCustomField"),
            ),
            "version" => (
                "SingleVersionIssueCustomField",
                Some("MultiVersionIssueCustomField"),
            ),
            "build" => (
                "SingleBuildIssueCustomField",
                Some("MultiBuildIssueCustomField"),
            ),
            "ownedField" => (
                "SingleOwnedIssueCustomField",
                Some("MultiOwnedIssueCustomField"),
            ),
            "group" => (
                "SingleGroupIssueCustomField",
                Some("MultiGroupIssueCustomField"),
            ),
            "date" | "date and time" => ("DateIssueCustomField", None),
            "period" => ("PeriodIssueCustomField", None),
            "integer" | "float" | "string" => ("SimpleIssueCustomField", None),
            "text" => ("TextIssueCustomField", None),
            _ => return None,
        };
        if self.multi { multi } else { Some(single) }
    }

    pub fn matches(&self, name: &str) -> bool {
        same(&self.name, name) || self.aliases.iter().any(|alias| same(alias, name))
    }
}

impl ProjectSchema {
    pub fn field(&self, name: &str) -> Option<&SchemaField> {
        self.fields.iter().find(|field| field.matches(name))
    }
}

const PROJECT_FIELD: &str = "canBeEmpty,defaultValues(name,login),field(name,aliases,fieldType(id,isMultiValue)),bundle(values(name,login,archived),aggregatedUsers(login))";

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawProjectField {
    #[serde(default)]
    can_be_empty: bool,
    #[serde(default)]
    default_values: Value,
    field: RawField,
    bundle: Option<RawBundle>,
}

impl Projection for RawProjectField {
    const FIELDS: &'static str = PROJECT_FIELD;
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawField {
    name: String,
    #[serde(default)]
    aliases: Value,
    field_type: RawFieldType,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawFieldType {
    id: String,
    #[serde(default)]
    is_multi_value: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawBundle {
    #[serde(default)]
    values: Vec<RawValue>,
    #[serde(default)]
    aggregated_users: Vec<RawUser>,
}

#[derive(Deserialize)]
struct RawValue {
    name: Option<String>,
    login: Option<String>,
    #[serde(default)]
    archived: bool,
}

#[derive(Deserialize)]
struct RawUser {
    login: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct IssueFields {
    #[serde(default)]
    custom_fields: Vec<IssueField>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct IssueField {
    project_custom_field: Option<RawProjectField>,
}

impl Projection for IssueFields {
    const FIELDS: &'static str = "customFields(projectCustomField(canBeEmpty,defaultValues(name,login),field(name,aliases,fieldType(id,isMultiValue)),bundle(values(name,login,archived),aggregatedUsers(login))))";
}

impl From<RawProjectField> for SchemaField {
    fn from(raw: RawProjectField) -> Self {
        let id = raw.field.field_type.id;
        let type_id = id
            .split_once('[')
            .map_or(id.as_str(), |(base, _)| base)
            .to_string();
        let values = match raw.bundle {
            Some(bundle) if !bundle.aggregated_users.is_empty() => bundle
                .aggregated_users
                .into_iter()
                .map(|user| user.login)
                .collect(),
            Some(bundle) => bundle
                .values
                .into_iter()
                .filter(|value| !value.archived)
                .filter_map(|value| {
                    if type_id == "user" {
                        value.login.or(value.name)
                    } else {
                        value.name.or(value.login)
                    }
                })
                .collect(),
            None => Vec::new(),
        };
        let aliases = match raw.field.aliases {
            Value::String(text) => text
                .split(',')
                .map(str::trim)
                .filter(|alias| !alias.is_empty())
                .map(str::to_string)
                .collect(),
            Value::Array(items) => items
                .into_iter()
                .filter_map(|item| item.as_str().map(str::to_string))
                .collect(),
            _ => Vec::new(),
        };
        let has_default = match raw.default_values {
            Value::Array(items) => !items.is_empty(),
            Value::Null => false,
            _ => true,
        };
        Self {
            name: raw.field.name,
            aliases,
            type_id,
            multi: raw.field.field_type.is_multi_value,
            can_be_empty: raw.can_be_empty,
            has_default,
            values,
        }
    }
}

/// Reads a project's fields: from the project, or from one of its issues when the project lists none
/// (some instances hide project settings from non-admins).
pub async fn fetch(client: &Client, project: &str) -> Result<ProjectSchema> {
    check_short_name(project)?;
    let fields: Vec<RawProjectField> = client
        .get(
            &format!("admin/projects/{project}/customFields"),
            &[("$top", "500")],
        )
        .await?;
    let fields: Vec<SchemaField> = if fields.is_empty() {
        let query = format!("project: {{{project}}}");
        let issues: Vec<IssueFields> = client
            .get("issues", &[("query", query.as_str()), ("$top", "1")])
            .await?;
        issues
            .into_iter()
            .next()
            .map(|issue| {
                issue
                    .custom_fields
                    .into_iter()
                    .filter_map(|field| field.project_custom_field)
                    .map(SchemaField::from)
                    .collect()
            })
            .unwrap_or_default()
    } else {
        fields.into_iter().map(SchemaField::from).collect()
    };
    Ok(ProjectSchema {
        project: project.to_string(),
        fields,
    })
}

/// Project short names go into URL paths; anything else is rejected.
pub fn check_short_name(project: &str) -> Result<()> {
    if !project.is_empty() && project.chars().all(|c| c.is_alphanumeric() || c == '_') {
        Ok(())
    } else {
        Err(usage(format!("'{project}' is not a project short name")))
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::api::test_client;

    fn project_fields() -> serde_json::Value {
        json!([
            {"canBeEmpty": false, "defaultValues": [{"name": "Normal"}],
             "field": {"name": "Priority", "aliases": "prio, p", "fieldType": {"id": "enum[1]", "isMultiValue": false}},
             "bundle": {"values": [{"name": "Normal", "archived": false}, {"name": "Major", "archived": false}, {"name": "Old", "archived": true}]}},
            {"canBeEmpty": true, "defaultValues": [],
             "field": {"name": "Assignee", "aliases": "", "fieldType": {"id": "user[1]", "isMultiValue": false}},
             "bundle": {"aggregatedUsers": [{"login": "jane"}, {"login": "bob"}]}},
            {"canBeEmpty": true, "defaultValues": [],
             "field": {"name": "Fix versions", "fieldType": {"id": "version[*]", "isMultiValue": true}},
             "bundle": {"values": [{"name": "1.0", "archived": false}]}},
            {"canBeEmpty": true,
             "field": {"name": "Due Date", "fieldType": {"id": "date", "isMultiValue": false}}}
        ])
    }

    fn expected() -> Vec<SchemaField> {
        vec![
            SchemaField {
                name: "Priority".into(),
                aliases: vec!["prio".into(), "p".into()],
                type_id: "enum".into(),
                multi: false,
                can_be_empty: false,
                has_default: true,
                values: vec!["Normal".into(), "Major".into()],
            },
            SchemaField {
                name: "Assignee".into(),
                aliases: vec![],
                type_id: "user".into(),
                multi: false,
                can_be_empty: true,
                has_default: false,
                values: vec!["jane".into(), "bob".into()],
            },
            SchemaField {
                name: "Fix versions".into(),
                aliases: vec![],
                type_id: "version".into(),
                multi: true,
                can_be_empty: true,
                has_default: false,
                values: vec!["1.0".into()],
            },
            SchemaField {
                name: "Due Date".into(),
                aliases: vec![],
                type_id: "date".into(),
                multi: false,
                can_be_empty: true,
                has_default: false,
                values: vec![],
            },
        ]
    }

    #[tokio::test]
    async fn the_schema_comes_from_the_project() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/admin/projects/WEB/customFields"))
            .respond_with(ResponseTemplate::new(200).set_body_json(project_fields()))
            .expect(1)
            .mount(&server)
            .await;
        let schema = fetch(&test_client(&server.uri()), "WEB").await.unwrap();
        assert_eq!(schema.project, "WEB");
        assert_eq!(schema.fields, expected());
    }

    #[tokio::test]
    async fn an_empty_project_list_falls_back_to_an_issue() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/admin/projects/WEB/customFields"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
            .mount(&server)
            .await;
        let issue_fields: Vec<serde_json::Value> = project_fields()
            .as_array()
            .unwrap()
            .iter()
            .map(|field| json!({"projectCustomField": field}))
            .collect();
        Mock::given(method("GET"))
            .and(path("/api/issues"))
            .and(query_param("query", "project: {WEB}"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!([{"customFields": issue_fields}])),
            )
            .expect(1)
            .mount(&server)
            .await;
        let schema = fetch(&test_client(&server.uri()), "WEB").await.unwrap();
        assert_eq!(schema.fields, expected());
    }

    #[test]
    fn kinds_and_write_types_follow_the_field_type() {
        let mut field = expected().remove(2);
        assert_eq!(field.kind(), Kind::Bundle);
        assert_eq!(field.write_type(), Some("MultiVersionIssueCustomField"));
        field.multi = false;
        assert_eq!(field.write_type(), Some("SingleVersionIssueCustomField"));
        for (type_id, kind, write_type) in [
            ("state", Kind::State, Some("StateIssueCustomField")),
            (
                "date and time",
                Kind::DateTime,
                Some("DateIssueCustomField"),
            ),
            ("period", Kind::Period, Some("PeriodIssueCustomField")),
            ("integer", Kind::Integer, Some("SimpleIssueCustomField")),
            ("text", Kind::Text, Some("TextIssueCustomField")),
            ("", Kind::Other, None),
        ] {
            field.type_id = type_id.into();
            assert_eq!(
                (field.kind(), field.write_type()),
                (kind, write_type),
                "{type_id}"
            );
        }
    }

    #[test]
    fn fields_are_found_by_name_or_alias() {
        let schema = ProjectSchema {
            project: "WEB".into(),
            fields: expected(),
        };
        assert_eq!(
            schema.field("PRIO").map(|field| field.name.as_str()),
            Some("Priority")
        );
        assert_eq!(
            schema
                .field("fix VERSIONS")
                .map(|field| field.name.as_str()),
            Some("Fix versions")
        );
        assert!(schema.field("Type").is_none());
    }

    #[test]
    fn only_short_names_are_accepted_in_paths() {
        assert!(check_short_name("WEB_2").is_ok());
        assert!(check_short_name("A/B").is_err());
        assert!(check_short_name("").is_err());
    }
}
