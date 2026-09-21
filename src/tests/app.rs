use super::*;
use crate::model::Status;

fn row(status: Status) -> Row {
    Row {
        id: String::new(),
        account_id: String::new(),
        account_name: String::new(),
        created_at: None,
        status,
        model: String::new(),
        routed_model: None,
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
        attempt_count: 1,
        failed_attempts: 0,
        attempts_truncated: false,
    }
}

fn seeded() -> App {
    let mut app = App::new(Config::default());
    app.axon_total = 5;
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
fn filter_keeps_only_matching_rows() {
    let mut app = seeded();
    assert_eq!(app.rows.len(), 5);

    app.click_filter(Filter::Completed);
    assert_eq!(app.rows.len(), 1);
    assert_eq!(app.rows[0].status, Status::Completed);

    app.click_filter(Filter::All);
    assert_eq!(app.rows.len(), 5);

    // 进行 covers both Processing and Pending, mirroring `is_active`.
    app.click_filter(Filter::Active);
    assert_eq!(app.rows.len(), 2);
    assert!(app.rows.iter().all(|r| r.status.is_active()));

    app.click_filter(Filter::All);
    app.click_filter(Filter::Failed);
    assert_eq!(app.rows.len(), 1);
    assert_eq!(app.rows[0].status, Status::Failed);

    app.click_filter(Filter::All);
    assert_eq!(app.rows.len(), 5);
}

#[test]
fn chips_combine_and_toggle_independently() {
    let mut app = seeded();

    // 成功 + 进行 selected at once.
    app.click_filter(Filter::Completed);
    app.click_filter(Filter::Active);
    assert_eq!(app.rows.len(), 3);
    assert!(
        app.rows
            .iter()
            .all(|r| { r.status == Status::Completed || r.status.is_active() })
    );

    // Toggling one off keeps the other.
    app.click_filter(Filter::Active);
    assert_eq!(app.rows.len(), 1);
    assert_eq!(app.rows[0].status, Status::Completed);

    // Toggling the last one off returns to the full list.
    app.click_filter(Filter::Completed);
    assert_eq!(app.rows.len(), 5);
    assert_eq!(app.filter, model::FILTER_NONE);
}

#[test]
fn total_always_counts_every_source_row() {
    let mut app = seeded();
    let total_was = app.total;
    app.click_filter(Filter::Failed);
    assert_eq!(app.total, total_was);
    assert_eq!(app.rows.len(), 1);
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
    app.axon_total = 2;
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
