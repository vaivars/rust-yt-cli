use futures::TryStreamExt as _;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::api::client::LIST_PAGE;
use crate::api::issues::UserRef;
use crate::api::projects::{self, WorkItemType};
use crate::api::{Client, Projection};
use crate::error::{Result, YtError};
use crate::fields::{choices, closest, same, usage};

#[derive(Clone, Debug, Deserialize)]
pub struct DurationValue {
    #[serde(default)]
    pub minutes: i64,
    #[serde(default)]
    pub presentation: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueId {
    pub id_readable: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct WorkItem {
    pub id: String,
    #[serde(default)]
    pub date: i64,
    pub duration: Option<DurationValue>,
    #[serde(rename = "type")]
    pub work_type: Option<WorkItemType>,
    pub text: Option<String>,
    pub author: Option<UserRef>,
    pub issue: Option<IssueId>,
    #[serde(default)]
    pub created: i64,
}

impl Projection for WorkItem {
    const FIELDS: &'static str = "id,date,duration(minutes,presentation),type(id,name),text,author(login,fullName),issue(idReadable),created";
}

impl WorkItem {
    pub fn minutes(&self) -> i64 {
        self.duration
            .as_ref()
            .map_or(0, |duration| duration.minutes)
    }

    pub fn presentation(&self) -> String {
        match &self.duration {
            Some(duration) if !duration.presentation.is_empty() => duration.presentation.clone(),
            _ => crate::time::hours_minutes(self.minutes()),
        }
    }
}

/// What `time log` sets or `time edit` changes; `None` leaves a part out of the request.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WorkItemChange {
    pub date: Option<i64>,
    pub duration: Option<String>,
    pub type_id: Option<String>,
    pub text: Option<String>,
}

impl WorkItemChange {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    pub fn body(&self) -> Value {
        let mut body = Map::new();
        if let Some(date) = self.date {
            body.insert("date".into(), json!(date));
        }
        if let Some(duration) = &self.duration {
            body.insert("duration".into(), json!({"presentation": duration}));
        }
        // YouTrack finds a work item type only by ID.
        if let Some(type_id) = &self.type_id {
            body.insert("type".into(), json!({"id": type_id}));
        }
        if let Some(text) = &self.text {
            body.insert("text".into(), json!(text));
        }
        Value::Object(body)
    }
}

pub async fn create(client: &Client, issue: &str, change: &WorkItemChange) -> Result<WorkItem> {
    client
        .post(
            &format!("issues/{issue}/timeTracking/workItems"),
            &change.body(),
        )
        .await
}

pub async fn update(
    client: &Client,
    issue: &str,
    item: &str,
    change: &WorkItemChange,
) -> Result<WorkItem> {
    check_item(item)?;
    client
        .post(
            &format!("issues/{issue}/timeTracking/workItems/{item}"),
            &change.body(),
        )
        .await
}

pub async fn delete(client: &Client, issue: &str, item: &str) -> Result<()> {
    check_item(item)?;
    client
        .delete(&format!("issues/{issue}/timeTracking/workItems/{item}"))
        .await
}

/// Filters for `GET workItems`; `None` leaves a filter out.
#[derive(Clone, Debug, Default)]
pub struct ListFilter {
    pub query: Option<String>,
    pub start: Option<String>,
    pub end: Option<String>,
    pub author: Option<String>,
}

/// Every matching work item, newest first; YouTrack returns them in creation order and cannot sort them.
pub async fn list(client: &Client, filter: &ListFilter) -> Result<Vec<WorkItem>> {
    let mut parameters = Vec::new();
    for (key, value) in [
        ("query", &filter.query),
        ("startDate", &filter.start),
        ("endDate", &filter.end),
        ("author", &filter.author),
    ] {
        if let Some(value) = value {
            parameters.push((key.to_string(), value.clone()));
        }
    }
    let mut items: Vec<WorkItem> = client
        .paginate("workItems".to_string(), parameters, LIST_PAGE)
        .try_collect()
        .await?;
    sort_newest_first(&mut items);
    Ok(items)
}

pub fn sort_newest_first(items: &mut [WorkItem]) {
    items.sort_by(|a, b| b.date.cmp(&a.date).then(b.created.cmp(&a.created)));
}

/// The work item types of a project, or of the instance without one.
pub async fn types(client: &Client, project: Option<&str>) -> Result<Vec<WorkItemType>> {
    let Some(project) = project else {
        return instance_types(client).await;
    };
    match projects::time_tracking(client, project).await {
        Ok(settings) if !settings.enabled => Err(usage(format!(
            "time tracking is turned off in project {project}"
        ))),
        Ok(settings) => Ok(settings.work_item_types),
        // Some instances hide project settings from non-admins; the instance-wide list still names the types.
        Err(error) if forbidden(&error) => instance_types(client).await,
        Err(error) => Err(error),
    }
}

async fn instance_types(client: &Client) -> Result<Vec<WorkItemType>> {
    client
        .get_all("admin/timeTrackingSettings/workItemTypes")
        .await
}

/// YouTrack refused the request (403), as some instances do for project settings.
pub fn forbidden(error: &miette::Report) -> bool {
    matches!(
        error.downcast_ref::<YtError>(),
        Some(YtError::Api { status: 403, .. })
    )
}

/// A type by name in any case, or by ID.
pub fn resolve_type<'a>(types: &'a [WorkItemType], name: &str) -> Result<&'a WorkItemType> {
    if let Some(found) = types
        .iter()
        .find(|candidate| same(&candidate.name, name) || candidate.id == name)
    {
        return Ok(found);
    }
    let names: Vec<String> = types
        .iter()
        .map(|candidate| candidate.name.clone())
        .collect();
    let mut message = format!("there is no work item type '{name}'");
    if let Some(close) = closest(name, names.iter().map(String::as_str)) {
        message.push_str(&format!(" (did you mean '{close}'?)"));
    } else if names.is_empty() {
        message.push_str("; the project has no work item types");
    } else {
        message.push_str(&format!("; types: {}", choices(&names)));
    }
    Err(usage(message))
}

/// Work item IDs (`270-1`) go into paths; anything else is rejected.
pub fn check_item(id: &str) -> Result<()> {
    if id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        && id.chars().any(|c| c.is_ascii_alphanumeric())
    {
        Ok(())
    } else {
        Err(usage(format!(
            "'{id}' is not a work item ID; `yt time list` shows them"
        )))
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use wiremock::matchers::{body_json, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::api::test_client;

    fn item(id: &str, date: i64, created: i64) -> serde_json::Value {
        json!({"id": id, "date": date, "duration": {"minutes": 90, "presentation": "1h 30m"}, "type": null, "text": null, "author": {"login": "jane", "fullName": "Jane"}, "issue": {"idReadable": "WEB-1"}, "created": created})
    }

    fn named_types() -> Vec<WorkItemType> {
        ["Izstrāde", "Testing"]
            .iter()
            .enumerate()
            .map(|(index, name)| WorkItemType {
                id: format!("85-{index}"),
                name: (*name).into(),
            })
            .collect()
    }

    #[test]
    fn types_match_in_any_case_and_script() {
        assert_eq!(resolve_type(&named_types(), "IZSTRĀDE").unwrap().id, "85-0");
        assert_eq!(
            resolve_type(&named_types(), "85-1").unwrap().name,
            "Testing"
        );
        let error = resolve_type(&named_types(), "Testng")
            .unwrap_err()
            .to_string();
        assert!(error.contains("did you mean 'Testing'"), "{error}");
    }

    #[test]
    fn changes_send_only_what_is_given() {
        let change = WorkItemChange {
            duration: Some("2h".into()),
            type_id: Some("85-0".into()),
            ..WorkItemChange::default()
        };
        assert_eq!(
            change.body(),
            json!({"duration": {"presentation": "2h"}, "type": {"id": "85-0"}})
        );
        assert!(WorkItemChange::default().is_empty());
        assert!(!change.is_empty());
    }

    #[test]
    fn newest_days_come_first_then_newest_created() {
        let mut items: Vec<WorkItem> = [item("a", 1, 10), item("b", 3, 5), item("c", 3, 7)]
            .into_iter()
            .map(|value| serde_json::from_value(value).unwrap())
            .collect();
        sort_newest_first(&mut items);
        let ids: Vec<&str> = items.iter().map(|item| item.id.as_str()).collect();
        assert_eq!(ids, ["c", "b", "a"]);
    }

    #[test]
    fn item_ids_cannot_steer_paths() {
        for id in ["", "..", "../x", "270_1"] {
            assert!(check_item(id).is_err(), "{id:?}");
        }
        assert!(check_item("270-1").is_ok());
    }

    #[tokio::test]
    async fn list_reads_every_page_with_the_filters_and_sorts() {
        let server = MockServer::start().await;
        let full: Vec<serde_json::Value> = (0..crate::api::client::LIST_PAGE)
            .map(|n| item(&format!("old-{n}"), 1_000, n as i64))
            .collect();
        Mock::given(method("GET"))
            .and(path("/api/workItems"))
            .and(query_param("author", "jane"))
            .and(query_param("startDate", "2026-10-01"))
            .and(query_param("query", "project: {WEB}"))
            .and(query_param("$skip", "0"))
            .respond_with(ResponseTemplate::new(200).set_body_json(full))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/workItems"))
            .and(query_param(
                "$skip",
                crate::api::client::LIST_PAGE.to_string(),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([item("new", 2_000, 0)])))
            .mount(&server)
            .await;
        let filter = ListFilter {
            query: Some("project: {WEB}".into()),
            start: Some("2026-10-01".into()),
            end: None,
            author: Some("jane".into()),
        };
        let items = list(&test_client(&server.uri()), &filter).await.unwrap();
        assert_eq!(items.len(), crate::api::client::LIST_PAGE + 1);
        assert_eq!(items[0].id, "new");
    }

    #[tokio::test]
    async fn a_project_with_time_tracking_off_has_no_types() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/admin/projects/WEB/timeTrackingSettings"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"enabled": false, "workItemTypes": []})),
            )
            .mount(&server)
            .await;
        let error = types(&test_client(&server.uri()), Some("WEB"))
            .await
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("time tracking is turned off in project WEB"),
            "{error}"
        );
    }

    #[tokio::test]
    async fn hidden_project_settings_fall_back_to_the_instance_types() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/admin/projects/WEB/timeTrackingSettings"))
            .respond_with(
                ResponseTemplate::new(403).set_body_json(
                    json!({"error": "Forbidden", "error_description": "Access denied"}),
                ),
            )
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/admin/timeTrackingSettings/workItemTypes"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!([{"id": "85-0", "name": "Development"}])),
            )
            .mount(&server)
            .await;
        let found = types(&test_client(&server.uri()), Some("WEB"))
            .await
            .unwrap();
        assert_eq!(found[0].name, "Development");
    }

    #[tokio::test]
    async fn items_are_created_updated_and_deleted_on_the_issue() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/issues/WEB-1/timeTracking/workItems"))
            .and(body_json(
                json!({"date": 1_791_590_400_000_i64, "duration": {"presentation": "1h 30m"}}),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(item(
                "270-1",
                1_791_590_400_000,
                1,
            )))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/api/issues/WEB-1/timeTracking/workItems/270-1"))
            .and(body_json(json!({"text": "Fixed"})))
            .respond_with(ResponseTemplate::new(200).set_body_json(item(
                "270-1",
                1_791_590_400_000,
                1,
            )))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("DELETE"))
            .and(path("/api/issues/WEB-1/timeTracking/workItems/270-1"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;
        let client = test_client(&server.uri());
        let created = create(
            &client,
            "WEB-1",
            &WorkItemChange {
                date: Some(1_791_590_400_000),
                duration: Some("1h 30m".into()),
                ..WorkItemChange::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(created.minutes(), 90);
        update(
            &client,
            "WEB-1",
            "270-1",
            &WorkItemChange {
                text: Some("Fixed".into()),
                ..WorkItemChange::default()
            },
        )
        .await
        .unwrap();
        delete(&client, "WEB-1", "270-1").await.unwrap();
    }
}
