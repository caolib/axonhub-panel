//! Value formatting that mirrors the AxonHub requests page.

/// Duration rule lifted from the frontend `formatDuration`.
pub fn duration(ms: Option<i64>) -> String {
    let Some(ms) = ms else { return DASH.into() };
    if ms <= 0 {
        return DASH.into();
    }
    let f = ms as f64;
    if f < 1.0 {
        format!("{f:.3}ms")
    } else if f < 1000.0 {
        format!("{f:.0}ms")
    } else if f < 60_000.0 {
        format!("{:.1}s", f / 1000.0)
    } else {
        format!("{:.1}m", f / 60_000.0)
    }
}

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


/// Cost with the fixed six decimals the requests page requests from `Intl`.
pub fn cost(v: Option<f64>) -> String {
    match v {
        Some(v) => format!("${v:.6}"),
        None => DASH.into(),
    }
}

pub const DASH: &str = "—";

/// Relative age such as `刚刚` / `3 分钟前` / `2 小时前` / `昨天 11:20` / `3 天前`.
///
/// Recent requests are what the panel is for, so the first minutes are shown in
/// seconds and anything older than a week falls back to an absolute date, where
/// "12 天前" stops being easier to read than the date itself.
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
        0..=4 => "刚刚".into(),
        5..=59 => format!("{delta} 秒前"),
        60..=3599 => format!("{} 分钟前", delta / 60),
        3600..=86_399 => format!("{} 小时前", delta / 3600),
        86_400..=172_799 => format!("昨天 {clock}"),
        _ if delta < 7 * 86_400 => format!("{} 天前", delta / 86_400),
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
    Some(format!("{:02}:{:02}:{:02}", local.hour, local.minute, local.second))
}

/// Short protocol label used by the requests page (`API_FORMAT_LABELS`).
pub fn format_label(raw: &str) -> &str {
    match raw {
        "openai/chat_completions" => "chat",
        "openai/responses" => "responses",
        "anthropic/messages" => "messages",
        other => other,
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
        assert_eq!(at(0), "刚刚");
        assert_eq!(at(4), "刚刚");
        assert_eq!(at(5), "5 秒前");
        assert_eq!(at(59), "59 秒前");
        assert_eq!(at(60), "1 分钟前");
        assert_eq!(at(3599), "59 分钟前");
        assert_eq!(at(3600), "1 小时前");
        assert_eq!(at(86_399), "23 小时前");
    }

    #[test]
    fn switches_to_a_date_beyond_a_week() {
        // A week out, an absolute clock reading is easier to read than "7 天前".
        assert_eq!(at(7 * 86_400), local_clock(Some("2026-09-12T02:20:57Z")));
        assert!(at(6 * 86_400).ends_with("天前"));
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

    #[test]
    fn duration_matches_the_requests_page_rules() {
        assert_eq!(duration(Some(0)), DASH);
        assert_eq!(duration(None), DASH);
        assert_eq!(duration(Some(500)), "500ms");
        assert_eq!(duration(Some(3361)), "3.4s");
        // The minute boundary is where the unit changes.
        assert_eq!(duration(Some(59_999)), "60.0s");
        assert_eq!(duration(Some(60_000)), "1.0m");
        assert_eq!(duration(Some(162_289)), "2.7m");
    }

    #[test]
    fn cost_uses_six_decimals() {
        assert_eq!(cost(Some(0.00548008)), "$0.005480");
        assert_eq!(cost(None), DASH);
    }
}
