//! Octopus log capture via its server-sent-events overview stream.
//!
//! Octopus keeps recent request state in memory and publishes it on
//! `GET /api/v1/log/overview/stream`: on connect it sends a snapshot (newest
//! first), then re-sends a record whenever that request changes. There is no
//! REST snapshot endpoint and no historical query, so the stream is the only
//! source.
//!
//! The panel does not keep the connection open. Each poll reads for a short
//! window and disconnects: the next poll's snapshot carries the latest state of
//! every record anyway, so nothing displayed is lost by reconnecting — it just
//! costs one cheap local connection per poll.
//!
//! Authentication is the `auth` cookie only (the login endpoint exchanges the
//! username/password for a JWT); the panel stores a pasted cookie value.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader};
use std::time::Duration;

use crate::client::ApiError;
use crate::model::{OctopusRecord, Row};

/// How long one poll stays on the stream before hanging up. The snapshot
/// arrives within milliseconds; the remainder only catches live updates of
/// in-flight requests, which the next poll's snapshot would carry anyway. Kept
/// short so a blocking read does not stretch the panel's poll cadence.
pub const WINDOW: Duration = Duration::from_millis(1000);

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

    /// The URL of the overview stream for a gateway root.
    pub fn stream_url(endpoint: &str) -> String {
        format!(
            "{}/api/v1/log/overview/stream",
            endpoint.trim_end_matches('/')
        )
    }

    /// Read one snapshot plus any updates that arrive inside the window.
    ///
    /// A timeout after the snapshot is the normal way this ends and is not an
    /// error; `Err` is reserved for connect and authentication failures, or a
    /// response that carried no stream data at all.
    pub fn fetch_overview(&self, endpoint: &str, token: &str) -> Result<Vec<Row>, ApiError> {
        let response = self
            .agent
            .get(Self::stream_url(endpoint))
            .header("Accept", "text/event-stream")
            .header("Cookie", &format!("auth={token}"))
            .call()
            .map_err(map_transport)?;

        let reader = BufReader::new(response.into_body().into_reader());
        let mut records = BTreeMap::new();
        let saw_data = drain(reader, &mut records);

        if !saw_data {
            return Err(ApiError::Unreachable("日志流没有返回数据".into()));
        }

        Ok(records.values().map(Row::from_octopus).collect())
    }
}

/// Consume SSE lines, upserting each `data:` record. Returns whether anything
/// at all arrived: a healthy but empty log stream still sends a `: connected`
/// comment, so silence means the endpoint is not serving the stream.
fn drain<R: BufRead>(mut reader: R, records: &mut BTreeMap<u64, OctopusRecord>) -> bool {
    let mut saw_data = false;
    let mut line = String::new();
    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => return saw_data,
            Ok(_) => {
                if line.trim().is_empty() {
                    continue;
                }
                saw_data = true;
                if let Some(record) = parse_event(&line) {
                    // Updates arrive repeatedly under the same id; the newest
                    // record wins, which is the whole point of the stream.
                    records.insert(record.id, record);
                }
            }
            // The panel's read window expiring surfaces here. Everything read
            // before it is a complete snapshot, so it is not a failure.
            Err(_) => return saw_data,
        }
    }
}

/// Parse one SSE line into a record. Comments and `event:` lines yield `None`.
fn parse_event(line: &str) -> Option<OctopusRecord> {
    let data = line.strip_prefix("data:")?.trim();
    serde_json::from_str(data).ok()
}

/// Map transport errors onto the panel's error vocabulary. Unlike the AxonHub
/// client, a 401 here means the pasted cookie is stale rather than the app's
/// sign-in having lapsed, but the panel treats both as "credentials gone".
fn map_transport(err: ureq::Error) -> ApiError {
    match err {
        ureq::Error::StatusCode(401) => ApiError::Expired,
        ureq::Error::StatusCode(code) => ApiError::Rejected(format!("HTTP {code}")),
        ureq::Error::Timeout(_) => ApiError::Unreachable("连接超时".into()),
        other => ApiError::Unreachable(other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn builds_the_stream_url_with_or_without_a_trailing_slash() {
        assert_eq!(
            Client::stream_url("http://localhost:8091"),
            "http://localhost:8091/api/v1/log/overview/stream"
        );
        assert_eq!(
            Client::stream_url("http://localhost:8091/"),
            "http://localhost:8091/api/v1/log/overview/stream"
        );
    }

    #[test]
    fn reads_a_snapshot_and_applies_updates_by_id() {
        // Two snapshot records, then a later state for the newer one.
        let stream = concat!(
            ": connected\n\n",
            "event:log\n",
            "data: {\"id\":2,\"status\":\"running\",\"model\":\"a\"}\n\n",
            "event:log\n",
            "data: {\"id\":1,\"status\":\"success\",\"model\":\"b\"}\n\n",
            ": ping\n\n",
            "event:log\n",
            "data: {\"id\":2,\"status\":\"success\",\"model\":\"a\",\"target_model\":\"c\"}\n\n",
        );

        let mut records = BTreeMap::new();
        assert!(drain(Cursor::new(stream), &mut records));
        assert_eq!(records.len(), 2, "updates replace, they do not append");
        assert_eq!(records[&2].status.as_deref(), Some("success"));
        assert_eq!(records[&2].target_model.as_deref(), Some("c"));
        assert_eq!(records[&1].status.as_deref(), Some("success"));
    }

    #[test]
    fn an_empty_stream_is_still_a_successful_connect() {
        let mut records = BTreeMap::new();
        assert!(drain(Cursor::new(": connected\n\n"), &mut records));
        assert!(records.is_empty());
    }

    #[test]
    fn silence_is_reported_as_a_failure() {
        let mut records = BTreeMap::new();
        assert!(!drain(Cursor::new(""), &mut records));
    }

    #[test]
    fn garbage_lines_are_skipped_not_fatal() {
        let stream = "event:log\ndata: {not json}\ndata: {\"id\":3,\"status\":\"failed\"}\n";
        let mut records = BTreeMap::new();
        assert!(drain(Cursor::new(stream), &mut records));
        assert_eq!(records.len(), 1);
        assert_eq!(records[&3].status.as_deref(), Some("failed"));
    }
}
