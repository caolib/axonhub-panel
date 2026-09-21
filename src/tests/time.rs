use super::*;

#[test]
fn parses_an_axonhub_timestamp() {
    // 2026-09-12T02:20:57.7544744Z
    assert_eq!(
        parse_unix("2026-09-12T02:20:57.7544744Z"),
        Some(1_789_179_657)
    );
}

#[test]
fn honours_the_utc_offset() {
    assert_eq!(
        parse_unix("2026-09-12T10:20:57+08:00"),
        parse_unix("2026-09-12T02:20:57Z")
    );
}

#[test]
fn rejects_malformed_timestamps() {
    assert_eq!(parse_unix(""), None);
    assert_eq!(parse_unix("not-a-date"), None);
    assert_eq!(parse_unix("2026-13-12T02:20:57Z"), None);
}

#[test]
fn utc_parts_inverts_days_from_civil() {
    let parts = utc_parts(0);
    assert_eq!(
        (parts.year, parts.month, parts.day, parts.hour),
        (1970, 1, 1, 0)
    );
    let parts = utc_parts(1_789_179_657);
    assert_eq!((parts.year, parts.month, parts.day), (2026, 9, 12));
    assert_eq!((parts.hour, parts.minute, parts.second), (2, 20, 57));
    // Round trip a leap day.
    let leap = parse_unix("2024-02-29T23:59:59Z").unwrap();
    let parts = utc_parts(leap);
    assert_eq!(
        (parts.year, parts.month, parts.day, parts.hour),
        (2024, 2, 29, 23)
    );
}

#[test]
fn local_clock_is_derived_from_the_instant_not_the_wall_text() {
    // The same instant written with an offset must format identically to
    // its UTC spelling, which is what `+08:00` AxonHub timestamps need.
    let utc = local_parts(parse_unix("2026-09-12T02:20:57Z").unwrap());
    let offset = local_parts(parse_unix("2026-09-12T10:20:57+08:00").unwrap());
    assert_eq!(
        utc.map(|p| (p.hour, p.minute, p.second)),
        offset.map(|p| (p.hour, p.minute, p.second))
    );
}
