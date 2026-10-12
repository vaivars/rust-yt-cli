use serde::Deserialize;

use crate::api::issues::UserRef;
use crate::api::{Client, Projection};
use crate::error::Result;
use crate::fields::schema::check_short_name;
use crate::fields::{choices, closest, same, usage};

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    pub short_name: String,
    #[serde(default)]
    pub name: String,
    pub leader: Option<UserRef>,
    pub description: Option<String>,
    #[serde(default)]
    pub archived: bool,
}

impl Projection for Project {
    const FIELDS: &'static str = "shortName,name,leader(login,fullName),description,archived";
}

/// Every project the user can see, archived ones included.
pub async fn list(client: &Client) -> Result<Vec<Project>> {
    client.get_all("admin/projects").await
}

/// Short names of the projects the user can see, without archived ones.
pub async fn short_names(client: &Client) -> Result<Vec<String>> {
    Ok(list(client)
        .await?
        .into_iter()
        .filter(|project| !project.archived)
        .map(|project| project.short_name)
        .collect())
}

/// The project with this short name in any case; otherwise an error naming the closest one.
pub fn find<'a>(projects: &'a [Project], name: &str) -> Result<&'a Project> {
    if let Some(project) = projects
        .iter()
        .find(|project| same(&project.short_name, name))
    {
        return Ok(project);
    }
    let names: Vec<String> = projects
        .iter()
        .map(|project| project.short_name.clone())
        .collect();
    let mut message = format!("there is no project '{name}'");
    if let Some(close) = closest(name, names.iter().map(String::as_str)) {
        message.push_str(&format!(" (did you mean '{close}'?)"));
    } else if !names.is_empty() {
        message.push_str(&format!("; projects: {}", choices(&names)));
    }
    Err(usage(message))
}

#[derive(Clone, Debug, Deserialize)]
pub struct WorkItemType {
    pub id: String,
    pub name: String,
}

impl Projection for WorkItemType {
    const FIELDS: &'static str = "id,name";
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimeTracking {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub work_item_types: Vec<WorkItemType>,
}

impl Projection for TimeTracking {
    const FIELDS: &'static str = "enabled,workItemTypes(id,name)";
}

pub async fn time_tracking(client: &Client, project: &str) -> Result<TimeTracking> {
    check_short_name(project)?;
    client
        .get(
            &format!("admin/projects/{project}/timeTrackingSettings"),
            &[],
        )
        .await
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::api::test_client;

    #[tokio::test]
    async fn archived_projects_are_left_out() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/admin/projects"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([
                {"shortName": "WEB", "archived": false},
                {"shortName": "OLD", "archived": true}
            ])))
            .mount(&server)
            .await;
        assert_eq!(
            short_names(&test_client(&server.uri())).await.unwrap(),
            ["WEB"]
        );
    }

    fn project(short: &str) -> Project {
        Project {
            short_name: short.into(),
            name: format!("{short} project"),
            leader: None,
            description: None,
            archived: false,
        }
    }

    #[test]
    fn projects_are_found_in_any_case_with_a_suggestion_otherwise() {
        let projects = [project("TEST"), project("APP")];
        assert_eq!(find(&projects, "test").unwrap().short_name, "TEST");
        let error = find(&projects, "TES").unwrap_err().to_string();
        assert!(error.contains("did you mean 'TEST'"), "{error}");
        let error = find(&projects, "zzz").unwrap_err().to_string();
        assert!(error.contains("projects: TEST, APP"), "{error}");
    }

    #[tokio::test]
    async fn time_tracking_settings_are_read_by_short_name() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/admin/projects/TEST/timeTrackingSettings"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "enabled": true,
                "workItemTypes": [{"id": "85-0", "name": "Development"}]
            })))
            .mount(&server)
            .await;
        let settings = time_tracking(&test_client(&server.uri()), "TEST")
            .await
            .unwrap();
        assert!(settings.enabled);
        assert_eq!(settings.work_item_types[0].id, "85-0");
        assert!(
            time_tracking(&test_client(&server.uri()), "../x")
                .await
                .is_err()
        );
    }
}
