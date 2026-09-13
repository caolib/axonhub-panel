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

/// Relative age such as `just now` / `30s ago` / `2 min ago` / `1 hr ago`.
///
/// Recent requests are what the panel is for, so the first minutes are shown in
/// seconds and anything older than a week falls back to an absolute date.
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
        0..=4 => "just now".into(),
        5..=59 => format!("{delta}s"),
        60..=3599 => format!("{} min", delta / 60),
        3600..=86_399 => format!("{} hr", delta / 3600),
        86_400..=172_799 => format!("yesterday {clock}"),
        _ if delta < 7 * 86_400 => format!("{} days", delta / 86_400),
        _ => clock,
    }
}

/// `HH:MM:SS` in local time, derived from the wire's RFC3339 UTC instant.
pub fn local_clock(iso: Option<&str>) -> String {
    iso.and_then(crate::format::parse_local_hms)
        .unwrap_or_else(|| DASH.into())
}

/// Parse an RFC3339 instant and convert it to wall-clock time for this machine.
fn parse_local_hms(iso: &str) -> Option<String> {
    let (date, rest) = iso.split_once('T')?;
    let mut d = date.split('-');
    let year: u16 = d.next()?.parse().ok()?;
    let month: u16 = d.next()?.parse().ok()?;
    let day: u16 = d.next()?.parse().ok()?;

    let mut t = rest.split(':');
    let hour: u16 = t.next()?.parse().ok()?;
    let minute: u16 = t.next()?.parse().ok()?;
    let second: u16 = t.next().map(|s| s.get(..2).unwrap_or(s))?.parse().ok()?;

    let utc = super::time::SystemTimeParts {
        year,
        month,
        day,
        hour,
        minute,
        second,
    };
    let local = super::time::to_local(utc).unwrap_or(utc);
    Some(format!(
        "{:02}:{:02}:{:02}",
        local.hour, local.minute, local.second
    ))
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
        assert_eq!(at(0), "just now");
        assert_eq!(at(4), "just now");
        assert_eq!(at(5), "5s");
        assert_eq!(at(59), "59s");
        assert_eq!(at(60), "1 min");
        assert_eq!(at(3599), "59 min");
        assert_eq!(at(3600), "1 hr");
        assert_eq!(at(86_399), "23 hr");
    }

    #[test]
    fn switches_to_a_date_beyond_a_week() {
        // A week out, an absolute clock reading is easier to read than "7 days".
        assert_eq!(at(7 * 86_400), local_clock(Some("2026-09-12T02:20:57Z")));
        assert!(at(6 * 86_400).ends_with("days"));
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
