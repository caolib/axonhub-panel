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
        upstream_model: None,
        channel: None,
        channel_api_key_index: None,
        channel_key_label: None,
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
        cost_items: Vec::new(),
        attempt_count: 0,
        failed_attempts: 0,
        attempts_truncated: false,
        error_status_code: None,
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

#[test]
fn latest_execution_keeps_the_raw_upstream_model_and_zero_key_index() {
    let req = wire_request(
        r#"{
          "id": "gid://axonhub/Request/8",
          "modelID": "client-model",
          "executions": { "totalCount": 2, "edges": [
            { "node": { "modelID": "routed-model", "upstreamModelID": " provider/raw-model ", "channelAPIKeyIndex": 0 } },
            { "node": { "modelID": "old-model", "upstreamModelID": "old-raw", "channelAPIKeyIndex": 3 } }
          ] }
        }"#,
    );
    let row = Row::from_wire(&req);
    assert_eq!(row.model, "client-model");
    assert_eq!(row.routed_model.as_deref(), Some("routed-model"));
    assert_eq!(row.upstream_model.as_deref(), Some("provider/raw-model"));
    assert_eq!(row.channel_api_key_index, Some(0));

    let req = wire_request(
        r#"{
          "id": "gid://axonhub/Request/9",
          "executions": { "edges": [
            { "node": { "upstreamModelID": "  " } },
            { "node": { "upstreamModelID": "old-raw", "channelAPIKeyIndex": 3 } }
          ] }
        }"#,
    );
    let row = Row::from_wire(&req);
    assert!(row.upstream_model.is_none());
    assert!(row.channel_api_key_index.is_none());
}

#[test]
fn execution_detail_preserves_uppercase_acronym_fields() {
    let execution: ExecutionDetail = serde_json::from_str(
        r#"{"modelID":"mapped","upstreamModelID":"actual","channelAPIKeyIndex":0}"#,
    )
    .unwrap();
    assert_eq!(execution.upstream_model_id.as_deref(), Some("actual"));
    assert_eq!(execution.channel_api_key_index, Some(0));
}

#[test]
fn cost_items_accept_decimal_strings_numbers_and_optional_tiers() {
    let req = wire_request(
        r#"{
          "id": "gid://axonhub/Request/10",
          "usageLogs": { "edges": [ { "node": {
            "promptWriteCachedTokens": 3000,
            "costItems": [
              { "itemCode": "prompt_tokens", "quantity": 1000, "subtotal": "0.0012", "tierBreakdown": null },
              { "itemCode": "completion_tokens", "quantity": 200, "subtotal": 0.0034,
                "tierBreakdown": [{"upTo": null, "units": 200, "subtotal": "0.0034"}] },
              { "itemCode": "provider_special", "quantity": 1, "subtotal": "0" }
            ]
          } } ] }
        }"#,
    );
    let row = Row::from_wire(&req);
    assert_eq!(row.write_cached_tokens, 3000);
    assert_eq!(row.cost_items.len(), 3);
    assert_eq!(row.cost_items[0].label(), "输入");
    assert_eq!(row.cost_items[0].subtotal, 0.0012);
    assert!(row.cost_items[0].tier_breakdown.is_empty());
    assert_eq!(row.cost_items[1].tier_breakdown[0].up_to, None);
    assert_eq!(row.cost_items[1].tier_breakdown[0].units, 200);
    assert_eq!(row.cost_items[1].tier_breakdown[0].subtotal, 0.0034);
    assert_eq!(row.cost_items[2].label(), "provider_special");
    assert_eq!(
        row.cost_breakdown_text().as_deref(),
        Some("入 $0.0012 · 出 $0.0034 · provider_special $0.0000")
    );
}

#[test]
fn absent_or_null_cost_items_and_upstream_metadata_stay_empty() {
    for usage in [r#"{}"#, r#"{"costItems":null}"#, r#"{"costItems":[]}"#] {
        let json = format!(
            r#"{{"id":"gid://axonhub/Request/11","usageLogs":{{"edges":[{{"node":{usage}}}]}}}}"#
        );
        let row = Row::from_wire(&wire_request(&json));
        assert!(row.cost_items.is_empty());
        assert!(row.cost_breakdown_text().is_none());
        assert!(row.upstream_model.is_none());
        assert!(row.channel_api_key_index.is_none());
    }
}

#[test]
fn cost_items_reject_invalid_decimal_amounts() {
    for amount in [r#""NaN""#, r#""inf""#, r#""not-a-number""#, "-1", "null"] {
        let json = format!(r#"{{"itemCode":"prompt_tokens","quantity":1,"subtotal":{amount}}}"#);
        assert!(serde_json::from_str::<CostItem>(&json).is_err(), "{json}");
    }
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
fn the_latest_failed_attempts_status_code_is_kept() {
    // Attempts arrive newest first: the code kept for the table is the newest
    // failure's, so a retried request reports what it ended with rather than
    // what it started with, and a success's 200 is never picked up.
    let req = wire_request(
        r#"{
          "id": "gid://axonhub/Request/5",
          "status": "failed",
          "modelID": "glm-5.2",
          "executions": { "totalCount": 3, "edges": [
            { "node": { "status": "failed", "responseStatusCode": 500 } },
            { "node": { "status": "failed", "responseStatusCode": 429 } },
            { "node": { "status": "completed", "responseStatusCode": 200 } }
          ] }
        }"#,
    );
    let row = Row::from_wire(&req);
    assert_eq!(row.error_status_code, Some(500));
}

#[test]
fn a_failure_without_a_code_falls_back_to_the_last_one_that_has_it() {
    // A cancellation carries no code of its own; the cell shows the last
    // error seen instead of going blank.
    let req = wire_request(
        r#"{
          "id": "gid://axonhub/Request/6",
          "status": "canceled",
          "modelID": "glm-5.2",
          "executions": { "totalCount": 2, "edges": [
            { "node": { "status": "canceled" } },
            { "node": { "status": "failed", "responseStatusCode": 429 } }
          ] }
        }"#,
    );
    let row = Row::from_wire(&req);
    assert_eq!(row.error_status_code, Some(429));

    // No failed attempt at all: nothing to show.
    let req = wire_request(
        r#"{
          "id": "gid://axonhub/Request/7",
          "status": "completed",
          "modelID": "glm-5.2",
          "executions": { "totalCount": 1, "edges": [
            { "node": { "status": "completed", "responseStatusCode": 200 } }
          ] }
        }"#,
    );
    let row = Row::from_wire(&req);
    assert_eq!(row.error_status_code, None);
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
