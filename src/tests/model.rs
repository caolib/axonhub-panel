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
        account_id: String::new(),
        account_name: String::new(),
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
        attempts_truncated: false,
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

#[test]
fn a_request_with_more_attempts_than_fetched_stays_clickable() {
    // `GetRequests` returns at most 10 executions, so a request with more
    // attempts than that could hide failures we never received. Even though
    // the fetched slice shows no failures, the card must still open.
    let req = wire_request(
        r#"{
          "id": "gid://axonhub/Request/4",
          "status": "completed",
          "modelID": "glm-5.2",
          "executions": { "totalCount": 12, "edges": [
            { "node": { "status": "completed", "modelID": "glm-5.2" } },
            { "node": { "status": "completed", "modelID": "glm-5.2" } },
            { "node": { "status": "completed", "modelID": "glm-5.2" } },
            { "node": { "status": "completed", "modelID": "glm-5.2" } },
            { "node": { "status": "completed", "modelID": "glm-5.2" } },
            { "node": { "status": "completed", "modelID": "glm-5.2" } },
            { "node": { "status": "completed", "modelID": "glm-5.2" } },
            { "node": { "status": "completed", "modelID": "glm-5.2" } },
            { "node": { "status": "completed", "modelID": "glm-5.2" } },
            { "node": { "status": "completed", "modelID": "glm-5.2" } }
          ] }
        }"#,
    );
    let row = Row::from_wire(&req);
    assert_eq!(row.attempt_count, 12);
    assert!(
        row.attempts_truncated,
        "12 attempts vs 10 fetched must be flagged"
    );
    assert!(row.is_error(), "truncated attempts must stay clickable");
}
