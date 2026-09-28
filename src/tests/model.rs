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
        updated_at: None,
        status: Status::Completed,
        model: requested.to_string(),
        routed_model: served.map(str::to_string),
        channel: None,
        caller: None,
        source: None,
        client_ip: None,
        format: None,
        reasoning_effort: None,
        upstream_format: None,
        pass_through: false,
        stream: false,
        latency_ms: None,
        first_token_ms: None,
        reasoning_ms: None,
        prompt_tokens: 0,
        completion_tokens: 0,
        reasoning_tokens: 0,
        total_tokens: 0,
        cached_tokens: 0,
        write_cached_tokens: 0,
        total_cost: None,
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
fn usage_and_caller_follow_the_requests_page() {
    let req = wire_request(
        r#"{
          "id": "gid://axonhub/Request/50944",
          "createdAt": "2026-09-23T11:59:59.0343735Z",
          "updatedAt": "2026-09-23T12:00:57.8669073Z",
          "status": "completed",
          "modelID": "gpt-6-astra",
          "source": "api",
          "clientIP": "::1",
          "metricsLatencyMs": 58749,
          "metricsFirstTokenLatencyMs": 56870,
          "metricsReasoningDurationMs": 1200,
          "apiKey": { "name": "cc", "user": { "firstName": "caolib", "lastName": "cao" } },
          "usageLogs": { "edges": [ { "node": {
            "promptTokens": 266568,
            "completionTokens": 1138,
            "completionReasoningTokens": 1025,
            "totalTokens": 267706,
            "promptCachedTokens": 264519,
            "promptWriteCachedTokens": 2046,
            "totalCost": 0.1388096
          } } ] }
        }"#,
    );
    let row = Row::from_wire(&req);
    assert_eq!(row.caller.as_deref(), Some("cc"));
    assert_eq!(row.source.as_deref(), Some("api"));
    assert_eq!(row.client_ip.as_deref(), Some("::1"));
    assert_eq!(row.latency_ms, Some(58_749));
    assert_eq!(row.first_token_ms, Some(56_870));
    assert_eq!(row.reasoning_ms, Some(1_200));
    assert_eq!(row.prompt_tokens, 266_568);
    assert_eq!(row.completion_tokens, 1_138);
    assert_eq!(row.reasoning_tokens, 1_025);
    assert_eq!(row.total_tokens, 267_706);
    assert_eq!(row.cached_tokens, 264_519);
    assert_eq!(row.write_cached_tokens, 2_046);
    assert_eq!(row.total_cost, Some(0.1388096));
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

#[test]
fn strips_the_vendor_prefix_from_model_names() {
    assert_eq!(short_model_name("zhipu/glm-5.2"), "glm-5.2");
    assert_eq!(short_model_name("a/b/c"), "c");
    assert_eq!(short_model_name("glm-5.2"), "glm-5.2");
    assert_eq!(short_model_name(""), "");

    let routed = row_with("req", Some("vendor/served-model"));
    assert_eq!(routed.served_model(), "served-model");
    let plain = row_with("vendor/requested-model", None);
    assert_eq!(plain.served_model(), "requested-model");
}

#[test]
fn model_aliases_rewrite_every_match_in_order() {
    use crate::config::ModelAlias;
    let alias = |from: &str, to: &str| ModelAlias {
        from: from.into(),
        to: to.into(),
    };
    let deepseek = vec![alias("deepseek", "ds")];

    // The case from the settings hint: matches are replaced
    // case-insensitively, the rest of the name keeps its own case.
    assert_eq!(
        display_model_name("deepseek-v4.1-flash", &deepseek),
        "ds-v4.1-flash"
    );
    assert_eq!(
        display_model_name("DeepSeek-V4.1-Flash", &deepseek),
        "ds-V4.1-Flash"
    );
    // The vendor prefix is stripped first, so the alias matches the short name.
    assert_eq!(
        display_model_name("vendor/deepseek-chat", &deepseek),
        "ds-chat"
    );
    // A model that matches nothing passes through unchanged.
    assert_eq!(display_model_name("glm-5.2", &deepseek), "glm-5.2");
    // An empty alias list is the same as none.
    assert_eq!(display_model_name("deepseek-v4", &[]), "deepseek-v4");

    // An empty short form deletes the matches instead of replacing them.
    let strip = vec![alias("deepseek-", "")];
    assert_eq!(
        display_model_name("deepseek-v4.1-flash", &strip),
        "v4.1-flash"
    );
    assert_eq!(display_model_name("DeepSeek-X", &strip), "X");

    // Rules chain: each one rewrites every occurrence (anywhere in the name,
    // not just the head) in the result of the previous one.
    let chained = vec![alias("deepseek", "ds"), alias("flash", "f")];
    assert_eq!(
        display_model_name("deepseek-v4.1-flash", &chained),
        "ds-v4.1-f"
    );
    assert_eq!(
        display_model_name("DeepSeek-V4.1-Flash", &chained),
        "ds-V4.1-f"
    );

    // Overlapping rules resolve by order: an earlier rewrite can hide a
    // later rule's match.
    let many = vec![alias("deepseek", "ds"), alias("deepseek-r1", "r1")];
    assert_eq!(display_model_name("deepseek-r1-0528", &many), "ds-r1-0528");
    assert_eq!(display_model_name("deepseek-chat", &many), "ds-chat");

    // Blank or whitespace-only sides never match, and a re-added `from`
    // replaces the short form in place.
    let mut config = crate::config::Config::default();
    config.upsert_model_alias("deepseek", "ds");
    config.upsert_model_alias("DeepSeek", "DS");
    assert_eq!(config.model_aliases.len(), 1);
    assert_eq!(config.model_aliases[0].to, "DS");
    config.remove_model_alias(0);
    config.remove_model_alias(5);
    assert!(config.model_aliases.is_empty());
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
