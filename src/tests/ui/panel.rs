use super::*;
use crate::model::{Status, mask_matches};
use crate::ui::layout::Metrics;

fn view<'a>(rows: &'a [Row], pinned: bool) -> ListView<'a> {
    ListView {
        rows,
        total: rows.len() as i64,
        now: 0,
        scroll: 0.0,
        hover: None,
        selected: None,
        status_text: None,
        user: None,
        accounts: &[],
        settings_alert: false,
        pinned,
        filter: model::FILTER_NONE,
        hidden_fields: &[],
    }
}

#[test]
fn both_layouts_fit_their_card_height() {
    for single in [false, true] {
        let m = Metrics::new(452.0, 300.0, 1.0).with_single_line(single);
        let c = CardMetrics::new(m);
        assert_eq!(c.single, single);
        let band = c.band_h(m.card_h);
        let content = if single {
            // One line: the chip is the tallest element.
            c.chip_h()
        } else {
            c.line1 + c.gap + c.line2
        };
        assert!(
            content <= band + 0.01,
            "single {single}: content {content} overflows the {band}px band"
        );
        assert!(
            c.line1 <= band + 0.01,
            "single {single}: the body line does not fit the band"
        );
    }
}

#[test]
fn chips_hit_where_they_are_drawn() {
    let m = Metrics::new(452.0, 300.0, 1.0);
    let v = view(&[], false);
    for (i, f) in FILTERS.iter().enumerate() {
        let r = chip_rect(m, &v, *f);
        assert_eq!(
            hit_filter(m, &v, r.x + r.w / 2.0, r.y + r.h / 2.0),
            Some(*f),
            "chip {} ({}) should hit at its center",
            i,
            f.label()
        );
    }
}

#[test]
fn gaps_and_the_list_area_do_not_hit() {
    let m = Metrics::new(452.0, 300.0, 1.0);
    let v = view(&[], false);
    let first = chip_rect(m, &v, FILTERS[0]);
    let second = chip_rect(m, &v, FILTERS[1]);
    let gap_mid = first.x + first.w + (second.x - first.x - first.w) / 2.0;
    assert!(hit_filter(m, &v, gap_mid, first.y + 1.0).is_none());
    assert!(hit_filter(m, &v, 10.0, m.header_h + 1.0).is_none());
    assert!(hit_filter(m, &v, -1.0, 10.0).is_none());
}

#[test]
fn pinned_mode_disables_the_chips() {
    let m = Metrics::new(452.0, 300.0, 1.0);
    let v = view(&[], true);
    assert!(hit_filter(m, &v, 10.0, 10.0).is_none());
}

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
        attempt_count: 0,
        failed_attempts: 0,
        attempts_truncated: false,
    }
}

#[test]
fn only_cards_with_something_to_explain_are_clickable() {
    let m = Metrics::new(452.0, 300.0, 1.0);
    // A request that ended well, but only after an attempt failed, is as
    // clickable as one that failed outright.
    let mut recovered = row(Status::Completed);
    recovered.failed_attempts = 1;
    let rows = vec![
        row(Status::Completed),
        recovered,
        row(Status::Failed),
        row(Status::Canceled),
        row(Status::Processing),
    ];
    let v = view(&rows, false);
    let middle = |index: usize| m.rows_top() + index as f32 * m.pitch() + m.card_h / 2.0;

    assert_eq!(hit_error_row(m, &v, 100.0, middle(0)), None);
    assert_eq!(hit_error_row(m, &v, 100.0, middle(1)), Some(1));
    assert_eq!(hit_error_row(m, &v, 100.0, middle(2)), Some(2));
    assert_eq!(hit_error_row(m, &v, 100.0, middle(3)), Some(3));
    assert_eq!(hit_error_row(m, &v, 100.0, middle(4)), None);
    // Outside the card area nothing hits, error or not.
    assert_eq!(hit_error_row(m, &v, 100.0, m.header_h / 2.0), None);

    // Pinned mode makes the whole list display-only, like the chips.
    let pinned = view(&rows, true);
    assert_eq!(hit_error_row(m, &pinned, 100.0, middle(2)), None);
}

#[test]
fn filter_matches_status() {
    // The empty mask shows everything, including Canceled.
    assert!(mask_matches(model::FILTER_NONE, Status::Canceled));
    assert!(mask_matches(model::FILTER_COMPLETED, Status::Completed));
    assert!(!mask_matches(model::FILTER_COMPLETED, Status::Failed));
    assert!(mask_matches(model::FILTER_FAILED, Status::Failed));
    assert!(!mask_matches(model::FILTER_FAILED, Status::Pending));
    assert!(mask_matches(model::FILTER_ACTIVE, Status::Processing));
    assert!(mask_matches(model::FILTER_ACTIVE, Status::Pending));
    assert!(!mask_matches(model::FILTER_ACTIVE, Status::Completed));
    // Combined masks accept either state.
    assert!(mask_matches(
        model::FILTER_COMPLETED | model::FILTER_ACTIVE,
        Status::Processing
    ));
    assert!(mask_matches(
        model::FILTER_COMPLETED | model::FILTER_ACTIVE,
        Status::Completed
    ));
    assert!(!mask_matches(
        model::FILTER_COMPLETED | model::FILTER_ACTIVE,
        Status::Failed
    ));
}
