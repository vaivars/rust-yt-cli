use std::collections::BTreeMap;

use reqwest::Method as HttpMethod;
use serde_json::{Map, Value};

use crate::apps::openapi::{Entity, Location, Method, Operation};
use crate::error::{Result, YtError};
use crate::instance::InstanceUrl;

/// What the command line gave an app command.
#[derive(Debug, Default, PartialEq)]
pub struct Inputs {
    /// In the order of [`Operation::positionals`].
    pub positionals: Vec<String>,
    /// Query values by flag name; a switch that was given has `true`.
    pub query: BTreeMap<String, String>,
    /// Body field values by flag name.
    pub body: BTreeMap<String, String>,
    /// A whole request body from `--input`.
    pub input: Option<Value>,
}

#[derive(Debug, PartialEq)]
pub struct Request {
    pub method: HttpMethod,
    /// Starts with `/api/`.
    pub path_and_query: String,
    pub body: Option<Value>,
}

pub fn build(operation: &Operation, inputs: &Inputs, instance: &InstanceUrl) -> Result<Request> {
    let mut path = operation.path.clone();
    let mut query = url::form_urlencoded::Serializer::new(String::new());
    for (positional, value) in operation.positionals.iter().zip(&inputs.positionals) {
        let value = entity_value(positional.entity, value, instance)?;
        match positional.location {
            Location::Path => {
                path = path.replace(
                    &format!("{{{}}}", positional.param),
                    &segment(&positional.name, &value)?,
                );
            }
            Location::Query => {
                query.append_pair(&positional.param, &value);
            }
        }
    }
    if inputs.positionals.len() != operation.positionals.len() || path.contains('{') {
        return Err(YtError::Usage {
            message: format!(
                "{} needs {} arguments",
                operation.command,
                operation.positionals.len()
            ),
        }
        .into());
    }
    for flag in &operation.flags {
        if let Some(value) = inputs.query.get(&flag.flag) {
            query.append_pair(&flag.param, &entity_value(flag.entity, value, instance)?);
        }
    }
    let query = query.finish();
    let path_and_query = if query.is_empty() {
        path
    } else {
        format!("{path}?{query}")
    };
    let body = match (&operation.body, &inputs.input) {
        (Some(_), Some(input)) => Some(input.clone()),
        (Some(body), None) => {
            let fields: Map<String, Value> = body
                .fields
                .iter()
                .filter_map(|field| {
                    let value = inputs.body.get(&field.flag)?;
                    let value = if field.text {
                        Value::String(value.clone())
                    } else {
                        serde_json::from_str(value).unwrap_or_else(|_| Value::String(value.clone()))
                    };
                    Some((field.name.clone(), value))
                })
                .collect();
            (!fields.is_empty() || body.required).then_some(Value::Object(fields))
        }
        (None, _) => None,
    };
    Ok(Request {
        method: match operation.method {
            Method::Get => HttpMethod::GET,
            Method::Post => HttpMethod::POST,
        },
        path_and_query,
        body,
    })
}

fn entity_value(entity: Option<Entity>, value: &str, instance: &InstanceUrl) -> Result<String> {
    match entity {
        Some(Entity::Issue) => instance.issue_id(value),
        Some(Entity::Article) => instance.article_id(value),
        _ => Ok(value.to_string()),
    }
}

/// One path segment: everything but unreserved characters is percent-encoded, so a value cannot add segments.
fn segment(name: &str, value: &str) -> Result<String> {
    if value.is_empty() || value == "." || value == ".." {
        return Err(YtError::Usage {
            message: format!("'{value}' is not a valid {name}"),
        }
        .into());
    }
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            out.push(char::from(byte));
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::apps::openapi::{Body, BodyField, Positional, QueryFlag, Scope};

    fn instance() -> InstanceUrl {
        InstanceUrl::parse("https://acme.youtrack.cloud").unwrap()
    }

    fn positional(location: Location, param: &str, entity: Option<Entity>) -> Positional {
        Positional {
            name: "ISSUE".into(),
            param: param.into(),
            location,
            entity,
            description: None,
        }
    }

    fn query_flag(flag: &str, param: &str) -> QueryFlag {
        QueryFlag {
            flag: flag.into(),
            param: param.into(),
            switch: false,
            required: false,
            entity: None,
            choices: vec![],
            description: None,
        }
    }

    fn get_issue(location: Location) -> Operation {
        let (path, param) = match location {
            Location::Query => ("/api/extensionEndpoints/app/global/issue", "issueId"),
            Location::Path => ("/api/issues/{id}/extensionEndpoints/app/api/issue", "id"),
        };
        Operation {
            command: "get-issue".into(),
            summary: None,
            description: None,
            method: Method::Get,
            path: path.into(),
            scope: Scope::Global,
            positionals: vec![positional(location, param, Some(Entity::Issue))],
            flags: vec![query_flag("full", "full"), query_flag("fields", "fields")],
            body: None,
        }
    }

    fn update_issue() -> Operation {
        Operation {
            command: "update-issue".into(),
            method: Method::Post,
            flags: vec![],
            body: Some(Body {
                required: true,
                fields: vec![
                    BodyField {
                        flag: "field".into(),
                        name: "field".into(),
                        text: true,
                        required: true,
                        description: None,
                    },
                    BodyField {
                        flag: "value".into(),
                        name: "value".into(),
                        text: false,
                        required: true,
                        description: None,
                    },
                ],
            }),
            ..get_issue(Location::Query)
        }
    }

    fn inputs(positionals: &[&str], query: &[(&str, &str)], body: &[(&str, &str)]) -> Inputs {
        let map = |pairs: &[(&str, &str)]| {
            pairs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect()
        };
        Inputs {
            positionals: positionals.iter().map(|p| p.to_string()).collect(),
            query: map(query),
            body: map(body),
            input: None,
        }
    }

    #[test]
    fn query_parameters_are_encoded_in_declaration_order() {
        let request = build(
            &get_issue(Location::Query),
            &inputs(
                &["WEB-1"],
                &[("full", "true"), ("fields", "HF External ID,Priority")],
                &[],
            ),
            &instance(),
        )
        .unwrap();
        assert_eq!(request.method, HttpMethod::GET);
        assert_eq!(
            request.path_and_query,
            "/api/extensionEndpoints/app/global/issue?issueId=WEB-1&full=true&fields=HF+External+ID%2CPriority"
        );
        assert_eq!(request.body, None);
    }

    #[test]
    fn an_issue_url_becomes_its_id() {
        let request = build(
            &get_issue(Location::Path),
            &inputs(
                &["https://acme.youtrack.cloud/issue/WEB-7/some-title"],
                &[],
                &[],
            ),
            &instance(),
        )
        .unwrap();
        assert_eq!(
            request.path_and_query,
            "/api/issues/WEB-7/extensionEndpoints/app/api/issue"
        );
        let error = build(
            &get_issue(Location::Path),
            &inputs(&["not an id"], &[], &[]),
            &instance(),
        );
        assert!(error.is_err());
    }

    #[test]
    fn a_path_value_stays_one_segment() {
        let mut operation = get_issue(Location::Path);
        operation.positionals[0].entity = None;
        let request = build(&operation, &inputs(&["a/b c"], &[], &[]), &instance()).unwrap();
        assert_eq!(
            request.path_and_query,
            "/api/issues/a%2Fb%20c/extensionEndpoints/app/api/issue"
        );
        for value in ["..", ".", ""] {
            assert!(
                build(&operation, &inputs(&[value], &[], &[]), &instance()).is_err(),
                "{value}"
            );
        }
    }

    #[test]
    fn body_values_are_json_unless_the_field_is_a_string() {
        let cases = [
            ("{\"text\":\"x\"}", json!({"text": "x"})),
            ("null", Value::Null),
            ("42", json!(42)),
            ("Major", json!("Major")),
        ];
        for (value, expected) in cases {
            let request = build(
                &update_issue(),
                &inputs(&["WEB-1"], &[], &[("field", "42"), ("value", value)]),
                &instance(),
            )
            .unwrap();
            assert_eq!(request.method, HttpMethod::POST);
            assert_eq!(
                request.body,
                Some(json!({"field": "42", "value": expected})),
                "{value}"
            );
        }
    }

    #[test]
    fn input_is_the_whole_body() {
        let mut given = inputs(&["WEB-1"], &[], &[]);
        given.input = Some(json!({"field": "x", "value": null}));
        let request = build(&update_issue(), &given, &instance()).unwrap();
        assert_eq!(request.body, Some(json!({"field": "x", "value": null})));
    }

    #[test]
    fn missing_arguments_are_refused() {
        assert!(
            build(
                &get_issue(Location::Query),
                &inputs(&[], &[], &[]),
                &instance()
            )
            .is_err()
        );
    }
}
