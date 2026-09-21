use super::*;
use crate::model::{DetailData, Envelope, Status};

/// Two attempts of one failed request — the shape the popup exists for.
/// Values are stand-ins: the real ones carry gateway internals.
const SAMPLE: &str = r#"{
  "data": { "node": { "executions": { "edges": [
    { "node": {
      "createdAt": "2026-09-18T11:32:33.2110863Z",
      "updatedAt": "2026-09-18T11:33:23.0073162Z",
      "modelID": "glm-5.3-flash",
      "channel": { "id": "gid://axonhub/Channel/2", "name": "ch-b", "type": "openai", "baseURL": "https://api.example.com/v1" },
      "status": "canceled",
      "responseStatusCode": null,
      "errorMessage": "failed to do request: HTTP request failed: context canceled",
      "requestURL": "https://api.example.com/v1/chat/completions",
      "format": "openai/chat_completions",
      "reasoningEffort": "max",
      "passThroughApplied": false,
      "metricsFirstTokenLatencyMs": null,
      "metricsReasoningDurationMs": null
    } },
    { "node": {
      "createdAt": "2026-09-18T11:28:23.0328274Z",
      "updatedAt": "2026-09-18T11:32:32.2058876Z",
      "modelID": "glm-5.3-flash",
      "channel": { "id": "gid://axonhub/Channel/1", "name": "ch-a", "type": "openai", "baseURL": "http://10.0.0.1:18090/v1" },
      "status": "failed",
      "responseStatusCode": 429,
      "errorMessage": "Concurrency limit exceeded for account, please retry later",
      "requestURL": "http://10.0.0.1:18090/v1/chat/completions",
      "format": "openai/chat_completions",
      "reasoningEffort": "max",
      "passThroughApplied": false,
      "metricsFirstTokenLatencyMs": null,
      "metricsReasoningDurationMs": null
    } }
  ], "totalCount": 2 } } }
}"#;

fn executions() -> Vec<ExecutionDetail> {
    // Parsed through the envelope, exactly as the client does.
    let envelope: Envelope<DetailData> = serde_json::from_str(SAMPLE).expect("sample parses");
    envelope
        .data
        .expect("data")
        .node
        .expect("node")
        .executions
        .expect("executions")
        .edges
        .into_iter()
        .filter_map(|e| e.node)
        .collect()
}

fn row() -> Row {
    Row {
        id: "gid://axonhub/Request/42899".into(),
        account_id: String::new(),
        account_name: String::new(),
        created_at: Some("2026-09-18T11:28:03Z".into()),
        status: Status::Failed,
        model: "glm-5.3-flash".into(),
        routed_model: None,
        channel: Some("ch-a".into()),
        caller: Some("cc".into()),
        format: Some("chat".into()),
        reasoning_effort: Some("max".into()),
        upstream_format: Some("chat".into()),
        pass_through: false,
        stream: false,
        latency_ms: Some(269_000),
        first_token_ms: None,
        prompt_tokens: 900,
        total_tokens: 1200,
        cached_tokens: 0,
        attempt_count: 2,
        failed_attempts: 1,
        attempts_truncated: false,
    }
}

#[test]
fn parses_the_executions_query_answer() {
    let found = executions();
    assert_eq!(found.len(), 2);
    assert_eq!(found[0].status(), Status::Canceled);
    assert_eq!(found[1].status(), Status::Failed);
    assert_eq!(found[1].response_status_code, Some(429));
    assert_eq!(
        found[1].channel.as_ref().and_then(|c| c.name.as_deref()),
        Some("ch-a")
    );
}

#[test]
fn the_full_document_carries_every_channel_code_and_error() {
    let doc = Doc::build(&row(), &executions(), 1_789_179_657, Depth::Full);
    let text = plain_text(&doc);

    assert!(text.contains("状态: 失败"), "{text}");
    assert!(text.contains("模型: glm-5.3-flash(max)"), "{text}");
    assert!(text.contains("执行 2 次"), "{text}");
    assert!(text.contains("执行 1 · 已取消"), "{text}");
    assert!(text.contains("执行 2 · 失败 · HTTP 429"), "{text}");
    assert!(text.contains("渠道: ch-a(openai)"), "{text}");
    assert!(text.contains("端点: http://10.0.0.1:18090/v1"), "{text}");
    assert!(
        text.contains("错误: Concurrency limit exceeded for account, please retry later"),
        "{text}"
    );
    assert!(
        text.contains("错误: failed to do request: HTTP request failed: context canceled"),
        "{text}"
    );
    // Every line stands alone — a field never runs into the next heading,
    // and a blank line separates one execution from the next.
    assert!(
        text.contains("context canceled\n\n执行 2 · 失败 · HTTP 429\n"),
        "{text:?}"
    );
    assert!(
        text.contains("地址: https://api.example.com/v1/chat/completions"),
        "{text}"
    );
}

#[test]
fn the_compact_document_keeps_the_channel_code_model_and_error() {
    let doc = Doc::build(&row(), &executions(), 1_789_179_657, Depth::Compact);
    let text = plain_text(&doc);

    assert!(text.contains("执行 1 · 已取消"), "{text}");
    assert!(text.contains("执行 2 · 失败"), "{text}");
    assert!(text.contains("渠道: ch-b(openai)"), "{text}");
    assert!(text.contains("渠道: ch-a(openai)"), "{text}");
    assert!(text.contains("状态码: 429"), "{text}");
    assert!(text.contains("模型: glm-5.3-flash(max)"), "{text}");
    assert!(
        text.contains("错误: Concurrency limit exceeded for account, please retry later"),
        "{text}"
    );
    // Everything else is behind the toggle.
    for hidden in [
        "请求",
        "调用者",
        "协议",
        "令牌",
        "耗时",
        "端点",
        "地址",
        "格式",
        "HTTP 429",
    ] {
        assert!(
            !text.contains(hidden),
            "{hidden} leaked into the compact view: {text}"
        );
    }
    // Every attempt gets a status row, even the one that never got a status
    // line — there the value is a dash rather than a number.
    assert_eq!(text.matches("状态码:").count(), 2, "{text}");
    assert!(text.contains("状态码: —"), "{text}");
}

#[test]
fn a_request_without_executions_says_so() {
    for depth in [Depth::Compact, Depth::Full] {
        let doc = Doc::build(&row(), &[], 1_789_179_657, depth);
        let text = plain_text(&doc);
        assert!(text.contains("没有记录到上游执行"), "{depth:?}: {text}");
    }
}

#[test]
fn buttons_hit_where_they_are_drawn() {
    let m = Popup::new(470.0, 430.0, 1.0);
    for button in [Button::Close, Button::Toggle, Button::Copy, Button::Open] {
        let r = button_rect(&m, button);
        let hit = hit_button(&m, r.x + r.w / 2.0, r.y + r.h / 2.0);
        assert_eq!(hit, Some(button), "{button:?} at {r:?}");
    }
    // The footer controls must not overlap: 完整信息 on the left, then 复制
    // and 打开网页 on the right.
    let toggle = button_rect(&m, Button::Toggle);
    let copy = button_rect(&m, Button::Copy);
    let open = button_rect(&m, Button::Open);
    assert!(copy.x > toggle.x + toggle.w, "{toggle:?} vs {copy:?}");
    assert!(open.x > copy.x + copy.w, "{copy:?} vs {open:?}");
    // The body is not a button.
    assert_eq!(hit_button(&m, m.width / 2.0, m.height / 2.0), None);
    // The toggle names what clicking it shows.
    assert_eq!(Button::Toggle.label(Depth::Compact), "完整信息");
    assert_eq!(Button::Toggle.label(Depth::Full), "简化信息");
}

#[test]
fn scaled_popup_keeps_its_proportions() {
    let at_100 = Popup::new(470.0, 430.0, 1.0);
    let at_150 = Popup::new(705.0, 645.0, 1.5);
    assert_eq!(at_150.row_h(), at_100.row_h() * 1.5);
    assert_eq!(at_150.content_top(), at_100.content_top() * 1.5);
    let r = button_rect(&at_150, Button::Copy);
    assert!(r.x + r.w <= at_150.width - at_150.pad() + 0.01);
    assert!(r.y + r.h <= at_150.height + 0.01);
    // The close button sits inside the header.
    let c = button_rect(&at_150, Button::Close);
    assert!(c.y + c.h <= at_150.header_h() + 0.01);
}
