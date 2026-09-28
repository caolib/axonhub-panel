use super::*;
use crate::model::Status;

fn row(status: Status) -> Row {
    Row {
        id: String::new(),
        account_id: String::new(),
        account_name: String::new(),
        created_at: None,
        updated_at: None,
        status,
        model: String::new(),
        routed_model: None,
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
        attempt_count: 1,
        failed_attempts: 0,
        attempts_truncated: false,
    }
}

fn seeded() -> App {
    let mut app = App::new(Config::default());
    app.axon_rows = vec![
        row(Status::Completed),
        row(Status::Failed),
        row(Status::Processing),
        row(Status::Pending),
        row(Status::Canceled),
    ];
    app.rebuild();
    app
}

#[test]
fn filter_click_sets_the_mask_and_drops_the_stale_page() {
    let mut app = seeded();
    assert_eq!(app.rows.len(), 5);

    // Status is applied by the next poll, so the previous page is cleared
    // rather than shown as if it were already filtered.
    assert!(app.click_filter(Filter::Completed));
    assert_eq!(app.filter, model::FILTER_COMPLETED);
    assert!(app.rows.is_empty());

    assert!(app.click_filter(Filter::All));
    assert_eq!(app.filter, model::FILTER_NONE);

    assert!(app.click_filter(Filter::Active));
    assert_eq!(app.filter, model::FILTER_ACTIVE);

    assert!(app.click_filter(Filter::All));
    assert!(app.click_filter(Filter::Failed));
    assert_eq!(app.filter, model::FILTER_FAILED);

    // 全部 while the mask is already empty changes nothing.
    assert!(app.click_filter(Filter::All));
    assert_eq!(app.filter, model::FILTER_NONE);
    assert!(!app.click_filter(Filter::All));
}

#[test]
fn chips_combine_and_toggle_independently() {
    let mut app = seeded();

    assert!(app.click_filter(Filter::Completed));
    assert!(app.click_filter(Filter::Active));
    assert_eq!(app.filter, model::FILTER_COMPLETED | model::FILTER_ACTIVE);

    // Toggling one off keeps the other.
    assert!(app.click_filter(Filter::Active));
    assert_eq!(app.filter, model::FILTER_COMPLETED);

    // Toggling the last one off returns to the unfiltered query.
    assert!(app.click_filter(Filter::Completed));
    assert_eq!(app.filter, model::FILTER_NONE);
}

#[test]
fn hiding_a_channel_only_drops_that_accounts_rows() {
    let mut app = App::new(Config::default());
    // The same channel name on both accounts: A forwards to B through it,
    // so the request is listed by both.
    let mut from_a = row(Status::Completed);
    from_a.account_id = "a1".into();
    from_a.account_name = "A".into();
    from_a.channel = Some("relay".into());
    let mut from_b = row(Status::Completed);
    from_b.account_id = "a2".into();
    from_b.account_name = "B".into();
    from_b.channel = Some("relay".into());
    app.axon_rows = vec![from_a, from_b];
    app.rebuild();
    assert_eq!(app.rows.len(), 2);

    app.config.hidden_channels.push(HiddenChannel {
        account_id: "a1".into(),
        channel: "relay".into(),
    });
    app.rebuild();
    assert_eq!(app.rows.len(), 1);
    assert_eq!(app.rows[0].account_id, "a2");

    // And the filters the menu offers keep the hidden pair listed, so it
    // can be switched back on.
    let filters = app.channel_filters();
    assert!(filters.contains(&HiddenChannel {
        account_id: "a1".into(),
        channel: "relay".into(),
    }));
}

#[test]
fn model_abbreviations_apply_on_rebuild_and_survive_it() {
    let mut app = App::new(Config::default());
    let mut routed = row(Status::Completed);
    routed.model = "deepseek-v4.1-flash".into();
    routed.routed_model = Some("DeepSeek-R1".into());
    app.axon_rows = vec![routed];
    app.config.upsert_model_alias("deepseek", "ds");
    app.rebuild();

    // Both the requested and the served model carry the abbreviation, and
    // the routed marker survives the rewrite.
    assert_eq!(app.rows[0].model, "ds-v4.1-flash");
    assert_eq!(app.rows[0].routed_model.as_deref(), Some("ds-R1"));
    assert!(app.rows[0].is_routed());

    // The worker's copy is untouched, so removing the alias restores the
    // original names on the next rebuild.
    app.config.remove_model_alias(0);
    app.rebuild();
    assert_eq!(app.rows[0].model, "deepseek-v4.1-flash");
    assert_eq!(app.rows[0].routed_model.as_deref(), Some("DeepSeek-R1"));
}
