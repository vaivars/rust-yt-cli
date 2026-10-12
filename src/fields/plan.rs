use chrono::NaiveDate;
use serde_json::{Map, Value, json};

use crate::error::Result;
use crate::fields::resolve::{Resolved, ResolvedOp};
use crate::fields::schema::{Kind, SchemaField};
use crate::fields::usage;

#[derive(Clone, Debug, PartialEq)]
pub enum Operation {
    /// One command applied to every issue in one request.
    Command {
        issues: Vec<String>,
        command: String,
    },
    /// One typed REST update of one issue; `summary` says what it changes.
    Update {
        issue: String,
        body: Value,
        summary: String,
    },
}

impl Operation {
    pub fn describe(&self) -> String {
        match self {
            Self::Command { issues, command } => {
                format!("{}: command \"{command}\"", issues.join(", "))
            }
            Self::Update { issue, summary, .. } => format!("{issue}: {summary}"),
        }
    }
}

/// Single-value sets of bundle-like fields go through commands (one request for many issues, workflows apply);
/// typed values, multi-value sets and clears go through REST, where the value's form is exact.
fn via_command(field: &SchemaField, op: &ResolvedOp) -> bool {
    match op {
        ResolvedOp::Add(_) | ResolvedOp::Remove(_) => true,
        ResolvedOp::Clear => false,
        ResolvedOp::Set(_) if field.multi => false,
        ResolvedOp::Set(_) => matches!(
            field.kind(),
            Kind::Bundle | Kind::State | Kind::User | Kind::Other
        ),
    }
}

fn command_for(field: &SchemaField, op: &ResolvedOp) -> Option<String> {
    match op {
        ResolvedOp::Set(values) => values
            .first()
            .map(|value| format!("{} {value}", field.name)),
        // A bare `NAME VALUE` replaces every value of a multi-value field.
        ResolvedOp::Add(value) => Some(format!("add {} {value}", field.name)),
        ResolvedOp::Remove(value) => Some(format!("remove {} {value}", field.name)),
        ResolvedOp::Clear => None,
    }
}

/// Operations that change existing issues: one REST update per issue, then one command per command change.
pub fn edit_operations(
    issues: &[String],
    resolved: &[Resolved],
    summary: Option<&str>,
    description: Option<&str>,
) -> Result<Vec<Operation>> {
    let mut body = Map::new();
    let mut notes = Vec::new();
    if let Some(summary) = summary {
        body.insert("summary".into(), json!(summary));
        notes.push("set the summary".to_string());
    }
    if let Some(description) = description {
        body.insert("description".into(), json!(description));
        notes.push("set the description".to_string());
    }
    let mut fields = Vec::new();
    for change in resolved
        .iter()
        .filter(|change| !via_command(&change.field, &change.op))
    {
        match &change.op {
            ResolvedOp::Clear => {
                fields.push(custom_field(&change.field, None)?);
                notes.push(format!("clear {}", change.field.name));
            }
            ResolvedOp::Set(values) => {
                fields.push(custom_field(&change.field, Some(values))?);
                notes.push(format!(
                    "set {} to {}",
                    change.field.name,
                    describe_values(&change.field, values)
                ));
            }
            ResolvedOp::Add(_) | ResolvedOp::Remove(_) => {}
        }
    }
    if !fields.is_empty() {
        body.insert("customFields".into(), Value::Array(fields));
    }
    let mut operations = Vec::new();
    if !body.is_empty() {
        for issue in issues {
            operations.push(Operation::Update {
                issue: issue.clone(),
                body: Value::Object(body.clone()),
                summary: notes.join(", "),
            });
        }
    }
    for change in resolved
        .iter()
        .filter(|change| via_command(&change.field, &change.op))
    {
        if let Some(command) = command_for(&change.field, &change.op) {
            operations.push(Operation::Command {
                issues: issues.to_vec(),
                command,
            });
        }
    }
    Ok(operations)
}

/// The REST body of a new issue, and the commands to run once it exists (state fields go through workflows).
pub fn create_request(
    project: &str,
    summary: &str,
    description: Option<&str>,
    resolved: &[Resolved],
) -> Result<(Value, Vec<String>)> {
    let mut grouped: Vec<(&SchemaField, Vec<String>)> = Vec::new();
    for change in resolved {
        let values = match &change.op {
            ResolvedOp::Set(values) => values.clone(),
            ResolvedOp::Add(value) => vec![value.clone()],
            ResolvedOp::Remove(_) | ResolvedOp::Clear => {
                return Err(usage(format!(
                    "{}: a new issue can only be given values (NAME=VALUE or NAME+=VALUE)",
                    change.field.name
                )));
            }
        };
        match grouped
            .iter_mut()
            .find(|(field, _)| field.name == change.field.name)
        {
            Some((_, existing)) => existing.extend(values),
            None => grouped.push((&change.field, values)),
        }
    }
    let mut fields = Vec::new();
    let mut commands = Vec::new();
    for (field, values) in grouped {
        if matches!(field.kind(), Kind::State | Kind::Other) {
            let verb = if field.multi { "add " } else { "" };
            commands.extend(
                values
                    .iter()
                    .map(|value| format!("{verb}{} {value}", field.name)),
            );
        } else {
            fields.push(custom_field(field, Some(&values))?);
        }
    }
    let mut body = json!({
        "project": {"shortName": project},
        "summary": summary,
    });
    if let Some(description) = description {
        body["description"] = json!(description);
    }
    if !fields.is_empty() {
        body["customFields"] = Value::Array(fields);
    }
    Ok((body, commands))
}

/// One entry of `customFields` in a REST write; `None` clears the field.
pub fn custom_field(field: &SchemaField, values: Option<&[String]>) -> Result<Value> {
    let Some(write_type) = field.write_type() else {
        return Err(usage(format!(
            "{}: yt cannot write this field directly; use `yt issue command`",
            field.name
        )));
    };
    let value = match values {
        None if field.multi => json!([]),
        None => Value::Null,
        Some(values) if field.multi => Value::Array(
            values
                .iter()
                .map(|value| one_value(field, value))
                .collect::<Result<_>>()?,
        ),
        Some(values) => one_value(
            field,
            values.first().map(String::as_str).unwrap_or_default(),
        )?,
    };
    Ok(json!({"name": field.name, "$type": write_type, "value": value}))
}

fn one_value(field: &SchemaField, value: &str) -> Result<Value> {
    let invalid = || {
        usage(format!(
            "{}: '{value}' cannot be sent as a {}",
            field.name, field.type_id
        ))
    };
    Ok(match field.kind() {
        Kind::Bundle | Kind::State => json!({"name": value}),
        Kind::User => json!({"login": value}),
        Kind::Date => {
            let date = NaiveDate::parse_from_str(value, "%Y-%m-%d").map_err(|_| invalid())?;
            json!(
                date.and_hms_opt(0, 0, 0)
                    .expect("midnight exists")
                    .and_utc()
                    .timestamp_millis()
            )
        }
        Kind::DateTime => json!(value.parse::<i64>().map_err(|_| invalid())?),
        Kind::Period => json!({"presentation": value}),
        Kind::Integer => json!(value.parse::<i64>().map_err(|_| invalid())?),
        Kind::Float => json!(value.parse::<f64>().map_err(|_| invalid())?),
        Kind::String => json!(value),
        Kind::Text => json!({"text": value}),
        Kind::Other => return Err(invalid()),
    })
}

/// Values for messages: a list for multi-value fields, date-times in local time.
pub fn describe_values(field: &SchemaField, values: &[String]) -> String {
    let shown: Vec<String> = values
        .iter()
        .map(|value| match (field.kind(), value.parse::<i64>()) {
            (Kind::DateTime, Ok(millis)) => crate::time::local(millis),
            _ => value.clone(),
        })
        .collect();
    if field.multi {
        format!("[{}]", shown.join(", "))
    } else {
        shown.join(", ")
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn field(name: &str, type_id: &str, multi: bool) -> SchemaField {
        SchemaField {
            name: name.into(),
            aliases: vec![],
            type_id: type_id.into(),
            multi,
            can_be_empty: true,
            has_default: false,
            values: vec![],
        }
    }

    fn resolved(name: &str, type_id: &str, multi: bool, op: ResolvedOp) -> Resolved {
        Resolved {
            field: field(name, type_id, multi),
            op,
        }
    }

    fn issues() -> Vec<String> {
        vec!["WEB-1".into(), "WEB-2".into()]
    }

    #[test]
    fn rest_updates_come_first_then_one_command_per_change() {
        let changes = [
            resolved(
                "Priority",
                "enum",
                false,
                ResolvedOp::Set(vec!["Major".into()]),
            ),
            resolved(
                "Fix versions",
                "version",
                true,
                ResolvedOp::Set(vec!["1.0".into(), "1.1".into()]),
            ),
            resolved("Assignee", "user", false, ResolvedOp::Clear),
            resolved("Labels", "enum", true, ResolvedOp::Add("ui".into())),
            resolved("Labels", "enum", true, ResolvedOp::Remove("old".into())),
            resolved(
                "State",
                "state",
                false,
                ResolvedOp::Set(vec!["In Progress".into()]),
            ),
        ];
        let operations = edit_operations(&issues(), &changes, None, None).unwrap();
        let body = json!({"customFields": [
            {"name": "Fix versions", "$type": "MultiVersionIssueCustomField", "value": [{"name": "1.0"}, {"name": "1.1"}]},
            {"name": "Assignee", "$type": "SingleUserIssueCustomField", "value": null}
        ]});
        let command = |text: &str| Operation::Command {
            issues: issues(),
            command: text.into(),
        };
        assert_eq!(
            operations,
            vec![
                Operation::Update {
                    issue: "WEB-1".into(),
                    body: body.clone(),
                    summary: "set Fix versions to [1.0, 1.1], clear Assignee".into()
                },
                Operation::Update {
                    issue: "WEB-2".into(),
                    body,
                    summary: "set Fix versions to [1.0, 1.1], clear Assignee".into()
                },
                command("Priority Major"),
                command("add Labels ui"),
                command("remove Labels old"),
                command("State In Progress"),
            ]
        );
    }

    #[test]
    fn typed_values_go_through_rest() {
        let changes = [
            resolved(
                "Due Date",
                "date",
                false,
                ResolvedOp::Set(vec!["2026-10-10".into()]),
            ),
            resolved(
                "Deadline",
                "date and time",
                false,
                ResolvedOp::Set(vec!["1791633600000".into()]),
            ),
            resolved(
                "Estimation",
                "period",
                false,
                ResolvedOp::Set(vec!["1w 2d".into()]),
            ),
            resolved(
                "Points",
                "integer",
                false,
                ResolvedOp::Set(vec!["5".into()]),
            ),
            resolved(
                "Notes",
                "text",
                false,
                ResolvedOp::Set(vec!["two words".into()]),
            ),
        ];
        let operations =
            edit_operations(&["WEB-1".into()], &changes, Some("New"), Some("Text")).unwrap();
        let Operation::Update { body, .. } = &operations[0] else {
            panic!("expected an update: {operations:?}")
        };
        assert_eq!(
            body,
            &json!({
                "summary": "New",
                "description": "Text",
                "customFields": [
                    {"name": "Due Date", "$type": "DateIssueCustomField", "value": 1_791_590_400_000_i64},
                    {"name": "Deadline", "$type": "DateIssueCustomField", "value": 1_791_633_600_000_i64},
                    {"name": "Estimation", "$type": "PeriodIssueCustomField", "value": {"presentation": "1w 2d"}},
                    {"name": "Points", "$type": "SimpleIssueCustomField", "value": 5},
                    {"name": "Notes", "$type": "TextIssueCustomField", "value": {"text": "two words"}}
                ]
            })
        );
        assert_eq!(operations.len(), 1);
    }

    #[test]
    fn unknown_fields_go_out_as_commands() {
        let changes = [Resolved {
            field: SchemaField::unknown("Custom"),
            op: ResolvedOp::Set(vec!["Value with spaces".into()]),
        }];
        let operations = edit_operations(&["WEB-1".into()], &changes, None, None).unwrap();
        assert_eq!(
            operations,
            vec![Operation::Command {
                issues: vec!["WEB-1".into()],
                command: "Custom Value with spaces".into()
            }]
        );
        let clear = [Resolved {
            field: SchemaField::unknown("Custom"),
            op: ResolvedOp::Clear,
        }];
        assert!(edit_operations(&["WEB-1".into()], &clear, None, None).is_err());
    }

    #[test]
    fn a_new_issue_gets_typed_fields_and_state_commands() {
        let changes = [
            resolved(
                "Priority",
                "enum",
                false,
                ResolvedOp::Set(vec!["Major".into()]),
            ),
            resolved(
                "Fix versions",
                "version",
                true,
                ResolvedOp::Add("1.0".into()),
            ),
            resolved(
                "Fix versions",
                "version",
                true,
                ResolvedOp::Add("1.1".into()),
            ),
            resolved(
                "State",
                "state",
                false,
                ResolvedOp::Set(vec!["In Progress".into()]),
            ),
        ];
        let (body, commands) =
            create_request("WEB", "Login fails", Some("Steps"), &changes).unwrap();
        assert_eq!(
            body,
            json!({
                "project": {"shortName": "WEB"},
                "summary": "Login fails",
                "description": "Steps",
                "customFields": [
                    {"name": "Priority", "$type": "SingleEnumIssueCustomField", "value": {"name": "Major"}},
                    {"name": "Fix versions", "$type": "MultiVersionIssueCustomField", "value": [{"name": "1.0"}, {"name": "1.1"}]}
                ]
            })
        );
        assert_eq!(commands, ["State In Progress"]);
        let remove = [resolved(
            "Fix versions",
            "version",
            true,
            ResolvedOp::Remove("1.0".into()),
        )];
        assert!(create_request("WEB", "x", None, &remove).is_err());
    }

    #[test]
    fn a_new_issue_gets_every_value_of_a_multi_value_command_field() {
        let changes = [
            resolved("Custom", "", true, ResolvedOp::Add("a".into())),
            resolved("Custom", "", true, ResolvedOp::Add("b".into())),
        ];
        let (_, commands) = create_request("WEB", "x", None, &changes).unwrap();
        assert_eq!(commands, ["add Custom a", "add Custom b"]);
    }

    #[test]
    fn operations_describe_themselves() {
        let command = Operation::Command {
            issues: issues(),
            command: "Priority Major".into(),
        };
        assert_eq!(
            command.describe(),
            "WEB-1, WEB-2: command \"Priority Major\""
        );
    }
}
