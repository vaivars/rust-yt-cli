use std::collections::BTreeMap;

use serde::Serialize;
use yt::api::issues::IssueDetail;

use super::fields_json;

/// An issue as yt prints it in JSON: the `issue show` shape, also used by writes.
#[derive(Serialize)]
pub(crate) struct IssueJson<'a> {
    id: &'a str,
    summary: Option<&'a str>,
    project: &'a str,
    reporter: Option<&'a str>,
    created: String,
    updated: String,
    resolved: Option<String>,
    tags: Vec<&'a str>,
    fields: BTreeMap<&'a str, serde_json::Value>,
    description: Option<&'a str>,
}

pub(crate) fn issue_json(issue: &IssueDetail) -> IssueJson<'_> {
    IssueJson {
        id: &issue.id_readable,
        summary: issue.summary.as_deref(),
        project: &issue.project.short_name,
        reporter: issue
            .reporter
            .as_ref()
            .map(|reporter| reporter.login.as_str()),
        created: yt::time::utc(issue.created),
        updated: yt::time::utc(issue.updated),
        resolved: issue.resolved.map(yt::time::utc),
        tags: issue.tags.iter().map(|tag| tag.name.as_str()).collect(),
        fields: fields_json(&issue.custom_fields),
        description: issue.description.as_deref(),
    }
}
