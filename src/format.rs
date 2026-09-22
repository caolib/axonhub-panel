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
        // Clock skew: AxonHub's clock is briefly ahead of ours, which shows
        // up on brand-new (often still-processing) requests. Treat the instant
        // as just now instead of jumping to a wall-clock reading.
        return "0s".into();
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
    // Convert from the parsed instant rather than the literal text: AxonHub
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

/// `YYYY-MM-DD HH:MM:SS` in local time. The detail popup shows an absolute
/// stamp, where a bare clock reading would be ambiguous about the day.
pub fn local_datetime(iso: Option<&str>) -> String {
    let local = iso
        .and_then(crate::time::parse_unix)
        .and_then(crate::time::local_parts);
    match local {
        Some(p) => format!(
            "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
            p.year, p.month, p.day, p.hour, p.minute, p.second
        ),
        None => DASH.into(),
    }
}

/// A coarse span, e.g. `45s` / `4m09s` / `1h02m`. Used for how long an upstream
/// attempt took from start to the record being written.
pub fn duration_span(seconds: i64) -> String {
    if seconds < 60 {
        format!("{seconds}s")
    } else if seconds < 3600 {
        format!("{}m{:02}s", seconds / 60, seconds % 60)
    } else {
        format!("{}h{:02}m", seconds / 3600, (seconds % 3600) / 60)
    }
}

#[cfg(test)]
#[path = "tests/format.rs"]
mod tests;
