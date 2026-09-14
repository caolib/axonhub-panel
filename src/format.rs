//! Value formatting that mirrors the AxonHub requests page.

/// Compact token count for the primary line of a card.
pub fn tokens_compact(n: i64) -> String {
    if n <= 0 {
        return DASH.into();
    }
    let f = n as f64;
    if f >= 1_000_000.0 {
        format!("{:.2}M", f / 1_000_000.0)
    } else if f >= 1000.0 {
        format!("{:.1}k", f / 1000.0)
    } else {
        n.to_string()
    }
}

pub const DASH: &str = "—";

/// Relative age such as `30s` / `2m` / `1h` / `3d`, falling back to an
/// absolute clock reading beyond a week.
///
/// Recent requests are what the panel is for, so the first minutes are shown in
/// seconds and anything older than a week shows the wall clock.
pub fn relative_time(iso: Option<&str>, now: i64) -> String {
    let Some(then) = iso.and_then(crate::time::parse_unix) else {
        return DASH.into();
    };
    let delta = now - then;
    if delta < 0 {
        // Clock skew between the panel and AxonHub; show the wall clock rather
        // than a nonsensical "in the future".
        return local_clock(iso);
    }
    let clock = local_clock(iso);
    match delta {
        0..=59 => format!("{delta}s"),
        60..=3599 => format!("{}m", delta / 60),
        3600..=86_399 => format!("{}h", delta / 3600),
        _ if delta < 7 * 86_400 => format!("{}d", delta / 86_400),
        _ => clock,
    }
}

/// `HH:MM:SS` in local time, derived from the wire's RFC3339 instant.
pub fn local_clock(iso: Option<&str>) -> String {
    // Convert from the parsed instant rather than the literal text: Octopus
    // sends `+08:00` timestamps whose date/time components are already local
    // to the gateway, and feeding those to the timezone API again would shift
    // them twice.
    let local = iso
        .and_then(crate::time::parse_unix)
        .and_then(crate::time::local_parts);
    match local {
        Some(p) => format!("{:02}:{:02}:{:02}", p.hour, p.minute, p.second),
        None => DASH.into(),
    }
}

#[cfg(test)]
mod tests {
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
    }
}
