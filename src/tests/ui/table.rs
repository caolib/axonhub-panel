use super::*;
use crate::model::{CostItem, Status, TierCost};

/// One unit per character: widths become exact and independent of any font.
fn stub(text: &str) -> f32 {
    text.chars().count() as f32
}

fn row() -> Row {
    Row {
        id: String::new(),
        account_id: String::new(),
        account_name: String::new(),
        created_at: None,
        updated_at: None,
        status: Status::Completed,
        model: String::new(),
        routed_model: None,
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

fn view<'a>(rows: &'a [Row], hidden: &'a [DisplayField]) -> ListView<'a> {
    ListView {
        rows,
        now: 0,
        scroll: 0.0,
        hover: None,
        selected: None,
        status_text: None,
        accounts: &[],
        settings_alert: false,
        pinned: false,
        filter: crate::model::FILTER_NONE,
        hidden_fields: hidden,
        id_query: None,
        id_draft: "",
        id_editing: false,
        id_caret: false,
    }
}

#[test]
fn a_column_is_as_wide_as_its_widest_text() {
    let mut long = row();
    long.model = "a-very-long-model-name".into();
    let mut short = row();
    short.model = "glm".into();
    let rows = vec![long, short];
    let columns = vec![Column::Status, Column::Field(DisplayField::Model)];
    let widths = column_widths(&rows, &columns, 0, 2.0, stub);
    // The ● cell under 状态 is one character.
    assert_eq!(widths[0], 1.0 + 4.0);
    // The longest model name on the page is twenty-two.
    assert_eq!(widths[1], 22.0 + 4.0);
}

#[test]
fn the_table_starts_with_status_and_follows_the_hidden_fields() {
    // One row carries a retry so the retry column has something to show;
    // without it the column would be dropped for having no data at all.
    let mut retried = row();
    retried.attempt_count = 2;
    retried.channel_api_key_index = Some(1);
    let rows = vec![retried];
    let hidden = [DisplayField::Tokens, DisplayField::Cache];
    let v = view(&rows, &hidden);
    let columns = Column::all(&v);
    assert_eq!(columns[0], Column::Status);
    assert!(!columns.contains(&Column::Field(DisplayField::Tokens)));
    assert!(!columns.contains(&Column::Field(DisplayField::Cache)));
    let fields: Vec<DisplayField> = columns
        .iter()
        .filter_map(|column| match column {
            Column::Field(field) => Some(*field),
            Column::Status => None,
        })
        .collect();
    let expected: Vec<DisplayField> = DisplayField::ALL
        .into_iter()
        .filter(|field| !hidden.contains(field))
        .collect();
    assert_eq!(fields, expected);
}

#[test]
fn a_retry_column_without_a_single_retry_is_dropped() {
    let rows = vec![row(), row()];
    let v = view(&rows, &[]);
    assert!(!Column::all(&v).contains(&Column::Field(DisplayField::Retry)));

    // One retried request on the page brings the column back.
    let mut retried = row();
    retried.attempt_count = 3;
    let rows = vec![row(), retried];
    let v = view(&rows, &[]);
    assert!(Column::all(&v).contains(&Column::Field(DisplayField::Retry)));
}

#[test]
fn a_retry_count_is_red_and_its_absence_wears_the_status_colour() {
    let rows = vec![row()];
    let v = view(&rows, &[]);
    let column = Column::Field(DisplayField::Retry);
    assert_eq!(column.text(&rows[0], 0), fmt::BLANK);
    assert_eq!(
        column.color(&rows[0], &v, &[], &[]),
        theme::status_color(Status::Completed)
    );

    let mut retried = row();
    retried.attempt_count = 2;
    assert_eq!(column.text(&retried, 0), "1×");
    assert_eq!(column.color(&retried, &v, &[], &[]), theme::RED);
}

#[test]
fn the_status_cell_is_a_dot_coloured_by_state() {
    let mut failed = row();
    failed.status = Status::Failed;
    let v = view(&[], &[]);
    assert_eq!(Column::Status.text(&failed, 0), STATUS_DOT);
    assert_eq!(
        Column::Status.color(&failed, &v, &[], &[]),
        theme::status_color(Status::Failed)
    );
}

#[test]
fn the_marks_name_the_flag_and_their_colour_says_whether_it_is_set() {
    let mut streamed = row();
    streamed.stream = true;
    let v = view(&[], &[]);
    // The character is the column's label — there is no header row — and the
    // colour carries the value, exactly like the card's 流 / 转 / 透 marks.
    assert_eq!(Column::Field(DisplayField::Stream).text(&streamed, 0), "流");
    assert_eq!(
        Column::Field(DisplayField::Conversion).text(&streamed, 0),
        "转"
    );
    assert_eq!(
        Column::Field(DisplayField::PassThrough).text(&streamed, 0),
        "透"
    );
    assert_eq!(
        Column::Field(DisplayField::Stream).color(&streamed, &v, &[], &[]),
        theme::WHITE
    );
    assert_eq!(
        Column::Field(DisplayField::PassThrough).color(&streamed, &v, &[], &[]),
        theme::GRAY
    );
}

#[test]
fn a_cell_without_data_stays_blank_in_the_row_status_colour() {
    // A request still in flight has nothing to report yet: no cache rate, no
    // speed, no latency. Each of those cells stays blank, while the colour
    // remains the row's status colour rather than the usual grey.
    let mut pending = row();
    pending.status = Status::Pending;
    let v = view(&[], &[]);
    for field in [
        DisplayField::UpstreamModel,
        DisplayField::Caller,
        DisplayField::ChannelAPIKeyIndex,
        DisplayField::Cost,
        DisplayField::Retry,
        DisplayField::Account,
        DisplayField::Cache,
        DisplayField::CacheWrite,
        DisplayField::CostBreakdown,
        DisplayField::Speed,
        DisplayField::Latency,
    ] {
        let column = Column::Field(field);
        assert_eq!(column.text(&pending, 0), fmt::BLANK, "{field:?} blank");
        assert_eq!(
            column.color(&pending, &v, &[], &[]),
            theme::status_color(Status::Pending),
            "{field:?} blank colour"
        );
    }
    // The status column always has something to say: its dot.
    assert_eq!(Column::Status.text(&pending, 0), STATUS_DOT);
}

#[test]
fn data_replaces_the_blank_as_soon_as_it_exists() {
    let mut done = row();
    done.status = Status::Completed;
    done.total_tokens = 12_300;
    done.latency_ms = Some(190_400);
    done.total_cost = Some(0.1388);
    let v = view(&[], &[]);
    assert_eq!(Column::Field(DisplayField::Tokens).text(&done, 0), "12.3k");
    assert_eq!(
        Column::Field(DisplayField::Latency).text(&done, 0),
        "[3m10s]"
    );
    assert_eq!(Column::Field(DisplayField::Cost).text(&done, 0), "$0.1388");
    assert_eq!(
        Column::Field(DisplayField::Cost).color(&done, &v, &[], &[]),
        theme::YELLOW
    );
}

#[test]
fn a_routed_model_reads_like_the_card_chip() {
    let mut routed = row();
    routed.model = "asked".into();
    routed.routed_model = Some("served".into());
    routed.reasoning_effort = Some("max".into());
    assert_eq!(
        Column::Field(DisplayField::Model).text(&routed, 0),
        "[served(max)]"
    );
}

#[test]
fn a_request_without_a_cache_rate_shows_its_failure_code_instead() {
    // A failed request has no cache rate, so the cell carries why it ended
    // there: the HTTP status of its latest failed attempt, in red.
    let mut failed = row();
    failed.status = Status::Failed;
    failed.error_status_code = Some(429);
    let v = view(&[], &[]);
    let column = Column::Field(DisplayField::Cache);
    assert_eq!(column.text(&failed, 0), "429");
    assert_eq!(column.color(&failed, &v, &[], &[]), theme::RED);
}

#[test]
fn a_cache_rate_wins_over_the_failure_code() {
    // A request that recovered after a retry carries both; the cache rate is
    // the reading that matters, so the code stays hidden.
    let mut recovered = row();
    recovered.prompt_tokens = 1_000;
    recovered.cached_tokens = 800;
    recovered.error_status_code = Some(500);
    let v = view(&[], &[]);
    let column = Column::Field(DisplayField::Cache);
    assert_eq!(column.text(&recovered, 0), "80.00%");
    assert_eq!(column.color(&recovered, &v, &[], &[]), theme::GREEN);
}

#[test]
fn the_new_columns_show_their_actual_values() {
    let mut rich = row();
    rich.upstream_model = Some("openai/actual-upstream".into());
    rich.channel_api_key_index = Some(0);
    rich.write_cached_tokens = 1536;
    rich.total_cost = Some(0.03);
    rich.cost_items = vec![
        CostItem {
            item_code: "prompt_tokens".into(),
            quantity: 900,
            subtotal: 0.01,
            tier_breakdown: Vec::new(),
        },
        CostItem {
            item_code: "completion_tokens".into(),
            quantity: 300,
            subtotal: 0.02,
            tier_breakdown: vec![TierCost {
                up_to: Some(1_000),
                units: 300,
                subtotal: 0.02,
            }],
        },
    ];
    let rows = [rich.clone()];
    let v = view(&rows, &[]);
    assert_eq!(
        Column::Field(DisplayField::UpstreamModel).text(&rich, 0),
        "actual-upstream",
        "the provider prefix is stripped, like the model column"
    );
    assert_eq!(
        Column::Field(DisplayField::ChannelAPIKeyIndex).text(&rich, 0),
        "Key #0",
        "the raw index is not renumbered"
    );
    assert_eq!(
        Column::Field(DisplayField::CacheWrite).text(&rich, 0),
        "写 1.5k"
    );
    assert_eq!(
        Column::Field(DisplayField::CostBreakdown).text(&rich, 0),
        "入 $0.0100 · 出 $0.0200"
    );
    assert_eq!(
        Column::Field(DisplayField::CostBreakdown).color(&rich, &v, &[], &[]),
        theme::YELLOW
    );
}

#[test]
fn a_key_alias_replaces_the_index_but_keeps_the_same_column_switch() {
    let mut mapped = row();
    mapped.channel_api_key_index = Some(2);
    mapped.channel_key_label = Some("my-k2".into());
    let column = Column::Field(DisplayField::ChannelAPIKeyIndex);
    assert_eq!(column.text(&mapped, 0), "my-k2");
    let rows = [mapped];
    let hidden = [DisplayField::ChannelAPIKeyIndex];
    assert!(!Column::all(&view(&rows, &hidden)).contains(&column));
}

#[test]
fn the_four_new_columns_toggle_independently() {
    let mut keyed = row();
    keyed.channel_api_key_index = Some(1);
    let rows = vec![keyed];
    let new_fields = [
        DisplayField::UpstreamModel,
        DisplayField::ChannelAPIKeyIndex,
        DisplayField::CacheWrite,
        DisplayField::CostBreakdown,
    ];
    for hidden_field in new_fields {
        let hidden = [hidden_field];
        let v = view(&rows, &hidden);
        let columns = Column::all(&v);
        for field in new_fields {
            assert_eq!(
                columns.contains(&Column::Field(field)),
                field != hidden_field,
                "hiding {hidden_field:?} should only remove itself"
            );
        }
    }
}
