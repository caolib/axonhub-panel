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

fn channel_row(channel: &str) -> Row {
    let mut r = row(Status::Completed);
    r.channel = Some(channel.to_string());
    r
}

fn model_row(model: &str) -> Row {
    let mut r = row(Status::Completed);
    r.model = model.to_string();
    r
}

/// The color `name` is drawn with, or `None` when it is not on the page.
fn color_of<'a>(palette: &'a [(&'a str, u32)], name: &str) -> Option<u32> {
    palette.iter().find(|(n, _)| *n == name).map(|(_, c)| *c)
}

#[test]
fn the_oldest_channel_keeps_the_default_colour() {
    // `rows` is newest first, as `App::rebuild` leaves it. The palette runs the
    // other way, so the bottom-most card is the one that gets the default blue.
    let rows = vec![channel_row("B"), channel_row("A")];
    let palette = channel_palette(&rows);
    assert_eq!(color_of(&palette, "A"), Some(NAME_PALETTE[0]));
    assert_eq!(color_of(&palette, "B"), Some(NAME_PALETTE[1]));
}

#[test]
fn a_new_channel_does_not_repaint_the_cards_already_shown() {
    let old = vec![channel_row("B"), channel_row("A")];
    let new = vec![channel_row("C"), channel_row("B"), channel_row("A")];
    let before = channel_palette(&old);
    let after = channel_palette(&new);
    for (name, color) in before {
        assert_eq!(
            color_of(&after, name),
            Some(color),
            "{name} changed colour when a newer channel arrived"
        );
    }
    assert_eq!(color_of(&after, "C"), Some(NAME_PALETTE[2]));
}

#[test]
fn a_new_model_does_not_repaint_the_cards_already_shown() {
    let old = vec![model_row("b"), model_row("a")];
    let new = vec![model_row("c"), model_row("b"), model_row("a")];
    let before = model_palette(&old);
    let after = model_palette(&new);
    for (name, color) in before {
        assert_eq!(
            color_of(&after, name),
            Some(color),
            "{name} changed colour when a newer model arrived"
        );
    }
    assert_eq!(color_of(&after, "a"), Some(NAME_PALETTE[0]));
    assert_eq!(color_of(&after, "c"), Some(NAME_PALETTE[2]));
}

#[test]
fn models_are_keyed_by_what_served_them() {
    // A routed request is coloured by the model that served it, not the one asked for.
    let mut routed = model_row("asked");
    routed.routed_model = Some("served".to_string());
    let rows = vec![routed, model_row("a")];
    let palette = model_palette(&rows);
    assert_eq!(color_of(&palette, "served"), Some(NAME_PALETTE[1]));
    assert_eq!(color_of(&palette, "asked"), None);
    assert_eq!(color_of(&palette, "a"), Some(NAME_PALETTE[0]));
}

#[test]
fn rows_without_a_channel_do_not_consume_a_colour() {
    let rows = vec![row(Status::Completed), channel_row("A")];
    let palette = channel_palette(&rows);
    assert_eq!(palette.len(), 1);
    assert_eq!(color_of(&palette, "A"), Some(NAME_PALETTE[0]));
}

#[test]
fn the_palette_wraps_past_its_last_entry() {
    // Newest first: n10 is at the top, n1 at the bottom. Ten names over nine
    // colours means the two ends collide, and every other name still shifts by
    // exactly one slot.
    let rows: Vec<Row> = (1..=10)
        .rev()
        .map(|i| channel_row(&format!("n{i}")))
        .collect();
    let palette = channel_palette(&rows);
    assert_eq!(palette.len(), 10);
    assert_eq!(color_of(&palette, "n1"), Some(NAME_PALETTE[0]));
    assert_eq!(color_of(&palette, "n2"), Some(NAME_PALETTE[1]));
    assert_eq!(color_of(&palette, "n9"), Some(NAME_PALETTE[8]));
    assert_eq!(color_of(&palette, "n10"), Some(NAME_PALETTE[0]));
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
