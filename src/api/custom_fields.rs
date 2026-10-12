use serde::Deserialize;
use serde_json::Value;

/// One selector for every custom field type; YouTrack drops the subfields a type does not have.
pub const FIELDS: &str = "name,$type,projectCustomField(field(fieldType(id))),value(name,login,fullName,isResolved,minutes,presentation,text,$type)";

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueCustomField {
    pub name: String,
    #[serde(rename = "$type")]
    pub kind: String,
    #[serde(default)]
    pub value: Value,
    #[serde(default)]
    pub project_custom_field: Option<ProjectFieldRef>,
}

/// Just enough of the field definition to tell a date from a date and time.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectFieldRef {
    pub field: Option<FieldRef>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FieldRef {
    pub field_type: Option<FieldTypeRef>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct FieldTypeRef {
    pub id: String,
}

#[derive(Clone, Debug, PartialEq)]
pub enum FieldValue {
    Empty,
    Bundle {
        name: String,
        is_resolved: Option<bool>,
    },
    Bundles(Vec<String>),
    User {
        login: String,
        full_name: Option<String>,
    },
    Users(Vec<String>),
    Period {
        minutes: i64,
        presentation: String,
    },
    Date(i64),
    Text(String),
    Scalar(Value),
    Unknown(Value),
}

impl IssueCustomField {
    pub fn is_date_time(&self) -> bool {
        self.project_custom_field
            .as_ref()
            .and_then(|definition| definition.field.as_ref())
            .and_then(|field| field.field_type.as_ref())
            .is_some_and(|field_type| field_type.id == "date and time")
    }

    pub fn typed(&self) -> FieldValue {
        let value = &self.value;
        if value.is_null() {
            return FieldValue::Empty;
        }
        let unknown = || FieldValue::Unknown(value.clone());
        let text = |key: &str| value.get(key).and_then(Value::as_str).map(str::to_string);
        match self.kind.as_str() {
            "SingleEnumIssueCustomField"
            | "StateIssueCustomField"
            | "StateMachineIssueCustomField"
            | "SingleVersionIssueCustomField"
            | "SingleBuildIssueCustomField"
            | "SingleOwnedIssueCustomField"
            | "SingleGroupIssueCustomField" => match text("name") {
                Some(name) => FieldValue::Bundle {
                    name,
                    is_resolved: value.get("isResolved").and_then(Value::as_bool),
                },
                None => unknown(),
            },
            "MultiEnumIssueCustomField"
            | "MultiVersionIssueCustomField"
            | "MultiBuildIssueCustomField"
            | "MultiOwnedIssueCustomField"
            | "MultiGroupIssueCustomField" => list(value, "name")
                .map_or_else(unknown, |names| non_empty(names, FieldValue::Bundles)),
            "SingleUserIssueCustomField" => match text("login") {
                Some(login) => FieldValue::User {
                    login,
                    full_name: text("fullName"),
                },
                None => unknown(),
            },
            "MultiUserIssueCustomField" => list(value, "login")
                .map_or_else(unknown, |logins| non_empty(logins, FieldValue::Users)),
            "PeriodIssueCustomField" => match (
                value.get("minutes").and_then(Value::as_i64),
                text("presentation"),
            ) {
                (Some(minutes), Some(presentation)) => FieldValue::Period {
                    minutes,
                    presentation,
                },
                _ => unknown(),
            },
            "DateIssueCustomField" => value.as_i64().map_or_else(unknown, FieldValue::Date),
            "TextIssueCustomField" => text("text").map_or_else(unknown, FieldValue::Text),
            "SimpleIssueCustomField" => FieldValue::Scalar(value.clone()),
            _ => unknown(),
        }
    }

    pub fn display(&self) -> String {
        match self.typed() {
            FieldValue::Empty => String::new(),
            FieldValue::Bundle { name, .. } => name,
            FieldValue::Bundles(names) | FieldValue::Users(names) => names.join(", "),
            FieldValue::User { login, .. } => login,
            FieldValue::Period { presentation, .. } => presentation,
            FieldValue::Date(millis) if self.is_date_time() => crate::time::local(millis),
            FieldValue::Date(millis) => crate::time::date(millis),
            FieldValue::Text(text) => text,
            FieldValue::Scalar(Value::String(text)) => text,
            FieldValue::Scalar(other) | FieldValue::Unknown(other) => other.to_string(),
        }
    }

    pub fn json(&self) -> Value {
        match self.typed() {
            FieldValue::Empty => Value::Null,
            FieldValue::Bundle { name, .. } => Value::String(name),
            FieldValue::Bundles(names) | FieldValue::Users(names) => Value::from(names),
            FieldValue::User { login, .. } => Value::String(login),
            FieldValue::Period { presentation, .. } => Value::String(presentation),
            FieldValue::Date(millis) if self.is_date_time() => {
                Value::String(crate::time::utc(millis))
            }
            FieldValue::Date(millis) => Value::String(crate::time::date(millis)),
            FieldValue::Text(text) => Value::String(text),
            FieldValue::Scalar(value) | FieldValue::Unknown(value) => value,
        }
    }
}

fn list(value: &Value, key: &str) -> Option<Vec<String>> {
    value
        .as_array()?
        .iter()
        .map(|item| item.get(key).and_then(Value::as_str).map(str::to_string))
        .collect()
}

fn non_empty(items: Vec<String>, wrap: fn(Vec<String>) -> FieldValue) -> FieldValue {
    if items.is_empty() {
        FieldValue::Empty
    } else {
        wrap(items)
    }
}

/// The issue's state: the value of the first field whose values know whether they resolve the issue.
pub fn state_of(fields: &[IssueCustomField]) -> Option<String> {
    fields.iter().find_map(|field| match field.typed() {
        FieldValue::Bundle {
            name,
            is_resolved: Some(_),
        } => Some(name),
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn field(kind: &str, value: Value) -> IssueCustomField {
        IssueCustomField {
            name: "F".into(),
            kind: kind.into(),
            value,
            project_custom_field: None,
        }
    }

    #[test]
    fn state_values_keep_is_resolved() {
        let state = field(
            "StateMachineIssueCustomField",
            json!({"name": "In Progress", "isResolved": false, "$type": "StateBundleElement"}),
        );
        assert_eq!(
            state.typed(),
            FieldValue::Bundle {
                name: "In Progress".into(),
                is_resolved: Some(false)
            }
        );
        assert_eq!(state.display(), "In Progress");
        assert_eq!(state.json(), json!("In Progress"));
    }

    #[test]
    fn multi_values_and_empty_ones() {
        let versions = field(
            "MultiVersionIssueCustomField",
            json!([{"name": "1.0"}, {"name": "1.1"}]),
        );
        assert_eq!(versions.display(), "1.0, 1.1");
        assert_eq!(versions.json(), json!(["1.0", "1.1"]));
        assert_eq!(
            field("MultiVersionIssueCustomField", json!([])).typed(),
            FieldValue::Empty
        );
        assert_eq!(
            field("SingleEnumIssueCustomField", Value::Null).display(),
            ""
        );
    }

    #[test]
    fn users_periods_dates_and_text() {
        assert_eq!(
            field(
                "SingleUserIssueCustomField",
                json!({"login": "jane", "fullName": "Jane Doe"})
            )
            .display(),
            "jane"
        );
        assert_eq!(
            field(
                "MultiUserIssueCustomField",
                json!([{"login": "a"}, {"login": "b"}])
            )
            .display(),
            "a, b"
        );
        assert_eq!(
            field(
                "PeriodIssueCustomField",
                json!({"minutes": 150, "presentation": "2h 30m"})
            )
            .display(),
            "2h 30m"
        );
        assert_eq!(
            field("DateIssueCustomField", json!(1_791_590_400_000_i64)).display(),
            "2026-10-10"
        );
        assert_eq!(
            field("TextIssueCustomField", json!({"text": "notes"})).display(),
            "notes"
        );
        assert_eq!(field("SimpleIssueCustomField", json!(3)).display(), "3");
        assert_eq!(
            field("SimpleIssueCustomField", json!("a@b.c")).display(),
            "a@b.c"
        );
    }

    #[test]
    fn unknown_types_keep_their_raw_value() {
        let odd = field("FutureIssueCustomField", json!({"weird": true}));
        assert_eq!(odd.typed(), FieldValue::Unknown(json!({"weird": true})));
        assert_eq!(odd.json(), json!({"weird": true}));
    }

    #[test]
    fn mixed_field_lists_always_decode() {
        let fields: Vec<IssueCustomField> = serde_json::from_value(json!([
            {"name": "Priority", "$type": "SingleEnumIssueCustomField", "value": {"name": "Normal", "$type": "EnumBundleElement"}},
            {"name": "Assignee", "$type": "SingleUserIssueCustomField", "value": null},
            {"name": "Estimation", "$type": "PeriodIssueCustomField", "value": {"minutes": 60, "presentation": "1h"}},
            {"name": "Due", "$type": "DateIssueCustomField"}
        ]))
        .unwrap();
        assert_eq!(fields.len(), 4);
        assert_eq!(fields[3].typed(), FieldValue::Empty);
    }

    #[test]
    fn date_and_time_fields_keep_their_time() {
        let mut date = field("DateIssueCustomField", json!(1_791_633_600_000_i64));
        assert_eq!(date.json(), json!("2026-10-10"));
        date.project_custom_field =
            serde_json::from_value(json!({"field": {"fieldType": {"id": "date and time"}}}))
                .unwrap();
        assert!(date.is_date_time());
        assert_eq!(date.json(), json!("2026-10-10T12:00:00Z"));
        assert_eq!(date.display(), crate::time::local(1_791_633_600_000));
    }
}
