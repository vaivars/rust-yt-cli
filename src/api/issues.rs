use futures::Stream;
use serde::Deserialize;
use serde_json::Value;

use crate::api::custom_fields::IssueCustomField;
use crate::api::{Client, Projection};
use crate::error::{Result, YtError};

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectRef {
    pub short_name: String,
    pub name: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserRef {
    pub login: String,
    pub full_name: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct TagRef {
    pub name: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueDetail {
    pub id_readable: String,
    pub summary: Option<String>,
    pub description: Option<String>,
    pub project: ProjectRef,
    pub reporter: Option<UserRef>,
    pub created: i64,
    pub updated: i64,
    pub resolved: Option<i64>,
    #[serde(default)]
    pub tags: Vec<TagRef>,
    #[serde(default)]
    pub custom_fields: Vec<IssueCustomField>,
}

impl Projection for IssueDetail {
    const FIELDS: &'static str = "idReadable,summary,description,project(shortName,name),reporter(login,fullName),created,updated,resolved,tags(name),customFields(name,$type,projectCustomField(field(fieldType(id))),value(name,login,fullName,isResolved,minutes,presentation,text,$type))";
}

pub async fn get(client: &Client, id: &str) -> Result<IssueDetail> {
    client.get(&format!("issues/{id}"), &[]).await
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueRow {
    pub id_readable: String,
    pub summary: Option<String>,
    pub project: ProjectRef,
    pub created: i64,
    pub updated: i64,
    pub resolved: Option<i64>,
    #[serde(default)]
    pub custom_fields: Vec<IssueCustomField>,
}

impl Projection for IssueRow {
    const FIELDS: &'static str = "idReadable,summary,project(shortName,name),created,updated,resolved,customFields(name,$type,projectCustomField(field(fieldType(id))),value(name,login,fullName,isResolved,minutes,presentation,text,$type))";
}

impl IssueRow {
    /// The value of the issue's state field: the one whose values know whether they are resolved.
    pub fn state(&self) -> Option<String> {
        crate::api::custom_fields::state_of(&self.custom_fields)
    }
}

pub fn search<'a>(
    client: &'a Client,
    query: &str,
    page_size: usize,
) -> impl Stream<Item = Result<IssueRow>> + Send + 'a {
    let mut parameters = Vec::new();
    if !query.is_empty() {
        parameters.push(("query".to_string(), query.to_string()));
    }
    client.paginate("issues".to_string(), parameters, page_size)
}

/// Joins convenience clauses and the user's query by juxtaposition; `stable_sort` adds an order that paging cannot skip or repeat under.
pub fn build_query(
    user_query: Option<&str>,
    project: Option<&str>,
    stable_sort: bool,
) -> Result<String> {
    let mut parts = Vec::new();
    if let Some(project) = project {
        if project.contains('}') {
            return Err(YtError::Usage {
                message: format!("'{project}' cannot be used in a query"),
            }
            .into());
        }
        parts.push(format!("project: {{{project}}}"));
    }
    if let Some(query) = user_query.map(str::trim).filter(|query| !query.is_empty()) {
        if parts.is_empty() {
            parts.push(query.to_string());
        } else {
            // `or` binds looser than `and`, so the flag clauses must scope the whole query. A group
            // needs an explicit `and` (juxtaposition only joins plain terms), and `sort by` stays outside it.
            let lower = query.to_ascii_lowercase();
            let sort = ["sort by", "order by"]
                .iter()
                .filter_map(|keyword| lower.find(keyword))
                .min()
                .unwrap_or(query.len());
            let (filter, order) = query.split_at(sort);
            if !filter.trim().is_empty() {
                parts.push(format!("and ({})", filter.trim()));
            }
            if !order.is_empty() {
                parts.push(order.to_string());
            }
        }
    }
    let mut query = parts.join(" ");
    let lower = query.to_ascii_lowercase();
    if stable_sort && !lower.contains("sort by") && !lower.contains("order by") {
        if !query.is_empty() {
            query.push(' ');
        }
        query.push_str("sort by: created asc");
    }
    Ok(query)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Created {
    id_readable: String,
}

impl Projection for Created {
    const FIELDS: &'static str = "idReadable";
}

/// Creates an issue from a REST body and returns its readable ID.
pub async fn create(client: &Client, body: &Value) -> Result<String> {
    let created: Created = client.post("issues", body).await?;
    Ok(created.id_readable)
}

pub async fn update(client: &Client, id: &str, body: &Value) -> Result<()> {
    client.post_unit(&format!("issues/{id}"), body).await
}

pub async fn delete(client: &Client, id: &str) -> Result<()> {
    client.delete(&format!("issues/{id}")).await
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct InProject {
    project: ProjectRef,
}

impl Projection for InProject {
    const FIELDS: &'static str = "project(shortName,name)";
}

/// The project an issue is in now; a moved issue keeps answering to its old ID.
pub async fn project(client: &Client, id: &str) -> Result<String> {
    let found: InProject = client.get(&format!("issues/{id}"), &[]).await?;
    Ok(found.project.short_name)
}

/// The project short name of a readable ID (`WEB-42` → `WEB`).
pub fn project_of(id: &str) -> &str {
    id.rsplit_once('-').map_or(id, |(project, _)| project)
}

/// Readable IDs grouped by project, in order of first appearance.
pub fn by_project(ids: &[String]) -> Vec<(String, Vec<String>)> {
    let mut groups: Vec<(String, Vec<String>)> = Vec::new();
    for id in ids {
        let project = project_of(id);
        match groups.iter_mut().find(|(name, _)| name == project) {
            Some((_, members)) => members.push(id.clone()),
            None => groups.push((project.to_string(), vec![id.clone()])),
        }
    }
    groups
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use wiremock::matchers::{body_json, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::api::projection::check_selector;
    use crate::api::test_client;

    #[test]
    fn selectors_are_valid_and_include_the_custom_field_selector() {
        assert_eq!(check_selector(IssueDetail::FIELDS), Ok(()));
        assert!(IssueDetail::FIELDS.contains(crate::api::custom_fields::FIELDS));
    }

    #[test]
    fn the_show_fixture_decodes() {
        let text = include_str!("../../tests/fixtures/issue-show.json");
        let issue: IssueDetail = serde_json::from_str(text).unwrap();
        assert_eq!(issue.id_readable, "WEB-42");
        assert_eq!(issue.custom_fields.len(), 4);
    }

    #[test]
    fn queries_are_built_from_flags_and_words() {
        assert_eq!(build_query(None, None, false).unwrap(), "");
        assert_eq!(
            build_query(Some(" for: me "), None, false).unwrap(),
            "for: me"
        );
        assert_eq!(
            build_query(Some("#Unresolved"), Some("Web Site"), false).unwrap(),
            "project: {Web Site} and (#Unresolved)"
        );
        assert_eq!(
            build_query(None, Some("WEB"), true).unwrap(),
            "project: {WEB} sort by: created asc"
        );
        assert_eq!(
            build_query(Some("sort by: updated desc"), None, true).unwrap(),
            "sort by: updated desc"
        );
        assert!(build_query(None, Some("A}B"), false).is_err());
    }

    #[test]
    fn the_list_fixture_decodes_and_finds_the_state() {
        let rows: Vec<IssueRow> =
            serde_json::from_str(include_str!("../../tests/fixtures/issue-list.json")).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].state().as_deref(), Some("Open"));
        assert_eq!(rows[1].state(), None);
        assert_eq!(check_selector(IssueRow::FIELDS), Ok(()));
    }

    #[test]
    fn the_project_scopes_the_whole_query() {
        assert_eq!(
            build_query(Some("for: me or #Critical"), Some("WEB"), false).unwrap(),
            "project: {WEB} and (for: me or #Critical)"
        );
        assert_eq!(
            build_query(Some("#Unresolved Sort By: updated desc"), Some("WEB"), true).unwrap(),
            "project: {WEB} and (#Unresolved) Sort By: updated desc"
        );
        assert_eq!(
            build_query(Some("sort by: votes"), Some("WEB"), false).unwrap(),
            "project: {WEB} sort by: votes"
        );
        assert_eq!(
            build_query(Some("for: me or #Critical"), None, false).unwrap(),
            "for: me or #Critical"
        );
    }

    #[tokio::test]
    async fn create_update_and_delete_use_the_issue_endpoints() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/issues"))
            .and(body_json(json!({"summary": "x"})))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"idReadable": "WEB-43"})))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/api/issues/WEB-43"))
            .and(body_json(json!({"summary": "y"})))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("DELETE"))
            .and(path("/api/issues/WEB-43"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;
        let client = test_client(&server.uri());
        let id = create(&client, &json!({"summary": "x"})).await.unwrap();
        assert_eq!(id, "WEB-43");
        update(&client, &id, &json!({"summary": "y"}))
            .await
            .unwrap();
        delete(&client, &id).await.unwrap();
    }

    #[test]
    fn issues_group_by_project_in_order() {
        let ids: Vec<String> = ["WEB-1", "OPS-2", "WEB-3", "MY_APP-4"]
            .map(String::from)
            .to_vec();
        assert_eq!(project_of("MY_APP-4"), "MY_APP");
        assert_eq!(
            by_project(&ids),
            vec![
                (
                    "WEB".to_string(),
                    vec!["WEB-1".to_string(), "WEB-3".to_string()]
                ),
                ("OPS".to_string(), vec!["OPS-2".to_string()]),
                ("MY_APP".to_string(), vec!["MY_APP-4".to_string()]),
            ]
        );
    }
}
