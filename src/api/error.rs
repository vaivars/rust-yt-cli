use reqwest::{Method, StatusCode};
use serde::Deserialize;

use crate::error::YtError;

#[derive(Default, Deserialize)]
struct ErrorBody {
    error: Option<String>,
    error_description: Option<String>,
    error_field: Option<String>,
    #[serde(default)]
    error_children: Vec<ErrorChild>,
    error_rule_name: Option<String>,
}

#[derive(Deserialize)]
struct ErrorChild {
    error: Option<String>,
}

pub fn api_error(method: &Method, path: &str, status: StatusCode, body: &str) -> miette::Report {
    let (code, message) = match serde_json::from_str::<ErrorBody>(body) {
        Ok(parsed) if parsed.error.is_some() || parsed.error_description.is_some() => {
            (parsed.error.clone(), describe(&parsed, status))
        }
        _ => (None, unexpected(status, body)),
    };
    let help = match status.as_u16() {
        403 => Some(
            "your account or token may lack permission; tokens need the YouTrack scope".to_string(),
        ),
        404 => Some("check the ID; you may also lack permission to see it".to_string()),
        _ => None,
    };
    YtError::Api {
        method: method.to_string(),
        path: path.to_string(),
        status: status.as_u16(),
        code,
        message,
        help,
    }
    .into()
}

/// A readable description of a body that is not YouTrack's JSON error.
pub fn unexpected(status: StatusCode, body: &str) -> String {
    let trimmed = body.trim_start();
    if trimmed.starts_with('<') {
        return format!(
            "unexpected HTML response (HTTP {status}); a proxy or sign-in page may be in the way"
        );
    }
    let excerpt = crate::redact::redact(&trimmed.chars().take(200).collect::<String>());
    if excerpt.is_empty() {
        format!("HTTP {status} with an empty body")
    } else {
        format!("HTTP {status}: {excerpt}")
    }
}

fn describe(body: &ErrorBody, status: StatusCode) -> String {
    let mut message = body
        .error_description
        .clone()
        .filter(|description| !description.is_empty())
        .or_else(|| body.error.clone())
        .unwrap_or_else(|| format!("HTTP {status}"));
    let children: Vec<&str> = body
        .error_children
        .iter()
        .filter_map(|child| child.error.as_deref())
        .filter(|error| !error.is_empty())
        .collect();
    if !children.is_empty() {
        message = format!("{message}: {}", children.join("; "));
    }
    if let Some(field) = &body.error_field {
        message = format!("{message} (field: {field})");
    }
    if let Some(rule) = &body.error_rule_name {
        message = format!("{message} (workflow rule: {rule})");
    }
    message
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_error_code_is_kept() {
        let report = api_error(
            &Method::GET,
            "/api/extensionEndpoints/app/global/issue",
            StatusCode::FORBIDDEN,
            r#"{"error":"disabled","error_description":"switched off"}"#,
        );
        let Some(YtError::Api { code, message, .. }) = report.downcast_ref::<YtError>() else {
            panic!("{report:?}");
        };
        assert_eq!(code.as_deref(), Some("disabled"));
        assert_eq!(message, "switched off");
        let report = api_error(&Method::GET, "/api/x", StatusCode::BAD_GATEWAY, "<html>");
        assert!(matches!(
            report.downcast_ref::<YtError>(),
            Some(YtError::Api { code: None, .. })
        ));
    }

    #[test]
    fn an_echoed_token_is_redacted_from_the_excerpt() {
        crate::redact::register("perm-ZWNobw==.dGVzdA==.echoed123456");
        let text = unexpected(
            StatusCode::BAD_REQUEST,
            "rejected: Authorization: Bearer perm-ZWNobw==.dGVzdA==.echoed123456",
        );
        assert!(!text.contains("echoed123456"), "{text}");
    }
}
