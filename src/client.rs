//! HTTP access to AxonHub: sign-in and the requests query.

use std::time::Duration;

use serde::Deserialize;
use serde_json::json;

use crate::model::{Envelope, Request};

/// Mirrors the requests page field selection. `modelID` and `apiKey` must stay
/// byte-exact: the leading-lowercase acronyms are not valid `camelCase`, so the
/// server rejects any other spelling.
const REQUESTS_QUERY: &str = r#"
query GetRequests($first: Int, $where: RequestWhereInput, $orderBy: RequestOrder) {
  requests(first: $first, where: $where, orderBy: $orderBy) {
    edges {
      node {
        id
        createdAt
        status
        modelID
        format
        reasoningEffort
        stream
        apiKey { name }
        channel { name }
        metricsLatencyMs
        metricsFirstTokenLatencyMs
        executions(first: 10, orderBy: { field: CREATED_AT, direction: DESC }) {
          edges { node { modelID status format reasoningEffort passThroughApplied channel { name } } }
          totalCount
        }
        usageLogs(first: 1) {
          edges {
            node {
              promptTokens
              completionTokens
              completionReasoningTokens
              totalTokens
              promptCachedTokens
              promptWriteCachedTokens
              totalCost
            }
          }
        }
      }
    }
    totalCount
  }
}
"#;

#[derive(Debug, Deserialize)]
pub struct SignInResponse {
    pub token: String,
    pub user: UserInfo,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserInfo {
    pub email: Option<String>,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    /// Per-project memberships; used to validate the configured project.
    #[serde(default)]
    pub projects: Vec<UserProject>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserProject {
    pub project_id: Option<String>,
}

impl UserInfo {
    pub fn display_name(&self) -> String {
        let full = format!(
            "{} {}",
            self.first_name.as_deref().unwrap_or("").trim(),
            self.last_name.as_deref().unwrap_or("").trim()
        );
        let full = full.trim();
        if !full.is_empty() {
            return full.to_string();
        }
        self.email.clone().unwrap_or_else(|| "未知用户".into())
    }
}

/// Every failure the panel can show the user, with the message already
/// localized for the status strip.
#[derive(Debug, Clone)]
pub enum ApiError {
    Unreachable(String),
    Rejected(String),
    Unauthorized,
    Expired,
    Malformed(String),
}

impl ApiError {
    pub fn message(&self) -> String {
        match self {
            ApiError::Unreachable(d) => format!("无法连接 AxonHub: {d}"),
            ApiError::Rejected(m) => format!("请求被拒绝: {m}"),
            ApiError::Unauthorized => "凭据无效,请重新登录".into(),
            ApiError::Expired => "登录已过期".into(),
            ApiError::Malformed(m) => format!("响应无法解析: {m}"),
        }
    }

    /// Credentials are gone or wrong; the caller must prompt for sign-in again.
    pub fn needs_signin(&self) -> bool {
        matches!(self, ApiError::Unauthorized | ApiError::Expired)
    }
}

pub struct Client {
    agent: ureq::Agent,
}

impl Client {
    pub fn new(timeout: Duration) -> Self {
        let config = ureq::Agent::config_builder()
            .timeout_global(Some(timeout))
            .build();
        Client {
            agent: config.new_agent(),
        }
    }

    pub fn sign_in(&self, url: &str, email: &str, password: &str) -> Result<SignInResponse, ApiError> {
        let body = json!({ "email": email, "password": password });
        let response = self
            .agent
            .post(url)
            .header("Content-Type", "application/json")
            .send_json(&body)
            .map_err(|e| map_transport(e, false))?;

        let status = response.status();
        let text = response
            .into_body()
            .read_to_string()
            .map_err(|e| ApiError::Unreachable(e.to_string()))?;

        if !status.is_success() {
            let message = extract_error(&text).unwrap_or_else(|| format!("HTTP {status}"));
            return Err(if status.as_u16() == 401 {
                ApiError::Unauthorized
            } else {
                ApiError::Rejected(message)
            });
        }

        serde_json::from_str(&text).map_err(|e| ApiError::Malformed(e.to_string()))
    }

    pub fn fetch_requests(
        &self,
        url: &str,
        token: &str,
        project_id: &str,
        first: i64,
    ) -> Result<(Vec<Request>, i64), ApiError> {
        let body = json!({
            "query": REQUESTS_QUERY,
            "operationName": "GetRequests",
            "variables": {
                "first": first,
                "where": { "projectID": project_id },
                "orderBy": { "field": "CREATED_AT", "direction": "DESC" }
            }
        });

        let response = self
            .agent
            .post(url)
            .header("Content-Type", "application/json")
            .header("Authorization", &format!("Bearer {token}"))
            .header("X-Project-ID", project_id)
            .send_json(&body)
            .map_err(|e| map_transport(e, true))?;

        let status = response.status();
        let text = response
            .into_body()
            .read_to_string()
            .map_err(|e| ApiError::Unreachable(e.to_string()))?;

        if status.as_u16() == 401 {
            return Err(ApiError::Expired);
        }
        if !status.is_success() {
            let message = extract_error(&text).unwrap_or_else(|| format!("HTTP {status}"));
            return Err(ApiError::Rejected(message));
        }

        let envelope: Envelope =
            serde_json::from_str(&text).map_err(|e| ApiError::Malformed(e.to_string()))?;

        if let Some(first) = envelope.errors.first() {
            // GraphQL surfaces auth problems inside the payload, not the status.
            let lower = first.message.to_lowercase();
            if lower.contains("unauthorized") || lower.contains("invalid token") {
                return Err(ApiError::Expired);
            }
            return Err(ApiError::Rejected(first.message.clone()));
        }

        let Some(data) = envelope.data else {
            return Err(ApiError::Malformed("响应缺少 data 字段".into()));
        };

        let total = data.requests.total_count.unwrap_or(0);
        Ok((data.requests.edges.into_iter().filter_map(|e| e.node).collect(), total))
    }
}

/// Resolve a project id, falling back to the user's first membership so a
/// default install works without hand-editing the config.
pub fn resolve_project(user: &UserInfo, configured: &str) -> String {
    if !configured.trim().is_empty() {
        return configured.trim().to_string();
    }
    user.projects
        .iter()
        .filter_map(|p| p.project_id.clone())
        .next()
        .unwrap_or_default()
}

fn extract_error(body: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(body).ok()?;
    value
        .get("error")
        .and_then(|e| e.get("message"))
        .and_then(|m| m.as_str())
        .or_else(|| value.get("errors")?.get(0)?.get("message")?.as_str())
        .map(str::to_string)
}

/// `auth` says whether a 401/403 from the transport should be treated as an
/// expired token rather than a hard failure.
fn map_transport(err: ureq::Error, auth: bool) -> ApiError {
    match err {
        ureq::Error::StatusCode(code) if auth && code == 401 => ApiError::Expired,
        ureq::Error::StatusCode(code) => ApiError::Rejected(format!("HTTP {code}")),
        ureq::Error::Timeout(_) => ApiError::Unreachable("连接超时".into()),
        other => ApiError::Unreachable(other.to_string()),
    }
}

/// Convenience for turning a wire response into paint-ready rows.
pub fn rows_from(requests: &[Request]) -> Vec<crate::model::Row> {
    requests.iter().map(crate::model::Row::from_wire).collect()
}
