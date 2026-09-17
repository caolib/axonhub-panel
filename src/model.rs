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
    pub format: Option<String>,
    pub reasoning_effort: Option<String>,
    pub pass_through_applied: Option<bool>,
    pub channel: Option<NamedRef>,
}

impl Default for Execution {
    fn default() -> Self {
        Execution {
            model_id: None,
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
pub struct Envelope {
    pub data: Option<RequestsData>,
    #[serde(default)]
    pub errors: Vec<GraphqlError>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GraphqlError {
    pub message: String,
}

/// One Octopus request as sent on `/api/v1/log/overview/stream`.
///
/// Field names are the wire's own snake_case; the stream is served by Go, not
/// by a GraphQL schema, so no renaming applies. `duration` is nanoseconds.
#[derive(Debug, Clone, Deserialize)]
pub struct OctopusRecord {
    pub id: u64,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub started_at: Option<String>,
    #[serde(default)]
    pub duration: Option<i64>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub protocol: Option<u32>,
    #[serde(default)]
    pub api_key_name: Option<String>,
    #[serde(default)]
    pub usage: Option<OctopusUsage>,
    #[serde(default)]
    pub round: Option<i64>,
    #[serde(default)]
    pub target_channel: Option<String>,
    #[serde(default)]
    pub target_model: Option<String>,
    #[serde(default)]
    pub target_protocol: Option<u32>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OctopusUsage {
    #[serde(default)]
    pub prompt_tokens: i64,
    #[serde(default)]
    pub total_tokens: i64,
    #[serde(default)]
    pub prompt_tokens_details: Option<OctopusPromptDetails>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OctopusPromptDetails {
    #[serde(default)]
    pub cached_tokens: i64,
}

/// Wire protocol bit set by `model.Protocol` on the Octopus side:
/// `OpenAIChatCompletion = 1 << 1`, `OpenAIResponse = 1 << 2`,
/// `AnthropicMessage = 1 << 3`. Zero (and any unknown bit) is "not chosen".
fn protocol_name(bits: Option<u32>) -> Option<String> {
    let name = match bits? {
        2 => "chat",
        4 => "resp",
        8 => "mess",
        _ => return None,
    };
    Some(name.to_string())
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

/// Which gateway a row came from. The panel can show both at once, so every
/// row carries its origin for the badge and for opening the right page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    AxonHub,
    Octopus,
}

impl Source {
    /// Short tag shown on cards when the list mixes sources.
    pub fn badge(self) -> &'static str {
        match self {
            Source::AxonHub => "AH",
            Source::Octopus => "OCT",
        }
    }
}

/// A single row, pre-resolved so painting never walks the wire types.
#[derive(Debug, Clone)]
pub struct Row {
    /// Full GUID, e.g. `gid://axonhub/Request/33188` (AxonHub) or the stream's
    /// numeric id as text (Octopus, which has no deep link).
    pub id: String,
    pub source: Source,
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
            source: Source::AxonHub,
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
        }
    }

    /// Build a row from one Octopus request-overview record.
    ///
    /// Octopus exposes less than AxonHub: no first-token latency, no
    /// reasoning effort and no pass-through flag, so those stay empty; the
    /// protocol is a small integer instead of a name.
    pub fn from_octopus(rec: &OctopusRecord) -> Self {
        let requested_model = rec.model.clone().unwrap_or_default();
        let routed_model = rec
            .target_model
            .clone()
            .filter(|m| !m.is_empty() && *m != requested_model);
        let usage = rec.usage.as_ref();

        Row {
            id: rec.id.to_string(),
            source: Source::Octopus,
            created_at: rec.started_at.clone(),
            status: Status::parse_octopus(rec.status.as_deref()),
            model: requested_model,
            routed_model,
            channel: rec.target_channel.clone().filter(|c| !c.is_empty()),
            caller: rec.api_key_name.clone().filter(|k| !k.is_empty()),
            // `target_protocol` is only meaningful once a round has started;
            // a zero bit means "not chosen yet", not "unknown protocol".
            format: protocol_name(rec.protocol),
            reasoning_effort: None,
            upstream_format: protocol_name(rec.target_protocol),
            pass_through: false,
            stream: false,
            // Go marshals `time.Duration` as nanoseconds.
            latency_ms: rec.duration.map(|ns| ns / 1_000_000),
            first_token_ms: None,
            prompt_tokens: usage.map_or(0, |u| u.prompt_tokens),
            total_tokens: usage.map_or(0, |u| u.total_tokens),
            cached_tokens: usage
                .and_then(|u| u.prompt_tokens_details.as_ref())
                .map_or(0, |d| d.cached_tokens),
            // The round counter is the retry count made visible.
            attempt_count: rec.round.unwrap_or(0),
        }
    }

    /// Cache hit rate over the prompt, mirroring the requests page rule.
    pub fn cache_hit_rate(&self) -> Option<f64> {
        if self.cached_tokens <= 0 || self.prompt_tokens <= 0 {
            return None;
        }
        // Octopus can report cached tokens beyond the prompt when rounds are
        // summed; a rate above 100% would only read as a bug.
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

/// Octopus's web UI has no per-request routes, so a row cannot be deep-linked;
/// the best target is the dashboard the user would navigate from.
pub fn octopus_url(endpoint: &str) -> String {
    let base = endpoint.trim_end_matches('/');
    if base.is_empty() {
        String::new()
    } else {
        format!("{base}/")
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
            source: Source::AxonHub,
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
        }
    }

    /// The exact shape observed on `/api/v1/log/overview/stream`.
    const OCTOPUS_SAMPLE: &str = r#"{
        "id": 75,
        "status": "success",
        "started_at": "2026-09-13T15:22:16.8747504+08:00",
        "duration": 23908870800,
        "model": "glm-5.3-flash",
        "protocol": 2,
        "group_id": 1,
        "api_key_name": "omp",
        "usage": {
            "prompt_tokens": 63280,
            "completion_tokens": 393,
            "total_tokens": 63673,
            "prompt_tokens_details": {"cached_tokens": 63280},
            "completion_tokens_details": {"reasoning_tokens": 1}
        },
        "cost": 0.00484425,
        "round": 1,
        "round_started_at": "2026-09-13T15:22:16.8747504+08:00",
        "target_channel": "fengwind",
        "target_model": "glm-5.3-flash",
        "target_protocol": 4,
        "sending": false
    }"#;

    #[test]
    fn octopus_record_maps_to_a_row() {
        let rec: OctopusRecord = serde_json::from_str(OCTOPUS_SAMPLE).unwrap();
        let row = Row::from_octopus(&rec);

        assert_eq!(row.source, Source::Octopus);
        assert_eq!(row.id, "75");
        assert_eq!(row.status, Status::Completed);
        assert_eq!(row.served_model(), "glm-5.3-flash");
        assert_eq!(row.channel.as_deref(), Some("fengwind"));
        assert_eq!(row.caller.as_deref(), Some("omp"));
        // Nanoseconds to milliseconds, truncating the sub-millisecond tail.
        assert_eq!(row.latency_ms, Some(23_908));
        assert_eq!(row.prompt_tokens, 63_280);
        assert_eq!(row.total_tokens, 63_673);
        assert_eq!(row.cached_tokens, 63_280);
        assert_eq!(row.attempt_count, 1);
        // Chat inbound, Responses upstream: a conversion in flight.
        assert!(row.protocol().is_converted());
    }

    #[test]
    fn octopus_record_handles_a_running_request() {
        let rec: OctopusRecord = serde_json::from_str(
            r#"{
                "id": 77, "status": "committed", "started_at": "2026-09-13T15:40:37.9+08:00",
                "duration": 0, "model": "glm-5.3-flash", "protocol": 2, "group_id": 1,
                "api_key_name": "omp",
                "usage": {"prompt_tokens": 0, "completion_tokens": 0, "total_tokens": 0,
                          "prompt_tokens_details": null, "completion_tokens_details": null},
                "cost": 0, "round": 1, "round_started_at": "2026-09-13T15:40:37.9+08:00",
                "target_channel": "fengwind", "target_model": "glm-5.3-flash",
                "target_protocol": 0, "sending": false
            }"#,
        )
        .unwrap();
        let row = Row::from_octopus(&rec);

        assert!(row.status.is_active(), "committed requests are in flight");
        // A zero target protocol is "not yet chosen", not a protocol name.
        assert_eq!(row.format.as_deref(), Some("chat"));
        assert_eq!(row.upstream_format, None);
        assert!(matches!(row.protocol(), Protocol::Single(_)));
    }

    #[test]
    fn octopus_cache_rate_cannot_exceed_one_hundred_percent() {
        // Seen in the wild: cached tokens summed across rounds outrun the prompt.
        let rec: OctopusRecord = serde_json::from_str(
            r#"{"id":1,"status":"success","model":"m","protocol":2,
                "usage":{"prompt_tokens":100,"total_tokens":120,
                         "prompt_tokens_details":{"cached_tokens":140}}}"#,
        )
        .unwrap();
        let row = Row::from_octopus(&rec);
        assert_eq!(row.cache_hit_rate(), Some(100.0));
    }

    #[test]
    fn octopus_url_opens_the_dashboard() {
        assert_eq!(
            octopus_url("http://localhost:8091"),
            "http://localhost:8091/"
        );
        assert_eq!(
            octopus_url("http://localhost:8091/"),
            "http://localhost:8091/"
        );
        assert_eq!(octopus_url(""), "");
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
    fn parse(raw: Option<&str>) -> Self {
        match raw.unwrap_or("") {
            "completed" => Status::Completed,
            "failed" => Status::Failed,
            "processing" => Status::Processing,
            "canceled" => Status::Canceled,
            _ => Status::Pending,
        }
    }

    /// Octopus states: `running` (choosing/waiting upstream), `committed`
    /// (client already receiving the response, no retry possible), `success`,
    /// `failed`, `canceled`. Both in-flight states map to processing.
    fn parse_octopus(raw: Option<&str>) -> Self {
        match raw.unwrap_or("") {
            "success" => Status::Completed,
            "failed" => Status::Failed,
            "canceled" => Status::Canceled,
            "running" | "committed" => Status::Processing,
            _ => Status::Pending,
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
