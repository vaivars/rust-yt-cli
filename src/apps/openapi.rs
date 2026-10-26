use std::collections::BTreeMap;

use serde_json::Value;

use crate::apps::registry::is_kebab;
use crate::error::{Result, YtError};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Method {
    Get,
    Post,
}

/// Where an endpoint works: `global` everywhere, the others only in projects the app is attached to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    Global,
    Project,
    Issue,
    Article,
    User,
}

impl Scope {
    fn parse(text: &str) -> Option<Self> {
        match text {
            "global" => Some(Self::Global),
            "project" => Some(Self::Project),
            "issue" => Some(Self::Issue),
            "article" => Some(Self::Article),
            "user" => Some(Self::User),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Global => "global",
            Self::Project => "project",
            Self::Issue => "issue",
            Self::Article => "article",
            Self::User => "user",
        }
    }
}

/// A YouTrack entity a parameter names, from `x-youtrack-entity`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Entity {
    Issue,
    Project,
    Article,
    User,
}

impl Entity {
    fn parse(text: &str) -> Option<Self> {
        match text {
            "issue" => Some(Self::Issue),
            "project" => Some(Self::Project),
            "article" => Some(Self::Article),
            "user" => Some(Self::User),
            _ => None,
        }
    }

    fn arg_name(self) -> &'static str {
        match self {
            Self::Issue => "ISSUE",
            Self::Project => "PROJECT",
            Self::Article => "ARTICLE",
            Self::User => "USER",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Location {
    Path,
    Query,
}

/// A required parameter given as a positional argument.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Positional {
    /// The argument's name in help, e.g. `ISSUE`.
    pub name: String,
    pub param: String,
    pub location: Location,
    pub entity: Option<Entity>,
    pub description: Option<String>,
}

/// A query parameter given as `--flag VALUE`, or `--flag` for a boolean.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QueryFlag {
    pub flag: String,
    pub param: String,
    pub switch: bool,
    pub required: bool,
    pub entity: Option<Entity>,
    pub choices: Vec<String>,
    pub description: Option<String>,
}

/// A top-level property of a JSON request body, given as `--flag VALUE`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BodyField {
    pub flag: String,
    pub name: String,
    /// A string property takes the value as typed; any other is parsed as JSON first.
    pub text: bool,
    pub required: bool,
    pub description: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Body {
    pub required: bool,
    pub fields: Vec<BodyField>,
}

/// One command: the operation yt calls for it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Operation {
    pub command: String,
    pub summary: Option<String>,
    pub description: Option<String>,
    pub method: Method,
    /// The path template, e.g. `/api/issues/{id}/extensionEndpoints/app/api/issue`.
    pub path: String,
    pub scope: Scope,
    pub positionals: Vec<Positional>,
    pub flags: Vec<QueryFlag>,
    pub body: Option<Body>,
}

/// What an app's description offers, as commands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppApi {
    pub id: String,
    pub title: Option<String>,
    pub description: Option<String>,
    pub version: String,
    /// One per command, sorted by command name.
    pub operations: Vec<Operation>,
}

/// Flags every app command has: `--input` for a raw request body.
pub const INPUT_FLAG: &str = "input";

impl AppApi {
    /// Reads the description of app `id`, skipping whatever yt does not understand: an administrator
    /// can switch endpoints off, and an app may add to its description. `taken` are flag names yt
    /// already uses, such as the global flags.
    pub fn parse(id: &str, text: &str, taken: &[&str]) -> Result<Self> {
        let document: Value = serde_json::from_str(text)
            .map_err(|error| invalid(id, format!("not JSON: {error}")))?;
        if !document["openapi"].as_str().is_some_and(supported) {
            return Err(invalid(id, "it is not OpenAPI 3.1 or newer".into()));
        }
        let info = &document["info"];
        if info["x-youtrack-app"]["id"].as_str() != Some(id) {
            return Err(invalid(id, "it describes another app".into()));
        }
        let version = info["version"]
            .as_str()
            .ok_or_else(|| invalid(id, "it has no version".into()))?;
        let reader = Reader {
            document: &document,
            id,
            taken,
        };
        let mut candidates = Vec::new();
        if let Some(paths) = document["paths"].as_object() {
            for (path, item) in paths {
                let item = reader.resolve(item);
                for (key, method) in [("get", Method::Get), ("post", Method::Post)] {
                    let Some(operation) = item.get(key) else {
                        continue;
                    };
                    match reader.operation(path, item, reader.resolve(operation), method) {
                        Ok(operation) => candidates.push(operation),
                        Err(reason) => log::debug!("{id}: skipped {key} {path}: {reason}"),
                    }
                }
            }
        }
        Ok(Self {
            id: id.to_string(),
            title: info["title"].as_str().and_then(line),
            description: info["description"].as_str().and_then(text_block),
            version: crate::output::printable(version).into_owned(),
            operations: choose(id, candidates),
        })
    }

    pub fn operation(&self, command: &str) -> Option<&Operation> {
        self.operations
            .iter()
            .find(|operation| operation.command == command)
    }
}

fn invalid(id: &str, message: String) -> miette::Report {
    YtError::AppDocument {
        id: id.to_string(),
        message,
    }
    .into()
}

fn supported(version: &str) -> bool {
    let mut parts = version.split('.');
    parts.next() == Some("3")
        && parts
            .next()
            .and_then(|minor| minor.parse::<u32>().ok())
            .is_some_and(|minor| minor >= 1)
}

/// Operations that share a command are one action at different scopes; the global one works everywhere.
fn choose(id: &str, mut candidates: Vec<Operation>) -> Vec<Operation> {
    candidates.sort_by(|a, b| {
        (a.scope != Scope::Global, &a.path, a.method == Method::Post).cmp(&(
            b.scope != Scope::Global,
            &b.path,
            b.method == Method::Post,
        ))
    });
    let mut chosen: BTreeMap<String, Operation> = BTreeMap::new();
    for operation in candidates {
        match chosen.get(&operation.command) {
            Some(first) => log::debug!(
                "{id}: {} uses {} {}, not {}",
                operation.command,
                first.scope.as_str(),
                first.path,
                operation.path
            ),
            None => {
                chosen.insert(operation.command.clone(), operation);
            }
        }
    }
    chosen.into_values().collect()
}

struct Reader<'a> {
    document: &'a Value,
    id: &'a str,
    taken: &'a [&'a str],
}

impl<'a> Reader<'a> {
    /// Follows local `$ref`s, a few levels deep.
    fn resolve(&self, mut value: &'a Value) -> &'a Value {
        for _ in 0..8 {
            let Some(pointer) = value["$ref"].as_str().and_then(|r| r.strip_prefix('#')) else {
                break;
            };
            match self.document.pointer(pointer) {
                Some(target) => value = target,
                None => return &Value::Null,
            }
        }
        value
    }

    fn operation(
        &self,
        path: &str,
        item: &Value,
        operation: &Value,
        method: Method,
    ) -> Result<Operation, String> {
        let (scope, path_param) =
            scope_of(path, self.id).ok_or("its path is not one of the app's endpoints")?;
        if let Some(declared) = item["x-youtrack-scope"].as_str()
            && Scope::parse(declared) != Some(scope)
        {
            return Err(format!("its scope '{declared}' does not match its path"));
        }
        let command = [
            operation["x-youtrack-command"].as_str(),
            operation["operationId"].as_str(),
        ]
        .into_iter()
        .flatten()
        .find(|name| is_kebab(name) && *name != "help")
        .ok_or("it has no usable command name")?;
        let mut positionals = Vec::new();
        let mut flags: Vec<QueryFlag> = Vec::new();
        let mut path_param_declared = false;
        for parameter in self.parameters(item, operation) {
            let Some(name) = parameter["name"].as_str().filter(|name| !name.is_empty()) else {
                continue;
            };
            let required = parameter["required"].as_bool().unwrap_or(false);
            let entity = parameter["x-youtrack-entity"]
                .as_str()
                .and_then(Entity::parse);
            let description = parameter["description"].as_str().and_then(text_block);
            let kind = kind(self.resolve(&parameter["schema"]));
            let skip = |reason: String| -> Result<(), String> {
                if required {
                    Err(reason)
                } else {
                    log::debug!("{}: {command}: skipped parameter {name}: {reason}", self.id);
                    Ok(())
                }
            };
            match parameter["in"].as_str() {
                Some("path") => {
                    if path_param.as_deref() != Some(name) {
                        return Err(format!("path parameter {name} is not in its path"));
                    }
                    path_param_declared = true;
                    positionals.push(Positional {
                        name: arg_name(name, entity)
                            .ok_or("a path parameter has no usable name")?,
                        param: name.to_string(),
                        location: Location::Path,
                        entity,
                        description,
                    });
                }
                Some("query") if required && entity.is_some() => positionals.push(Positional {
                    name: arg_name(name, entity).ok_or("a parameter has no usable name")?,
                    param: name.to_string(),
                    location: Location::Query,
                    entity,
                    description,
                }),
                Some("query") => {
                    let Some(kind) = kind else {
                        skip("its type is not a string, number or boolean".into())?;
                        continue;
                    };
                    let Some(flag) = self.flag_name(name, &flags, &[]) else {
                        skip("it has no usable flag name".into())?;
                        continue;
                    };
                    flags.push(QueryFlag {
                        flag,
                        param: name.to_string(),
                        switch: kind == Kind::Boolean,
                        required,
                        entity,
                        choices: choices(self.resolve(&parameter["schema"])),
                        description,
                    });
                }
                other => skip(format!(
                    "parameters in {} are not supported",
                    other.unwrap_or("an unknown place")
                ))?,
            }
        }
        if path_param.is_some() && !path_param_declared {
            return Err("its path has an undeclared parameter".into());
        }
        let mut names: Vec<&str> = positionals.iter().map(|p| p.name.as_str()).collect();
        names.sort_unstable();
        if names.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err("two arguments have the same name".into());
        }
        let body = match method {
            Method::Get => None,
            Method::Post => self.body(operation, &flags)?,
        };
        Ok(Operation {
            command: command.to_string(),
            summary: operation["summary"].as_str().and_then(line),
            description: operation["description"].as_str().and_then(text_block),
            method,
            path: path.to_string(),
            scope,
            positionals,
            flags,
            body,
        })
    }

    /// The path's parameters, overridden by the operation's own with the same name and place.
    fn parameters(&self, item: &'a Value, operation: &'a Value) -> Vec<&'a Value> {
        let mut parameters: Vec<&Value> = Vec::new();
        for list in [&item["parameters"], &operation["parameters"]] {
            for parameter in list.as_array().into_iter().flatten() {
                let parameter = self.resolve(parameter);
                parameters.retain(|existing| {
                    existing["name"] != parameter["name"] || existing["in"] != parameter["in"]
                });
                parameters.push(parameter);
            }
        }
        parameters
    }

    fn body(&self, operation: &Value, flags: &[QueryFlag]) -> Result<Option<Body>, String> {
        let request = self.resolve(&operation["requestBody"]);
        if request.is_null() {
            return Ok(None);
        }
        let required = request["required"].as_bool().unwrap_or(false);
        let Some(schema) = request["content"]
            .as_object()
            .and_then(|content| {
                content
                    .iter()
                    .find(|(media, _)| media.split(';').next() == Some("application/json"))
            })
            .map(|(_, media)| self.resolve(&media["schema"]))
        else {
            return if required {
                Err("its request body is not JSON".into())
            } else {
                Ok(None)
            };
        };
        let required_fields: Vec<&str> = schema["required"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect();
        let mut fields: Vec<BodyField> = Vec::new();
        for (name, property) in schema["properties"].as_object().into_iter().flatten() {
            let Some(flag) = self.flag_name(name, flags, &fields) else {
                log::debug!(
                    "{}: skipped body field {name}: no usable flag name",
                    self.id
                );
                continue;
            };
            let property = self.resolve(property);
            fields.push(BodyField {
                flag,
                name: name.clone(),
                text: property["type"].as_str() == Some("string"),
                required: required_fields.contains(&name.as_str()),
                description: property["description"].as_str().and_then(text_block),
            });
        }
        Ok(Some(Body { required, fields }))
    }

    /// `name` in kebab case, unless another flag already has it.
    fn flag_name(&self, name: &str, flags: &[QueryFlag], fields: &[BodyField]) -> Option<String> {
        let flag = kebab(name)?;
        let used = self.taken.contains(&flag.as_str())
            || ["help", "version", INPUT_FLAG].contains(&flag.as_str())
            || flags.iter().any(|other| other.flag == flag)
            || fields.iter().any(|other| other.flag == flag);
        (!used).then_some(flag)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Value,
    Boolean,
}

/// None for arrays and objects, which a single flag value cannot carry.
fn kind(schema: &Value) -> Option<Kind> {
    let types: Vec<&str> = match &schema["type"] {
        Value::String(single) => vec![single.as_str()],
        Value::Array(several) => several
            .iter()
            .filter_map(Value::as_str)
            .filter(|kind| *kind != "null")
            .collect(),
        _ => Vec::new(),
    };
    match types.as_slice() {
        [] => Some(Kind::Value),
        ["boolean"] => Some(Kind::Boolean),
        kinds
            if kinds
                .iter()
                .all(|kind| matches!(*kind, "string" | "integer" | "number")) =>
        {
            Some(Kind::Value)
        }
        _ => None,
    }
}

fn choices(schema: &Value) -> Vec<String> {
    schema["enum"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(|choice| crate::output::printable(choice).into_owned())
        .collect()
}

/// The scope a path belongs to, and its path parameter, if it is one of app `id`'s endpoints.
fn scope_of(path: &str, id: &str) -> Option<(Scope, Option<String>)> {
    let rest = path.strip_prefix("/api/")?;
    let marker = format!("extensionEndpoints/{id}/");
    let (scope, param, handler) = match rest.strip_prefix(&marker) {
        Some(handler) => (Scope::Global, None, handler),
        None => {
            let (scope, after) = [
                ("issues/", Scope::Issue),
                ("admin/projects/", Scope::Project),
                ("articles/", Scope::Article),
                ("users/", Scope::User),
            ]
            .into_iter()
            .find_map(|(prefix, scope)| rest.strip_prefix(prefix).map(|after| (scope, after)))?;
            let (segment, after) = after.split_once('/')?;
            let name = segment.strip_prefix('{')?.strip_suffix('}')?;
            if name.is_empty()
                || !name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
            {
                return None;
            }
            (scope, Some(name.to_string()), after.strip_prefix(&marker)?)
        }
    };
    handler
        .split('/')
        .all(plain_segment)
        .then_some((scope, param))
}

fn plain_segment(segment: &str) -> bool {
    !segment.is_empty()
        && segment != "."
        && segment != ".."
        && segment
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '~' | '-'))
}

/// `issueId` → `issue-id`.
fn kebab(name: &str) -> Option<String> {
    let mut out = String::with_capacity(name.len() + 4);
    let mut after_word = false;
    for c in name.chars() {
        if c.is_ascii_uppercase() {
            if after_word {
                out.push('-');
            }
            out.push(c.to_ascii_lowercase());
            after_word = false;
        } else if matches!(c, '_' | '-' | ' ') {
            out.push('-');
            after_word = false;
        } else {
            out.push(c);
            after_word = c.is_ascii_lowercase() || c.is_ascii_digit();
        }
    }
    is_kebab(&out).then_some(out)
}

fn arg_name(name: &str, entity: Option<Entity>) -> Option<String> {
    match entity {
        Some(entity) => Some(entity.arg_name().to_string()),
        None => kebab(name).map(|name| name.to_uppercase().replace('-', "_")),
    }
}

/// The first line of server text, safe for a terminal.
fn line(text: &str) -> Option<String> {
    let first = text.lines().map(str::trim).find(|line| !line.is_empty())?;
    Some(crate::output::printable(first).into_owned())
}

fn text_block(text: &str) -> Option<String> {
    let trimmed = text.trim();
    (!trimmed.is_empty()).then(|| crate::output::printable(trimmed).into_owned())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    const GLOBAL: &str = "/api/extensionEndpoints/hidden-fields/global/issue";
    const ATTACHED: &str = "/api/issues/{id}/extensionEndpoints/hidden-fields/api/issue";

    fn issue_id() -> Value {
        json!({"name": "issueId", "in": "query", "required": true, "schema": {"type": "string"}, "x-youtrack-entity": "issue"})
    }

    fn path_id() -> Value {
        json!({"name": "id", "in": "path", "required": true, "schema": {"type": "string"}, "x-youtrack-entity": "issue"})
    }

    fn operations(suffix: &str, command: Option<&str>) -> Value {
        let mut get = json!({
            "operationId": format!("get-issue{suffix}"),
            "summary": "Get an issue, including hidden fields",
            "parameters": [
                {"name": "fields", "in": "query", "required": false, "schema": {"type": "string"}},
                {"name": "full", "in": "query", "required": false, "schema": {"type": "boolean"}}
            ],
            "responses": {}
        });
        let mut post = json!({
            "operationId": format!("update-issue{suffix}"),
            "summary": "Set one exposed field",
            "parameters": [{"name": "full", "in": "query", "schema": {"type": "string"}}],
            "requestBody": {"required": true, "content": {"application/json": {"schema": {"$ref": "#/components/schemas/UpdateRequest"}}}},
            "responses": {}
        });
        if let Some(command) = command {
            get["x-youtrack-command"] = json!(command);
            post["x-youtrack-command"] = json!(command.replace("get", "update"));
        }
        json!({"get": get, "post": post})
    }

    fn document(paths: Value) -> Value {
        json!({
            "openapi": "3.1.0",
            "info": {"title": "Hidden Fields API", "version": "1.2.0", "x-youtrack-app": {"id": "hidden-fields"}},
            "paths": paths,
            "components": {"schemas": {"UpdateRequest": {
                "type": "object",
                "required": ["field", "value"],
                "properties": {"field": {"type": "string"}, "value": {"description": "any JSON"}}
            }}}
        })
    }

    fn both(command: Option<&str>) -> Value {
        let mut global = operations("", command);
        global["x-youtrack-scope"] = json!("global");
        global["parameters"] = json!([issue_id()]);
        let mut attached = operations("-attached", command.map(|_| "get-issue"));
        attached["x-youtrack-scope"] = json!("issue");
        attached["parameters"] = json!([path_id()]);
        document(json!({GLOBAL: global, ATTACHED: attached}))
    }

    fn parse(document: &Value) -> AppApi {
        AppApi::parse(
            "hidden-fields",
            &document.to_string(),
            &["format", "profile"],
        )
        .unwrap()
    }

    fn commands(api: &AppApi) -> Vec<&str> {
        api.operations
            .iter()
            .map(|op| op.command.as_str())
            .collect()
    }

    #[test]
    fn each_operation_id_is_a_command_without_the_extension() {
        let api = parse(&both(None));
        assert_eq!(api.version, "1.2.0");
        assert_eq!(
            commands(&api),
            [
                "get-issue",
                "get-issue-attached",
                "update-issue",
                "update-issue-attached"
            ]
        );
    }

    #[test]
    fn a_shared_command_prefers_the_global_operation() {
        let api = parse(&both(Some("get-issue")));
        assert_eq!(commands(&api), ["get-issue", "update-issue"]);
        let get = api.operation("get-issue").unwrap();
        assert_eq!(
            (get.scope, get.path.as_str(), get.method),
            (Scope::Global, GLOBAL, Method::Get)
        );
        assert_eq!(
            get.positionals,
            [Positional {
                name: "ISSUE".into(),
                param: "issueId".into(),
                location: Location::Query,
                entity: Some(Entity::Issue),
                description: None,
            }]
        );
        let flags: Vec<(&str, bool)> = get
            .flags
            .iter()
            .map(|f| (f.flag.as_str(), f.switch))
            .collect();
        assert_eq!(flags, [("fields", false), ("full", true)]);
        let update = api.operation("update-issue").unwrap();
        assert_eq!(update.method, Method::Post);
        let body = update.body.as_ref().unwrap();
        assert!(body.required);
        let fields: Vec<(&str, bool, bool)> = body
            .fields
            .iter()
            .map(|f| (f.flag.as_str(), f.text, f.required))
            .collect();
        assert_eq!(fields, [("field", true, true), ("value", false, true)]);
    }

    #[test]
    fn with_only_attached_endpoints_the_command_uses_the_path_parameter() {
        let mut attached = operations("-attached", Some("get-issue"));
        attached["parameters"] = json!([path_id()]);
        let api = parse(&document(json!({ATTACHED: attached})));
        let get = api.operation("get-issue").unwrap();
        assert_eq!((get.scope, get.path.as_str()), (Scope::Issue, ATTACHED));
        assert_eq!(get.positionals[0].name, "ISSUE");
        assert_eq!(get.positionals[0].location, Location::Path);
    }

    #[test]
    fn a_description_without_operations_has_no_commands() {
        assert!(parse(&document(json!({}))).operations.is_empty());
        let mut no_paths = document(json!({}));
        no_paths.as_object_mut().unwrap().remove("paths");
        assert!(parse(&no_paths).operations.is_empty());
    }

    #[test]
    fn only_a_description_of_this_app_is_accepted() {
        let mut other = document(json!({}));
        other["info"]["x-youtrack-app"]["id"] = json!("other-app");
        let mut old = document(json!({}));
        old["openapi"] = json!("3.0.3");
        let mut unversioned = document(json!({}));
        unversioned["info"]
            .as_object_mut()
            .unwrap()
            .remove("version");
        for text in [
            other.to_string(),
            old.to_string(),
            unversioned.to_string(),
            "<html>".into(),
        ] {
            let error = AppApi::parse("hidden-fields", &text, &[]).unwrap_err();
            assert!(
                matches!(
                    error.downcast_ref::<YtError>(),
                    Some(YtError::AppDocument { .. })
                ),
                "{error:?}"
            );
        }
        let mut newer = document(json!({}));
        newer["openapi"] = json!("3.2.0");
        assert!(AppApi::parse("hidden-fields", &newer.to_string(), &[]).is_ok());
    }

    #[test]
    fn paths_outside_the_apps_own_endpoints_are_skipped() {
        let get = json!({"operationId": "x", "responses": {}});
        for path in [
            "/api/admin/projects",
            "/api/extensionEndpoints/other-app/global/issue",
            "/api/extensionEndpoints/hidden-fields/../../admin/users",
            "/api/extensionEndpoints/hidden-fields/%2e%2e/x",
            "/api/extensionEndpoints/hidden-fields/",
            "/api/extensionEndpoints/hidden-fields/global/{x}",
            "/api/issues/{id}/extensionEndpoints/hidden-fields/api/issue",
            "/api/issues/WEB-1/extensionEndpoints/hidden-fields/api/issue",
            "https://evil.example/api/extensionEndpoints/hidden-fields/global/issue",
            "/hub/api/extensionEndpoints/hidden-fields/global/issue",
        ] {
            let api = parse(&document(json!({path: {"get": get}})));
            assert!(api.operations.is_empty(), "{path}");
        }
    }

    #[test]
    fn a_scope_that_contradicts_the_path_is_skipped() {
        let mut global = operations("", None);
        global["x-youtrack-scope"] = json!("issue");
        global["parameters"] = json!([issue_id()]);
        assert!(
            parse(&document(json!({GLOBAL: global})))
                .operations
                .is_empty()
        );
    }

    #[test]
    fn what_yt_does_not_understand_is_skipped() {
        let get = |parameters: Value| json!({"get": {"operationId": "get-it", "parameters": parameters, "responses": {}}});
        let header = json!({"name": "X-Trace", "in": "header", "required": false, "schema": {"type": "string"}});
        let array =
            json!({"name": "ids", "in": "query", "required": false, "schema": {"type": "array"}});
        let api = parse(&document(
            json!({GLOBAL: get(json!([header, array, issue_id()]))}),
        ));
        let op = api.operation("get-it").unwrap();
        assert!(op.flags.is_empty());
        assert_eq!(op.positionals.len(), 1);
        let required_header = json!({"name": "X-Trace", "in": "header", "required": true});
        let collides = json!({"name": "format", "in": "query", "required": true, "schema": {"type": "string"}});
        for parameters in [json!([required_header]), json!([collides])] {
            assert!(
                parse(&document(json!({GLOBAL: get(parameters)})))
                    .operations
                    .is_empty()
            );
        }
        let put = json!({"put": {"operationId": "put-it", "responses": {}}, "delete": {"operationId": "delete-it", "responses": {}}});
        assert!(parse(&document(json!({GLOBAL: put}))).operations.is_empty());
        let unnamed = json!({"get": {"operationId": "Get It", "responses": {}}});
        assert!(
            parse(&document(json!({GLOBAL: unnamed})))
                .operations
                .is_empty()
        );
    }

    #[test]
    fn optional_parameters_that_collide_are_dropped() {
        let parameters = json!([
            {"name": "profile", "in": "query", "schema": {"type": "string"}},
            {"name": "input", "in": "query", "schema": {"type": "string"}},
            {"name": "issueId", "in": "query", "schema": {"type": "string"}},
            {"name": "issue_id", "in": "query", "schema": {"type": "string"}}
        ]);
        let api = parse(&document(
            json!({GLOBAL: {"get": {"operationId": "get-it", "parameters": parameters, "responses": {}}}}),
        ));
        let flags: Vec<&str> = api.operations[0]
            .flags
            .iter()
            .map(|f| f.flag.as_str())
            .collect();
        assert_eq!(flags, ["issue-id"]);
    }

    #[test]
    fn server_text_is_made_safe_for_a_terminal() {
        let get = json!({"get": {"operationId": "get-it", "summary": "Get\u{1b}[2J it\nsecond line", "responses": {}}});
        let api = parse(&document(json!({GLOBAL: get})));
        assert_eq!(
            api.operations[0].summary.as_deref(),
            Some("Get\u{fffd}[2J it")
        );
    }

    #[test]
    fn the_captured_description_of_the_app_parses() {
        let text = include_str!("../../tests/fixtures/apps/hidden-fields.openapi.json");
        let api = AppApi::parse("hidden-fields", text, &["format"]).unwrap();
        assert_eq!(commands(&api), ["get-issue", "update-issue"]);
        let get = api.operation("get-issue").unwrap();
        assert_eq!(
            (get.scope, get.positionals[0].param.as_str()),
            (Scope::Global, "issueId")
        );
        let full = get.flags.iter().find(|f| f.flag == "full").unwrap();
        assert!(full.switch);
    }

    #[test]
    fn names_become_flags_and_arguments() {
        assert_eq!(kebab("issueId").as_deref(), Some("issue-id"));
        assert_eq!(kebab("full").as_deref(), Some("full"));
        assert_eq!(kebab("issue_id").as_deref(), Some("issue-id"));
        assert_eq!(kebab("$top"), None);
        assert_eq!(arg_name("projectId", None).as_deref(), Some("PROJECT_ID"));
        assert_eq!(
            arg_name("id", Some(Entity::Article)).as_deref(),
            Some("ARTICLE")
        );
    }
}
