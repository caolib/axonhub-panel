use super::*;

#[test]
fn layout_scales_with_the_monitor() {
    let at_100 = Metrics::new(400.0, 800.0, 1.0);
    let at_150 = Metrics::new(600.0, 1200.0, 1.5);
    assert_eq!(at_100.card_h, 68.0);
    assert_eq!(at_150.card_h, 102.0);
    assert_eq!(at_150.pitch(), 108.0);
    assert_eq!(at_150.pad(), 12.0);
}

#[test]
fn height_for_rows_round_trips_at_each_scale() {
    for mode in [LayoutMode::Cards, LayoutMode::SingleLine, LayoutMode::Table] {
        for scale in [1.0, 1.25, 1.5, 2.0] {
            for rows in [1usize, 5, 12, 30] {
                let h = height_for_rows(rows, scale, mode) as f32;
                assert_eq!(
                    rows_in_height(h, scale, mode),
                    rows,
                    "{mode:?} scale {scale} rows {rows}: height {h} reported a different count"
                );
            }
        }
    }
}

#[test]
fn single_line_cards_are_shorter_but_still_tile() {
    let wide = Metrics::new(400.0, 400.0, 1.5);
    let compact = Metrics::new(400.0, 400.0, 1.5).with_mode(LayoutMode::SingleLine);
    assert_eq!(compact.card_h, CARD_H_SINGLE * 1.5);
    assert_eq!(compact.pitch(), (CARD_H_SINGLE + CARD_GAP) * 1.5);

    let a = compact.card_rect(0, 0.0);
    let b = compact.card_rect(1, 0.0);
    assert_eq!(b.y, a.y + compact.pitch());
    assert!(b.y >= a.y + a.h);

    // The compact card is shorter than the two-line one and the same
    // window height holds more of them...
    assert!(compact.card_h < wide.card_h);
    assert!(
        rows_in_height(400.0, 1.5, LayoutMode::SingleLine)
            > rows_in_height(400.0, 1.5, LayoutMode::Cards)
    );
    // ...and flipping the mode back restores the authored height.
    assert_eq!(compact.with_mode(LayoutMode::Cards).card_h, wide.card_h);
}

#[test]
fn table_rows_tile_flush_against_the_panel_header() {
    let table = Metrics::new(400.0, 400.0, 1.0).with_mode(LayoutMode::Table);
    assert_eq!(table.card_h, TABLE_ROW_H);
    assert_eq!(table.card_gap, 0.0);
    // The first row starts right below the panel header — there is no column
    // header row above it.
    assert_eq!(table.rows_top(), HEADER_H);
    let first = table.card_rect(0, 0.0);
    let second = table.card_rect(1, 0.0);
    assert_eq!(second.y, first.y + TABLE_ROW_H);
    // No gap between rows, so every pixel of the viewport hits a row.
    assert_eq!(
        table.row_at(table.rows_top() + TABLE_ROW_H - 0.5, 0.0, 3),
        Some(0)
    );
    assert_eq!(
        table.row_at(table.rows_top() + TABLE_ROW_H + 0.5, 0.0, 3),
        Some(1)
    );
    // The height accounts for the header, so a fitted window round-trips.
    let h = height_for_rows(4, 1.0, LayoutMode::Table) as f32;
    assert_eq!(rows_in_height(h, 1.0, LayoutMode::Table), 4);
    assert_eq!(h, HEADER_H + 4.0 * TABLE_ROW_H);
}

#[test]
fn card_rects_do_not_overlap() {
    let m = Metrics::new(400.0, 400.0, 1.5);
    let a = m.card_rect(0, 0.0);
    let b = m.card_rect(1, 0.0);
    assert_eq!(b.y, a.y + m.pitch());
    assert!(b.y >= a.y + a.h);
}

#[test]
fn row_at_finds_the_card_under_a_point() {
    let m = Metrics::new(400.0, 800.0, 1.0);
    let first = m.rows_top();
    assert_eq!(m.row_at(first + 1.0, 0.0, 5), Some(0));
    assert_eq!(m.row_at(first + m.pitch() + 1.0, 0.0, 5), Some(1));
    // Inside the inter-card gap, and above the list, nothing is hovered.
    assert_eq!(m.row_at(first + m.card_h + 2.0, 0.0, 5), None);
    assert_eq!(m.row_at(first - 4.0, 0.0, 5), None);
}

#[test]
fn scrolling_is_clamped_to_the_content() {
    let m = Metrics::new(400.0, 200.0, 1.0);
    assert_eq!(m.max_scroll(0), 0.0);
    assert!(m.max_scroll(50) > 0.0);
    // A scroll offset beyond the end must still resolve to a real row.
    let scroll = m.max_scroll(50);
    let last_visible = m.row_at(m.rows_bottom() - 2.0, scroll, 50);
    assert!(last_visible.is_some());
}

#[test]
fn physical_scale_tracks_real_density() {
    // 24" 1080p — about 92 PPI, just under the 96-DPI authoring baseline.
    let s = physical_scale(1920, 1080, 527.0, 296.0).unwrap();
    assert!((s - 0.96).abs() < 0.02, "{s}");
    // 27" 4K — about 163 PPI, so the panel grows to keep its physical size.
    let big = physical_scale(3840, 2160, 597.0, 336.0).unwrap();
    assert!(big > 1.5 && big < 1.75, "{big}");
}

#[test]
fn physical_scale_rejects_missing_or_absurd_edid() {
    // No physical dimensions reported.
    assert_eq!(physical_scale(1920, 1080, 0.0, 0.0), None);
    // A driver that synthesises 96-DPI dimensions from the pixel count
    // reports 96 PPI — the canonical value that yields scale 1.0.
    let honest = physical_scale(1920, 1080, 508.0, 285.75).unwrap();
    assert!((honest - 1.0).abs() < 1e-3, "{honest}");
    // Absurd density — a driver returning the virtual desktop as 1 mm.
    assert_eq!(physical_scale(1920, 1080, 1.0, 1.0), None);
}
