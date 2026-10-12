use chrono::{DateTime, Local, NaiveDate, NaiveDateTime, TimeZone as _};

use crate::error::Result;
use crate::fields::change::{Change, Op};
use crate::fields::schema::{Kind, ProjectSchema, SchemaField};
use crate::fields::{choices, closest, period, same, usage};

#[derive(Clone, Debug, PartialEq)]
pub enum ResolvedOp {
    Set(Vec<String>),
    Add(String),
    Remove(String),
    Clear,
}

/// A change matched against the schema, with values in the form YouTrack expects.
#[derive(Clone, Debug, PartialEq)]
pub struct Resolved {
    pub field: SchemaField,
    pub op: ResolvedOp,
}

/// Matches `changes` against `schema`: names and aliases, allowed values, value formats and multiplicity.
/// Without `validate`, names and values are taken as given; fields the schema lacks go out as commands.
pub fn resolve(
    schema: &ProjectSchema,
    changes: &[Change],
    me: Option<&str>,
    validate: bool,
) -> Result<Vec<Resolved>> {
    let mut resolved: Vec<Resolved> = Vec::new();
    for change in changes {
        let field = match schema.field(&change.name) {
            Some(field) => field.clone(),
            None if !validate => SchemaField::unknown(&change.name),
            None => return Err(unknown_field(schema, &change.name)),
        };
        let both_relative = |other: &Resolved| {
            matches!(
                (&other.op, &change.op),
                (
                    ResolvedOp::Add(_) | ResolvedOp::Remove(_),
                    Op::Add(_) | Op::Remove(_)
                )
            )
        };
        if resolved
            .iter()
            .any(|other| other.field.name == field.name && !both_relative(other))
        {
            return Err(usage(format!(
                "-f {}: {} is changed twice",
                change.name, field.name
            )));
        }
        let op = match &change.op {
            Op::Set(values) => {
                if !field.multi && values.len() > 1 {
                    return Err(usage(format!(
                        "{} takes one value; it was given {}",
                        field.name,
                        values.len()
                    )));
                }
                ResolvedOp::Set(
                    values
                        .iter()
                        .map(|value| canonical(&field, value, me, validate))
                        .collect::<Result<_>>()?,
                )
            }
            Op::Add(_) | Op::Remove(_) if validate && !field.multi => {
                return Err(usage(format!(
                    "{} holds one value; set it with -f '{}=…'",
                    field.name, field.name
                )));
            }
            Op::Add(value) => ResolvedOp::Add(canonical(&field, value, me, validate)?),
            Op::Remove(value) => ResolvedOp::Remove(canonical(&field, value, me, validate)?),
            Op::Clear => {
                if validate && !field.can_be_empty {
                    return Err(usage(format!("{} cannot be empty", field.name)));
                }
                ResolvedOp::Clear
            }
        };
        resolved.push(Resolved { field, op });
    }
    Ok(resolved)
}

/// Fields a new issue must be given: not allowed to be empty, without a default, and not in `resolved`.
pub fn missing_required<'a>(
    schema: &'a ProjectSchema,
    resolved: &[Resolved],
) -> Vec<&'a SchemaField> {
    schema
        .fields
        .iter()
        .filter(|field| !field.can_be_empty && !field.has_default)
        .filter(|field| !resolved.iter().any(|item| item.field.name == field.name))
        .collect()
}

fn canonical(field: &SchemaField, value: &str, me: Option<&str>, validate: bool) -> Result<String> {
    let name = &field.name;
    match field.kind() {
        Kind::Bundle | Kind::State => pick(field, value, validate),
        Kind::User => {
            let login = match me {
                Some(me) if value.eq_ignore_ascii_case("me") => me,
                _ => value,
            };
            if field.values.is_empty() {
                Ok(login.to_string())
            } else {
                pick(field, login, validate)
            }
        }
        Kind::Date => NaiveDate::parse_from_str(value, "%Y-%m-%d")
            .map(|date| date.format("%Y-%m-%d").to_string())
            .map_err(|_| usage(format!("{name}: '{value}' is not a date (YYYY-MM-DD)"))),
        Kind::DateTime => date_time(value)
            .map(|millis| millis.to_string())
            .ok_or_else(|| {
                usage(format!(
                    "{name}: '{value}' is not a date and time (YYYY-MM-DD HH:MM, or RFC 3339)"
                ))
            }),
        Kind::Period => period(value).ok_or_else(|| {
            usage(format!(
                "{name}: '{value}' is not a period like 1w 2d 3h 30m"
            ))
        }),
        Kind::Integer => value
            .parse::<i64>()
            .map(|number| number.to_string())
            .map_err(|_| usage(format!("{name}: '{value}' is not a whole number"))),
        Kind::Float => value
            .parse::<f64>()
            .map(|_| value.to_string())
            .map_err(|_| usage(format!("{name}: '{value}' is not a number"))),
        Kind::String | Kind::Text | Kind::Other => Ok(value.to_string()),
    }
}

fn pick(field: &SchemaField, value: &str, validate: bool) -> Result<String> {
    if !validate {
        return Ok(value.to_string());
    }
    if let Some(allowed) = field.values.iter().find(|allowed| same(allowed, value)) {
        return Ok(allowed.clone());
    }
    let mut message = format!("{}: '{value}' is not allowed", field.name);
    if field.values.is_empty() {
        message.push_str("; the field has no values to choose from");
    } else {
        message.push_str(&format!("; choose from {}", choices(&field.values)));
        if let Some(close) = closest(value, field.values.iter().map(String::as_str)) {
            message.push_str(&format!(" (did you mean '{close}'?)"));
        }
    }
    Err(usage(message))
}

fn unknown_field(schema: &ProjectSchema, name: &str) -> miette::Report {
    if schema.fields.is_empty() {
        return usage(format!(
            "{name}: the fields of {} could not be read; pass --no-validate to send the change as given",
            schema.project
        ));
    }
    let names: Vec<String> = schema
        .fields
        .iter()
        .map(|field| field.name.clone())
        .collect();
    let mut message = format!(
        "{} has no field '{name}'; its fields are {}",
        schema.project,
        choices(&names)
    );
    if let Some(close) = closest(name, names.iter().map(String::as_str)) {
        message.push_str(&format!(" (did you mean '{close}'?)"));
    }
    usage(message)
}

fn date_time(value: &str) -> Option<i64> {
    if let Ok(parsed) = DateTime::parse_from_rfc3339(value) {
        return Some(parsed.timestamp_millis());
    }
    let naive = NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M").ok()?;
    Local
        .from_local_datetime(&naive)
        .single()
        .map(|local| local.timestamp_millis())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn field(
        name: &str,
        type_id: &str,
        multi: bool,
        can_be_empty: bool,
        values: &[&str],
    ) -> SchemaField {
        SchemaField {
            name: name.into(),
            aliases: vec![],
            type_id: type_id.into(),
            multi,
            can_be_empty,
            has_default: false,
            values: values.iter().map(|value| value.to_string()).collect(),
        }
    }

    fn schema() -> ProjectSchema {
        let mut priority = field(
            "Priority",
            "enum",
            false,
            false,
            &["Normal", "Major", "Critical"],
        );
        priority.aliases = vec!["prio".into()];
        priority.has_default = true;
        ProjectSchema {
            project: "WEB".into(),
            fields: vec![
                priority,
                field(
                    "State",
                    "state",
                    false,
                    false,
                    &["Open", "In Progress", "Fixed"],
                ),
                field("Assignee", "user", false, true, &["jane", "bob"]),
                field("Fix versions", "version", true, true, &["1.0", "1.1"]),
                field("Due Date", "date", false, true, &[]),
                field("Deadline", "date and time", false, true, &[]),
                field("Estimation", "period", false, true, &[]),
                field("Points", "integer", false, true, &[]),
                field("Notes", "text", false, true, &[]),
                field("Type", "enum", false, false, &["Bug", "Task"]),
            ],
        }
    }

    fn change(name: &str, op: Op) -> Change {
        Change {
            name: name.into(),
            op,
        }
    }

    fn set(name: &str, value: &str) -> Change {
        change(name, Op::Set(vec![value.into()]))
    }

    fn error_text(changes: &[Change]) -> String {
        format!(
            "{}",
            resolve(&schema(), changes, Some("jane"), true).unwrap_err()
        )
    }

    #[test]
    fn names_and_values_take_the_schema_spelling() {
        let resolved = resolve(
            &schema(),
            &[set("prio", "major"), set("state", "in progress")],
            None,
            true,
        )
        .unwrap();
        assert_eq!(resolved[0].field.name, "Priority");
        assert_eq!(resolved[0].op, ResolvedOp::Set(vec!["Major".into()]));
        assert_eq!(resolved[1].op, ResolvedOp::Set(vec!["In Progress".into()]));
    }

    #[test]
    fn unknown_names_and_values_suggest_the_closest() {
        let text = error_text(&[set("Priority", "Majr")]);
        assert!(text.contains("did you mean 'Major'"), "{text}");
        assert!(text.contains("Normal, Major, Critical"), "{text}");
        let text = error_text(&[set("Priorty", "Major")]);
        assert!(text.contains("did you mean 'Priority'"), "{text}");
    }

    #[test]
    fn me_is_the_signed_in_user() {
        let resolved = resolve(&schema(), &[set("Assignee", "ME")], Some("jane"), true).unwrap();
        assert_eq!(resolved[0].op, ResolvedOp::Set(vec!["jane".into()]));
    }

    #[test]
    fn multiplicity_and_emptiness_are_checked() {
        assert!(
            error_text(&[change(
                "Priority",
                Op::Set(vec!["Major".into(), "Normal".into()])
            )])
            .contains("takes one value")
        );
        assert!(
            error_text(&[change("Priority", Op::Add("Major".into()))]).contains("holds one value")
        );
        assert!(error_text(&[change("Priority", Op::Clear)]).contains("cannot be empty"));
        let resolved = resolve(
            &schema(),
            &[change(
                "Fix versions",
                Op::Set(vec!["1.0".into(), "1.1".into()]),
            )],
            None,
            true,
        )
        .unwrap();
        assert_eq!(
            resolved[0].op,
            ResolvedOp::Set(vec!["1.0".into(), "1.1".into()])
        );
    }

    #[test]
    fn typed_values_are_parsed() {
        let resolved = resolve(
            &schema(),
            &[
                set("Due Date", "2026-10-10"),
                set("Deadline", "2026-10-10T12:00:00Z"),
                set("Estimation", "1w  2d 30m"),
                set("Points", "5"),
                set("Notes", "free text"),
            ],
            None,
            true,
        )
        .unwrap();
        let values: Vec<ResolvedOp> = resolved.into_iter().map(|item| item.op).collect();
        assert_eq!(
            values,
            vec![
                ResolvedOp::Set(vec!["2026-10-10".into()]),
                ResolvedOp::Set(vec!["1791633600000".into()]),
                ResolvedOp::Set(vec!["1w 2d 30m".into()]),
                ResolvedOp::Set(vec!["5".into()]),
                ResolvedOp::Set(vec!["free text".into()]),
            ]
        );
        assert!(error_text(&[set("Due Date", "10/10/2026")]).contains("YYYY-MM-DD"));
        assert!(error_text(&[set("Estimation", "2d 1w")]).contains("period"));
        assert!(error_text(&[set("Points", "five")]).contains("whole number"));
    }

    #[test]
    fn local_date_times_use_the_local_zone() {
        let resolved = resolve(
            &schema(),
            &[set("Deadline", "2026-10-10 12:00")],
            None,
            true,
        )
        .unwrap();
        let expected = chrono::Local
            .with_ymd_and_hms(2026, 10, 10, 12, 0, 0)
            .single()
            .unwrap()
            .timestamp_millis()
            .to_string();
        assert_eq!(resolved[0].op, ResolvedOp::Set(vec![expected]));
    }

    #[test]
    fn without_validation_anything_goes_through() {
        let resolved = resolve(
            &schema(),
            &[set("Custom", "Value"), set("Priority", "whatever")],
            None,
            false,
        )
        .unwrap();
        assert_eq!(resolved[0].field, SchemaField::unknown("Custom"));
        assert_eq!(resolved[1].op, ResolvedOp::Set(vec!["whatever".into()]));
    }

    #[test]
    fn a_field_named_twice_through_an_alias_is_an_error() {
        assert!(
            error_text(&[set("Priority", "Major"), set("prio", "Normal")])
                .contains("changed twice")
        );
    }

    #[test]
    fn required_fields_without_defaults_are_missing() {
        let schema = schema();
        let missing: Vec<&str> = missing_required(&schema, &[])
            .into_iter()
            .map(|field| field.name.as_str())
            .collect();
        assert_eq!(missing, ["State", "Type"]);
        let resolved = resolve(&schema, &[set("Type", "bug")], None, true).unwrap();
        let missing: Vec<&str> = missing_required(&schema, &resolved)
            .into_iter()
            .map(|field| field.name.as_str())
            .collect();
        assert_eq!(missing, ["State"]);
    }
}
