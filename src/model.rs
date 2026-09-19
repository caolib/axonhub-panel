//! Wire types mirroring the AxonHub `GetRequests` GraphQL payload.
//!
//! Only the fields the panel actually renders are requested: a narrower
//! selection keeps each poll cheap and removes any chance of a field drifting
//! out of use unnoticed.
//!
//! Field naming caveat: AxonHub uses `modelID`, which is not valid `camelCase`
//! output of a `rename_all` conversion, so it needs an explicit `rename`.
//! Getting this wrong fails silently — the value deserializes to `None`.

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct Edge<T> {
    pub node: Option<T>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Connection<T> {
    /// AxonHub omits `edges` entirely for some nested connections; a
    /// path-based default avoids the `T: Default` bound a unit `default` adds.
    #[serde(default = "no_edges")]
    pub edges: Vec<Edge<T>>,
    pub total_count: Option<i64>,
}

fn no_edges<T>() -> Vec<Edge<T>> {
    Vec::new()
}

impl<T> Connection<T> {
    pub fn first(&self) -> Option<&T> {
        self.edges.iter().find_map(|e| e.node.as_ref())
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct NamedRef {
    pub name: Option<String>,
}

/// The API key name a request arrived with. The owning user is not fetched:
/// the panel identifies the caller by key, which is what the key is for.
impl NamedRef {
    pub fn non_empty_name(&self) -> Option<String> {
        self.name
            .as_deref()
            .map(str::trim)
            .filter(|n| !n.is_empty())
            .map(str::to_string)
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageLog {
    pub prompt_tokens: Option<i64>,
    pub total_tokens: Option<i64>,
    pub prompt_cached_tokens: Option<i64>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Execution {
    #[serde(rename = "modelID")]
    pub model_id: Option<String>,
    pub status: Option<String>,
    pub format: Option<String>,
    pub reasoning_effort: Option<String>,
    pub pass_through_applied: Option<bool>,
    pub channel: Option<NamedRef>,
}

impl Default for Execution {
    fn default() -> Self {
        Execution {
            model_id: None,
            status: None,
            format: None,
            reasoning_effort: None,
            pass_through_applied: None,
            channel: None,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Request {
    pub id: String,
    pub created_at: Option<String>,
    pub status: Option<String>,
    #[serde(rename = "modelID")]
    pub model_id: Option<String>,
    pub format: Option<String>,
    pub reasoning_effort: Option<String>,
    pub stream: Option<bool>,
    #[serde(rename = "apiKey")]
    pub api_key: Option<NamedRef>,
    pub channel: Option<NamedRef>,
    pub metrics_latency_ms: Option<i64>,
    pub metrics_first_token_latency_ms: Option<i64>,
    pub executions: Option<Connection<Execution>>,
    pub usage_logs: Option<Connection<UsageLog>>,
}

impl Default for Request {
    fn default() -> Self {
        Request {
            id: String::new(),
            created_at: None,
            status: None,
            model_id: None,
            format: None,
            reasoning_effort: None,
            stream: None,
            api_key: None,
            channel: None,
            metrics_latency_ms: None,
            metrics_first_token_latency_ms: None,
            executions: None,
            usage_logs: None,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct RequestsData {
    pub requests: Connection<Request>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Envelope<T> {
    pub data: Option<T>,
    #[serde(default)]
    pub errors: Vec<GraphqlError>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GraphqlError {
    pub message: String,
}

/// The channel an execution ran against, with the endpoint it dialled. The
/// list query only needs a name; the detail popup is where `type`/`baseURL`
/// matter, so they are fetched only for it.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelDetail {
    pub name: Option<String>,
    #[serde(rename = "type")]
    pub kind: Option<String>,
    #[serde(rename = "baseURL")]
    pub base_url: Option<String>,
}

/// One upstream attempt of a request. A failed request usually carries several
/// — AxonHub retries the next channel — so the popup lists them all and the
/// error is per execution, not per request.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionDetail {
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    #[serde(rename = "modelID")]
    pub model_id: Option<String>,
    pub format: Option<String>,
    pub reasoning_effort: Option<String>,
    pub pass_through_applied: Option<bool>,
    pub status: Option<String>,
    pub response_status_code: Option<i64>,
    pub error_message: Option<String>,
    /// `requestURL`, not `requestUrl`: the acronym is spelled out in the
    /// schema, and a wrong rename fails silently (see the module header).
    #[serde(rename = "requestURL")]
    pub request_url: Option<String>,
    pub channel: Option<ChannelDetail>,
    pub metrics_first_token_latency_ms: Option<i64>,
    pub metrics_reasoning_duration_ms: Option<i64>,
}

impl ExecutionDetail {
    pub fn status(&self) -> Status {
        Status::parse(self.status.as_deref())
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestWithExecutions {
    pub executions: Option<Connection<ExecutionDetail>>,
}

/// Answer to the `GetRequestExecutions` query: `node(id:)` resolves to the
/// request, whose `executions` connection carries the attempts.
#[derive(Debug, Clone, Deserialize)]
pub struct DetailData {
    pub node: Option<RequestWithExecutions>,
}

/// Collapse a wire format string (`openai/chat_completions`,
/// `anthropic/messages`, …) to the short badge shown on the card.
fn normalize_format(f: &str) -> String {
    if f.contains("chat") {
        "chat".into()
    } else if f.contains("responses") {
        "resp".into()
    } else if f.contains("messages") {
        "mess".into()
    } else {
        f.into()
    }
}

/// A single row, pre-resolved so painting never walks the wire types.
#[derive(Debug, Clone)]
pub struct Row {
    /// Full GUID, e.g. `gid://axonhub/Request/33188`.
    pub id: String,
    pub created_at: Option<String>,
    pub status: Status,
    pub model: String,
    /// Execution model when AxonHub routed to a model other than the request.
    pub routed_model: Option<String>,
    pub channel: Option<String>,
    /// Name of the API key the request arrived with.
    pub caller: Option<String>,
    pub format: Option<String>,
    pub reasoning_effort: Option<String>,
    /// Wire protocol used upstream; compared against `format` to show whether a
    /// conversion happened in flight.
    pub upstream_format: Option<String>,
    pub pass_through: bool,
    pub stream: bool,
    pub latency_ms: Option<i64>,
    pub first_token_ms: Option<i64>,
    pub prompt_tokens: i64,
    pub total_tokens: i64,
    pub cached_tokens: i64,
    /// Number of upstream attempts; above one means retries happened.
    pub attempt_count: i64,
    /// How many of the fetched attempts did not succeed. A request that ended
    /// well can still carry failures here, which is what makes its detail worth
    /// opening.
    pub failed_attempts: i64,
}

impl Row {
    pub fn from_wire(req: &Request) -> Self {
        let execution = req.executions.as_ref().and_then(Connection::first);
        let usage = req.usage_logs.as_ref().and_then(Connection::first);

        let requested_model = req.model_id.clone().unwrap_or_default();
        let routed_model = execution
            .and_then(|e| e.model_id.clone())
            .filter(|m| !m.is_empty() && *m != requested_model);

        // The page prefers the final execution's channel, falling back to the
        // request-level channel before any execution has been recorded.
        let channel = execution
            .and_then(|e| e.channel.as_ref())
            .or(req.channel.as_ref())
            .and_then(|c| c.name.clone())
            .filter(|n| !n.is_empty());

        Row {
            id: req.id.clone(),
            created_at: req.created_at.clone(),
            status: Status::parse(req.status.as_deref()),
            model: requested_model,
            routed_model,
            channel,
            caller: req.api_key.as_ref().and_then(NamedRef::non_empty_name),
            // The inbound protocol is the request's own; the upstream one is
            // what the execution actually spoke.
            format: req
                .format
                .clone()
                .or_else(|| execution.and_then(|e| e.format.clone()))
                .filter(|f| !f.is_empty())
                .map(|f| normalize_format(&f)),
            upstream_format: execution
                .and_then(|e| e.format.clone())
                .filter(|f| !f.is_empty())
                .map(|f| normalize_format(&f)),
            pass_through: execution
                .and_then(|e| e.pass_through_applied)
                .unwrap_or(false),
            reasoning_effort: execution
                .and_then(|e| e.reasoning_effort.clone())
                .or_else(|| req.reasoning_effort.clone())
                .filter(|r| !r.is_empty()),
            stream: req.stream.unwrap_or(false),
            latency_ms: req.metrics_latency_ms,
            first_token_ms: req.metrics_first_token_latency_ms,
            prompt_tokens: usage.and_then(|u| u.prompt_tokens).unwrap_or(0),
            total_tokens: usage.and_then(|u| u.total_tokens).unwrap_or(0),
            cached_tokens: usage.and_then(|u| u.prompt_cached_tokens).unwrap_or(0),
            attempt_count: req
                .executions
                .as_ref()
                .and_then(|c| c.total_count)
                .unwrap_or(0),
            failed_attempts: req
                .executions
                .as_ref()
                .map(|c| {
                    c.edges
                        .iter()
                        .filter_map(|e| e.node.as_ref())
                        .filter(|e| {
                            matches!(
                                Status::parse(e.status.as_deref()),
                                Status::Failed | Status::Canceled
                            )
                        })
                        .count() as i64
                })
                .unwrap_or(0),
        }
    }

    /// Cache hit rate over the prompt, mirroring the requests page rule.
    pub fn cache_hit_rate(&self) -> Option<f64> {
        if self.cached_tokens <= 0 || self.prompt_tokens <= 0 {
            return None;
        }
        // Clamped so a rate above 100% — which would only read as a bug —
        // cannot reach the card.
        Some((self.cached_tokens as f64 / self.prompt_tokens as f64 * 100.0).min(100.0))
    }

    /// The page flags a weak hit rate only for sizeable prompts.
    pub fn cache_hit_is_low(&self) -> bool {
        matches!(self.cache_hit_rate(), Some(r) if r < 80.0 && self.prompt_tokens >= 40_000)
    }

    /// Output throughput: output tokens per second of generation time.
    /// Excludes prompt processing (TTFT) so the number reflects the model's
    /// actual decoding speed, not the prompt length.
    pub fn tps(&self) -> Option<f64> {
        let ms = self.latency_ms?;
        if ms <= 0 {
            return None;
        }
        let output = (self.total_tokens - self.prompt_tokens).max(0);
        if output <= 0 {
            return None;
        }
        let gen_ms = (ms - self.first_token_ms.unwrap_or(0)).max(1);
        Some(output as f64 / (gen_ms as f64 / 1000.0))
    }

    /// The model that actually served the request. When AxonHub routed to a
    /// different model than the client asked for, the served one is what the
    /// card shows; the caller marks it with `is_routed()`.
    pub fn served_model(&self) -> &str {
        match &self.routed_model {
            Some(routed) => routed,
            None if self.model.is_empty() => "未知模型",
            None => &self.model,
        }
    }

    /// Whether the request was served by a model other than the one requested.
    pub fn is_routed(&self) -> bool {
        self.routed_model.is_some()
    }

    /// Whether the request has anything worth explaining, i.e. whether its card
    /// opens the detail popup: it failed or was canceled, or it only succeeded
    /// after an attempt failed. Walking the retries is the point of the popup,
    /// so a request that recovered is as interesting as one that did not.
    pub fn is_error(&self) -> bool {
        matches!(self.status, Status::Failed | Status::Canceled) || self.failed_attempts > 0
    }

    /// The served model with its reasoning effort, e.g. `glm-5.2(max)`. When the
    /// served model differs from the requested one, it is wrapped in [] instead
    /// of recoloured so the distinction is unambiguous.
    pub fn display_model(&self) -> String {
        let model = self.served_model();
        match (&self.reasoning_effort, self.is_routed()) {
            (Some(effort), true) => format!("[{}({})]", model, effort),
            (Some(effort), false) => format!("{}({})", model, effort),
            (None, true) => format!("[{}]", model),
            (None, false) => model.to_string(),
        }
    }

    /// How the inbound and upstream protocols relate, for the card's protocol
    /// cell. A missing side means "unknown", never "same".
    pub fn protocol(&self) -> Protocol {
        match (self.format.as_deref(), self.upstream_format.as_deref()) {
            (Some(i), Some(u)) if i != u => Protocol::Converted {
                from: i.to_string(),
                to: u.to_string(),
            },
            (Some(i), Some(_)) => Protocol::Same(i.to_string()),
            (Some(one), None) | (None, Some(one)) => Protocol::Single(one.to_string()),
            (None, None) => Protocol::Unknown,
        }
    }

    /// Sort key for the merged list: the request's start time in Unix seconds.
    /// Rows without a parsable timestamp sort last, since their age is unknown.
    pub fn age_key(&self) -> i64 {
        self.created_at
            .as_deref()
            .and_then(crate::time::parse_unix)
            .unwrap_or(i64::MIN)
    }
}

/// Deep link to a request's detail page.
///
/// The route matches the whole GUID as a single path segment, so it must be
/// percent-encoded:
///
/// ```text
/// /project/requests/gid%3A%2F%2Faxonhub%2FRequest%2F34009
/// ```
///
/// Passing the bare number instead lands on the router's not-found page, which
/// reports "guid must start with gid://axonhub/".
pub fn request_url(endpoint: &str, id: &str) -> String {
    format!(
        "{}/project/requests/{}",
        endpoint.trim_end_matches('/'),
        encode_path_segment(id)
    )
}

/// Percent-encode every byte outside the RFC 3986 unreserved set, so a GUID
/// becomes a valid single path segment (`:` and `/` are the ones that matter).
fn encode_path_segment(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 16);
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(byte as char);
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_the_encoded_detail_url_the_router_expects() {
        let url = request_url("http://localhost:8090", "gid://axonhub/Request/34009");
        assert_eq!(
            url,
            "http://localhost:8090/project/requests/gid%3A%2F%2Faxonhub%2FRequest%2F34009"
        );
    }

    #[test]
    fn tolerates_a_trailing_slash_on_the_endpoint() {
        assert_eq!(
            request_url("http://localhost:8090/", "gid://axonhub/Request/1"),
            "http://localhost:8090/project/requests/gid%3A%2F%2Faxonhub%2FRequest%2F1"
        );
    }

    fn row_with(requested: &str, served: Option<&str>) -> Row {
        Row {
            id: format!("gid://axonhub/Request/1"),
            created_at: None,
            status: Status::Completed,
            model: requested.to_string(),
            routed_model: served.map(str::to_string),
            channel: None,
            caller: None,
            format: None,
            reasoning_effort: None,
            upstream_format: None,
            pass_through: false,
            stream: false,
            latency_ms: None,
            first_token_ms: None,
            prompt_tokens: 0,
            total_tokens: 0,
            cached_tokens: 0,
            attempt_count: 0,
            failed_attempts: 0,
        }
    }

    #[test]
    fn protocol_distinguishes_conversion_from_match() {
        let mut a = row_with("m", None);
        a.format = Some("anthropic/messages".into());
        a.upstream_format = Some("openai/chat_completions".into());
        assert!(matches!(a.protocol(), Protocol::Converted { .. }));
        assert!(a.protocol().is_converted());

        let mut b = row_with("m", None);
        b.format = Some("anthropic/messages".into());
        b.upstream_format = Some("anthropic/messages".into());
        assert!(!b.protocol().is_converted());
    }

    #[test]
    fn protocol_never_claims_a_match_when_one_side_is_unknown() {
        // A missing upstream format must not read as "no conversion happened".
        let mut a = row_with("m", None);
        a.format = Some("anthropic/messages".into());
        assert!(matches!(a.protocol(), Protocol::Single(_)));
        assert!(!a.protocol().is_converted());

        let b = row_with("m", None);
        assert_eq!(b.protocol(), Protocol::Unknown);
    }

    #[test]
    fn caller_is_the_key_name_only() {
        let named = |n: Option<&str>| NamedRef {
            name: n.map(str::to_string),
        };
        assert_eq!(named(Some("cc")).non_empty_name().as_deref(), Some("cc"));
        assert_eq!(
            named(Some("  cc  ")).non_empty_name().as_deref(),
            Some("cc")
        );
        // A blank or absent name yields nothing rather than an empty cell.
        assert_eq!(named(Some("   ")).non_empty_name(), None);
        assert_eq!(named(None).non_empty_name(), None);
    }

    #[test]
    fn reports_the_served_model_when_routed() {
        let routed = row_with("claude-fable-5-1", Some("claude-opus-5"));
        assert_eq!(routed.served_model(), "claude-opus-5");
        assert!(routed.is_routed());
    }

    #[test]
    fn reports_the_requested_model_when_not_routed() {
        let plain = row_with("glm-5.2", None);
        assert_eq!(plain.served_model(), "glm-5.2");
        assert!(!plain.is_routed());
    }

    fn wire_request(json: &str) -> Request {
        serde_json::from_str(json).expect("request parses")
    }

    #[test]
    fn a_request_that_recovered_after_a_failure_is_still_trouble() {
        // The `GetRequests` shape: the final attempt succeeded, an earlier one
        // did not. The retry is what the detail popup exists to show.
        let req = wire_request(
            r#"{
              "id": "gid://axonhub/Request/1",
              "status": "completed",
              "modelID": "glm-5.2",
              "executions": { "totalCount": 2, "edges": [
                { "node": { "status": "completed", "modelID": "glm-5.2" } },
                { "node": { "status": "failed", "modelID": "glm-5.2" } }
              ] }
            }"#,
        );
        let row = Row::from_wire(&req);
        assert_eq!(row.status, Status::Completed);
        assert_eq!(row.attempt_count, 2);
        assert_eq!(row.failed_attempts, 1);
        assert!(row.is_error(), "a failed attempt makes the card clickable");
    }

    #[test]
    fn a_request_without_failed_attempts_has_nothing_to_open() {
        let req = wire_request(
            r#"{
              "id": "gid://axonhub/Request/2",
              "status": "completed",
              "modelID": "glm-5.2",
              "executions": { "totalCount": 1, "edges": [
                { "node": { "status": "completed", "modelID": "glm-5.2" } }
              ] }
            }"#,
        );
        let row = Row::from_wire(&req);
        assert_eq!(row.failed_attempts, 0);
        assert!(!row.is_error());

        // An execution without a status must not be read as a failure.
        let req = wire_request(
            r#"{
              "id": "gid://axonhub/Request/3",
              "status": "completed",
              "modelID": "glm-5.2",
              "executions": { "totalCount": 1, "edges": [ { "node": {} } ] }
            }"#,
        );
        let row = Row::from_wire(&req);
        assert_eq!(row.failed_attempts, 0);
        assert!(!row.is_error());
    }
}

/// Relationship between the protocol the client spoke and the one used upstream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Protocol {
    /// Both known and identical; conversion did not happen.
    Same(String),
    /// Both known and different; AxonHub converted in flight.
    Converted {
        from: String,
        to: String,
    },
    /// Only one side is known.
    Single(String),
    Unknown,
}

impl Protocol {
    pub fn is_converted(&self) -> bool {
        matches!(self, Protocol::Converted { .. })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Completed,
    Failed,
    Processing,
    Canceled,
    Pending,
}

impl Status {
    pub fn parse(raw: Option<&str>) -> Self {
        match raw.unwrap_or("") {
            "completed" => Status::Completed,
            "failed" => Status::Failed,
            "processing" => Status::Processing,
            "canceled" => Status::Canceled,
            _ => Status::Pending,
        }
    }

    /// Label for the detail popup, which names the state rather than encoding
    /// it as a colour the way the cards do.
    pub fn label(self) -> &'static str {
        match self {
            Status::Completed => "成功",
            Status::Failed => "失败",
            Status::Processing => "进行中",
            Status::Canceled => "已取消",
            Status::Pending => "等待中",
        }
    }

    /// Whether the row is still in flight; drives the faster poll cadence.
    pub fn is_active(self) -> bool {
        matches!(self, Status::Processing | Status::Pending)
    }
}

/// Which request states the list shows. The header chips mirror this enum:
/// one chip per filter plus 全部 for the empty mask. Chips toggle
/// independently, so any combination of states can be selected at once.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Filter {
    #[default]
    All,
    Completed,
    Failed,
    Active,
}

/// Bitmask of selected `Filter`s; the empty mask shows everything (`All`).
pub type FilterMask = u8;

pub const FILTER_NONE: FilterMask = 0;
pub const FILTER_COMPLETED: FilterMask = 1 << 0;
pub const FILTER_FAILED: FilterMask = 1 << 1;
pub const FILTER_ACTIVE: FilterMask = 1 << 2;

impl Filter {
    /// Header chip labels.
    pub fn label(self) -> &'static str {
        match self {
            Filter::All => "全部",
            Filter::Completed => "成功",
            Filter::Failed => "失败",
            Filter::Active => "进行",
        }
    }

    /// The single bit this chip toggles; `All` maps to the empty mask.
    pub fn mask(self) -> FilterMask {
        match self {
            Filter::All => FILTER_NONE,
            Filter::Completed => FILTER_COMPLETED,
            Filter::Failed => FILTER_FAILED,
            Filter::Active => FILTER_ACTIVE,
        }
    }
}

/// Whether a row with `status` passes the selected mask. The empty mask shows
/// everything; otherwise only the chosen states are kept (进行 covers both
/// Processing and Pending, mirroring `Status::is_active`).
pub fn mask_matches(mask: FilterMask, status: Status) -> bool {
    if mask == FILTER_NONE {
        return true;
    }
    (status == Status::Completed && mask & FILTER_COMPLETED != 0)
        || (status == Status::Failed && mask & FILTER_FAILED != 0)
        || (status.is_active() && mask & FILTER_ACTIVE != 0)
}
