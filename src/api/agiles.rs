use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::api::issues::{IssueRow, ProjectRef, UserRef};
use crate::api::{Client, Projection};
use crate::error::Result;
use crate::fields::{choices, closest, same, usage};

#[derive(Clone, Debug, Deserialize)]
pub struct Named {
    pub name: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct SprintRef {
    pub id: String,
    pub name: String,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SprintsSettings {
    #[serde(default)]
    pub disable_sprints: bool,
    #[serde(default)]
    pub is_explicit: bool,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Column {
    #[serde(default)]
    pub presentation: String,
    #[serde(default)]
    pub is_resolved: bool,
    #[serde(default)]
    pub field_values: Vec<Named>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct ColumnSettings {
    pub field: Option<Named>,
    #[serde(default)]
    pub columns: Vec<Column>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct SwimlaneSettings {
    #[serde(default)]
    pub enabled: bool,
    pub field: Option<Named>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Board {
    pub id: String,
    pub name: String,
    pub owner: Option<UserRef>,
    #[serde(default)]
    pub projects: Vec<ProjectRef>,
    pub sprints_settings: Option<SprintsSettings>,
    pub current_sprint: Option<SprintRef>,
    pub column_settings: Option<ColumnSettings>,
    pub swimlane_settings: Option<SwimlaneSettings>,
}

impl Projection for Board {
    const FIELDS: &'static str = "id,name,owner(login,fullName),projects(shortName,name),sprintsSettings(disableSprints,isExplicit),currentSprint(id,name),columnSettings(field(name),columns(presentation,isResolved,fieldValues(name))),swimlaneSettings(enabled,field(name))";
}

impl Board {
    pub fn sprints_enabled(&self) -> bool {
        !self
            .sprints_settings
            .as_ref()
            .is_some_and(|settings| settings.disable_sprints)
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct Sprint {
    pub id: String,
    pub name: String,
    pub goal: Option<String>,
    pub start: Option<i64>,
    pub finish: Option<i64>,
    #[serde(default)]
    pub archived: bool,
}

impl Projection for Sprint {
    const FIELDS: &'static str = "id,name,goal,start,finish,archived";
}

pub async fn boards(client: &Client) -> Result<Vec<Board>> {
    client.get_all("agiles").await
}

pub fn find_board<'a>(boards: &'a [Board], name_or_id: &str) -> Result<&'a Board> {
    find_named(
        boards,
        name_or_id,
        |board| &board.id,
        |board| &board.name,
        |board| {
            let projects: Vec<&str> = board
                .projects
                .iter()
                .map(|project| project.short_name.as_str())
                .collect();
            if projects.is_empty() {
                "no projects".to_string()
            } else {
                projects.join(", ")
            }
        },
        "board",
    )
}

/// A sprint's dates: `2026-10-12 – 2026-10-19`, `from 2026-10-12`, `until 2026-10-19`, or none.
pub fn sprint_dates(sprint: &Sprint) -> Option<String> {
    match (
        sprint.start.map(crate::time::date),
        sprint.finish.map(crate::time::date),
    ) {
        (Some(start), Some(finish)) => Some(format!("{start} – {finish}")),
        (Some(start), None) => Some(format!("from {start}")),
        (None, Some(finish)) => Some(format!("until {finish}")),
        (None, None) => None,
    }
}

pub async fn sprints(client: &Client, board: &str) -> Result<Vec<Sprint>> {
    client.get_all(&format!("agiles/{board}/sprints")).await
}

pub async fn sprint(client: &Client, board: &str, sprint: &str) -> Result<Sprint> {
    client
        .get(&format!("agiles/{board}/sprints/{sprint}"), &[])
        .await
}

pub fn find_sprint<'a>(sprints: &'a [Sprint], name_or_id: &str) -> Result<&'a Sprint> {
    find_named(
        sprints,
        name_or_id,
        |sprint| &sprint.id,
        |sprint| &sprint.name,
        |sprint| sprint_dates(sprint).unwrap_or_else(|| "no dates".to_string()),
        "sprint",
    )
}

/// An item by exact ID, else by its only case-insensitive name match.
fn find_named<'a, T>(
    items: &'a [T],
    name_or_id: &str,
    id: fn(&T) -> &str,
    name: fn(&T) -> &str,
    detail: fn(&T) -> String,
    what: &str,
) -> Result<&'a T> {
    if let Some(found) = items.iter().find(|item| id(item) == name_or_id) {
        return Ok(found);
    }
    let named: Vec<&T> = items
        .iter()
        .filter(|item| same(name(item), name_or_id))
        .collect();
    match named.as_slice() {
        [one] => Ok(one),
        [] => {
            let names: Vec<String> = items.iter().map(|item| name(item).to_string()).collect();
            let mut message = format!("there is no {what} '{name_or_id}'");
            if let Some(close) = closest(name_or_id, names.iter().map(String::as_str)) {
                message.push_str(&format!(" (did you mean '{close}'?)"));
            } else if !names.is_empty() {
                message.push_str(&format!("; {what}s: {}", choices(&names)));
            }
            Err(usage(message))
        }
        several => Err(usage(format!(
            "{} {what}s are named '{name_or_id}'; pick one by ID: {}",
            several.len(),
            several
                .iter()
                .map(|item| format!("{} ({})", id(item), detail(item)))
                .collect::<Vec<_>>()
                .join(", ")
        ))),
    }
}

pub async fn sprint_issues(client: &Client, board: &str, sprint: &str) -> Result<Vec<IssueRow>> {
    client
        .get_all(&format!("agiles/{board}/sprints/{sprint}/issues"))
        .await
}

/// A board column and its issues; `name: None` collects issues whose value is on no column.
#[derive(Debug)]
pub struct ColumnGroup<'a> {
    pub name: Option<String>,
    pub resolved: bool,
    pub issues: Vec<&'a IssueRow>,
}

pub fn group_by_column<'a>(board: &Board, issues: &'a [IssueRow]) -> Vec<ColumnGroup<'a>> {
    let settings = board.column_settings.clone().unwrap_or_default();
    let field = settings.field.as_ref().map(|field| field.name.clone());
    let mut groups: Vec<ColumnGroup<'a>> = settings
        .columns
        .iter()
        .map(|column| ColumnGroup {
            name: Some(column.presentation.clone()),
            resolved: column.is_resolved,
            issues: Vec::new(),
        })
        .collect();
    let mut other = Vec::new();
    for issue in issues {
        let value = field.as_deref().and_then(|field| {
            issue
                .custom_fields
                .iter()
                .find(|candidate| same(&candidate.name, field))
                .map(|candidate| candidate.display())
        });
        let column = value.as_deref().and_then(|value| {
            settings.columns.iter().position(|column| {
                column
                    .field_values
                    .iter()
                    .any(|field_value| field_value.name == value)
            })
        });
        match column {
            Some(index) => groups[index].issues.push(issue),
            None => other.push(issue),
        }
    }
    if !other.is_empty() {
        groups.push(ColumnGroup {
            name: None,
            resolved: false,
            issues: other,
        });
    }
    groups
}

pub fn sprint_body(
    name: &str,
    goal: Option<&str>,
    start: Option<i64>,
    finish: Option<i64>,
) -> Value {
    let mut body = Map::new();
    body.insert("name".into(), json!(name));
    if let Some(goal) = goal {
        body.insert("goal".into(), json!(goal));
    }
    if let Some(start) = start {
        body.insert("start".into(), json!(start));
    }
    if let Some(finish) = finish {
        body.insert("finish".into(), json!(finish));
    }
    Value::Object(body)
}

pub async fn create_sprint(client: &Client, board: &str, body: &Value) -> Result<Sprint> {
    client.post(&format!("agiles/{board}/sprints"), body).await
}

pub async fn add_issue(client: &Client, board: &str, sprint: &str, issue: &str) -> Result<()> {
    client
        .post_unit(
            &format!("agiles/{board}/sprints/{sprint}/issues"),
            &json!({"idReadable": issue}),
        )
        .await
}

#[derive(Deserialize)]
struct EntityId {
    id: String,
}

impl Projection for EntityId {
    const FIELDS: &'static str = "id";
}

/// Removal takes the issue's entity ID; YouTrack does not find a sprint issue by its readable ID here.
pub async fn remove_issue(client: &Client, board: &str, sprint: &str, issue: &str) -> Result<()> {
    let entity: EntityId = client.get(&format!("issues/{issue}"), &[]).await?;
    client
        .delete(&format!(
            "agiles/{board}/sprints/{sprint}/issues/{}",
            entity.id
        ))
        .await
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use wiremock::matchers::{body_json, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::api::test_client;

    fn board(id: &str, name: &str) -> Board {
        serde_json::from_value(json!({
            "id": id,
            "name": name,
            "projects": [{"shortName": "WEB", "name": "Web"}],
            "sprintsSettings": {"disableSprints": true, "isExplicit": false},
            "currentSprint": {"id": "111-0", "name": "First sprint"},
            "columnSettings": {"field": {"name": "Stage"}, "columns": [
                {"presentation": "Procesā", "isResolved": false, "fieldValues": [{"name": "Procesā"}]},
                {"presentation": "Darīts", "isResolved": true, "fieldValues": [{"name": "Darīts"}]}
            ]}
        }))
        .unwrap()
    }

    fn issue(id: &str, stage: Option<&str>) -> IssueRow {
        serde_json::from_value(json!({
            "idReadable": id,
            "summary": id,
            "project": {"shortName": "WEB", "name": "Web"},
            "created": 0,
            "updated": 0,
            "resolved": null,
            "customFields": stage.map(|stage| vec![json!({
                "name": "Stage",
                "$type": "SingleEnumIssueCustomField",
                "projectCustomField": {"field": {"fieldType": {"id": "enum[1]"}}},
                "value": {"name": stage, "$type": "EnumBundleElement"}
            })]).unwrap_or_default()
        }))
        .unwrap()
    }

    #[test]
    fn boards_match_by_id_or_name_in_any_case_and_script() {
        let boards = [
            board("110-0", "Web Kanban"),
            board("110-1", "Mājaslapa Kanban Board"),
        ];
        assert_eq!(
            find_board(&boards, "mājaslapa kanban board").unwrap().id,
            "110-1"
        );
        assert_eq!(find_board(&boards, "110-0").unwrap().name, "Web Kanban");
        let error = find_board(&boards, "Web Kanbn").unwrap_err().to_string();
        assert!(error.contains("did you mean 'Web Kanban'"), "{error}");
    }

    #[test]
    fn two_boards_with_one_name_ask_for_an_id() {
        let boards = [board("110-0", "Team"), board("110-5", "team")];
        let error = find_board(&boards, "Team").unwrap_err().to_string();
        assert!(error.contains("110-0 (WEB), 110-5 (WEB)"), "{error}");
    }

    #[test]
    fn two_sprints_with_one_name_show_their_dates() {
        let sprints: Vec<Sprint> = serde_json::from_value(json!([
            {"id": "111-0", "name": "Sprint", "goal": null, "start": 1_791_763_200_000_i64, "finish": 1_792_368_000_000_i64, "archived": false},
            {"id": "111-1", "name": "Sprint", "goal": null, "start": null, "finish": null, "archived": true}
        ]))
        .unwrap();
        let error = find_sprint(&sprints, "sprint").unwrap_err().to_string();
        assert!(
            error.contains("111-0 (2026-10-12 – 2026-10-19), 111-1 (no dates)"),
            "{error}"
        );
    }

    #[test]
    fn issues_group_under_their_column_with_the_rest_under_other() {
        let board = board("110-0", "Web");
        let issues = [
            issue("WEB-1", Some("Procesā")),
            issue("WEB-2", Some("Darīts")),
            issue("WEB-3", Some("Ideja")),
            issue("WEB-4", None),
        ];
        let groups = group_by_column(&board, &issues);
        let shape: Vec<(Option<&str>, bool, Vec<&str>)> = groups
            .iter()
            .map(|group| {
                (
                    group.name.as_deref(),
                    group.resolved,
                    group
                        .issues
                        .iter()
                        .map(|issue| issue.id_readable.as_str())
                        .collect(),
                )
            })
            .collect();
        assert_eq!(
            shape,
            [
                (Some("Procesā"), false, vec!["WEB-1"]),
                (Some("Darīts"), true, vec!["WEB-2"]),
                (None, false, vec!["WEB-3", "WEB-4"]),
            ]
        );
    }

    #[test]
    fn sprints_are_off_when_the_board_says_so() {
        assert!(!board("110-0", "Web").sprints_enabled());
    }

    #[test]
    fn sprint_bodies_leave_out_what_is_not_given() {
        assert_eq!(
            sprint_body("Sprint 1", None, Some(1), None),
            json!({"name": "Sprint 1", "start": 1})
        );
    }

    #[tokio::test]
    async fn sprint_writes_use_the_sprint_paths() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/agiles/110-0/sprints"))
            .and(body_json(json!({"name": "Sprint 1", "goal": "Ship"})))
            .respond_with(ResponseTemplate::new(200).set_body_json(
                json!({"id": "111-2", "name": "Sprint 1", "goal": "Ship", "start": null, "finish": null, "archived": false}),
            ))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/api/agiles/110-0/sprints/111-2/issues"))
            .and(body_json(json!({"idReadable": "WEB-1"})))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/issues/WEB-1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "2-19"})))
            .mount(&server)
            .await;
        Mock::given(method("DELETE"))
            .and(path("/api/agiles/110-0/sprints/111-2/issues/2-19"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;
        let client = test_client(&server.uri());
        let sprint = create_sprint(
            &client,
            "110-0",
            &sprint_body("Sprint 1", Some("Ship"), None, None),
        )
        .await
        .unwrap();
        assert_eq!(sprint.id, "111-2");
        add_issue(&client, "110-0", "111-2", "WEB-1").await.unwrap();
        remove_issue(&client, "110-0", "111-2", "WEB-1")
            .await
            .unwrap();
    }
}
