use super::*;

const BASE: i64 = 1_789_179_657; // 2026-09-12T02:20:57Z

fn at(offset: i64) -> String {
    relative_time(Some("2026-09-12T02:20:57Z"), BASE + offset)
}

#[test]
fn buckets_recent_ages() {
    assert_eq!(at(0), "0s");
    assert_eq!(at(4), "4s");
    assert_eq!(at(5), "5s");
    assert_eq!(at(59), "59s");
    assert_eq!(at(60), "1m");
    assert_eq!(at(3599), "59m");
    assert_eq!(at(3600), "1h");
    assert_eq!(at(86_399), "23h");
    assert_eq!(at(86_400), "1d");
}

#[test]
fn switches_to_a_date_beyond_a_week() {
    // A week out, an absolute clock reading is easier to read than "7d".
    assert_eq!(at(7 * 86_400), local_clock(Some("2026-09-12T02:20:57Z")));
    assert_eq!(at(6 * 86_400), "6d");
}

#[test]
fn clock_skew_does_not_produce_a_future_age() {
    // AxonHub briefly ahead of the panel: fall back to the wall clock.
    assert_eq!(at(-30), local_clock(Some("2026-09-12T02:20:57Z")));
}

#[test]
fn missing_timestamp_is_a_dash() {
    assert_eq!(relative_time(None, BASE), DASH);
    assert_eq!(relative_time(Some("garbage"), BASE), DASH);
    assert_eq!(local_datetime(None), DASH);
}

#[test]
fn spans_read_at_three_scales() {
    assert_eq!(duration_span(45), "45s");
    assert_eq!(duration_span(249), "4m09s");
    assert_eq!(duration_span(3725), "1h02m");
}
