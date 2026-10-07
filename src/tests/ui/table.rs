use super::*;
use crate::model::Status;

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

fn view<'a>(rows: &'a [Row], hidden: &'a [DisplayField]) -> ListView<'a> {
    ListView {
        rows,
        now: 0,
        scroll: 0.0,
        hover: None,
        selected: None,
        status_text: None,
        user: None,
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
fn the_retry_dash_is_gray_while_a_real_retry_is_red() {
    let rows = vec![row()];
    let v = view(&rows, &[]);
    let column = Column::Field(DisplayField::Retry);
    assert_eq!(column.text(&rows[0], 0), fmt::DASH);
    assert_eq!(column.color(&rows[0], &v, &[], &[]), theme::TEXT_DIM);

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
fn cells_fall_back_to_a_dash_instead_of_disappearing() {
    let empty = row();
    assert_eq!(
        Column::Field(DisplayField::Caller).text(&empty, 0),
        fmt::DASH
    );
    assert_eq!(Column::Field(DisplayField::Cost).text(&empty, 0), fmt::DASH);
    assert_eq!(
        Column::Field(DisplayField::Retry).text(&empty, 0),
        fmt::DASH
    );
    assert_eq!(
        Column::Field(DisplayField::Account).text(&empty, 0),
        fmt::DASH
    );
    // The status column always has something to say: its dot.
    assert_eq!(Column::Status.text(&empty, 0), STATUS_DOT);
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
