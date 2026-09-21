//! UTC -> local wall-clock conversion via the Win32 timezone API.

use windows::Win32::Foundation::SYSTEMTIME;
use windows::Win32::System::Time::SystemTimeToTzSpecificLocalTime;

#[derive(Debug, Clone, Copy)]
pub struct SystemTimeParts {
    pub year: u16,
    pub month: u16,
    pub day: u16,
    pub hour: u16,
    pub minute: u16,
    pub second: u16,
}

impl From<SystemTimeParts> for SYSTEMTIME {
    fn from(p: SystemTimeParts) -> Self {
        SYSTEMTIME {
            wYear: p.year,
            wMonth: p.month,
            wDayOfWeek: 0,
            wDay: p.day,
            wHour: p.hour,
            wMinute: p.minute,
            wSecond: p.second,
            wMilliseconds: 0,
        }
    }
}

/// Seconds since the Unix epoch, by the system clock.
pub fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Parse an RFC3339 instant into Unix seconds. Fractional seconds are ignored
/// and the offset is honoured, which is all the relative-time display needs.
pub fn parse_unix(iso: &str) -> Option<i64> {
    let bytes = iso.as_bytes();
    if bytes.len() < 19 {
        return None;
    }
    let num = |from: usize, to: usize| -> Option<i64> { iso.get(from..to)?.parse::<i64>().ok() };

    let year = num(0, 4)?;
    let month = num(5, 7)?;
    let day = num(8, 10)?;
    let hour = num(11, 13)?;
    let minute = num(14, 16)?;
    let second = num(17, 19)?;
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }

    let days = days_from_civil(year, month, day);
    let mut seconds = days * 86_400 + hour * 3600 + minute * 60 + second;

    // Trailing offset: `Z`, `+HH:MM` or `-HH:MM`.
    let rest = &iso[19..];
    if let Some(offset) = rest.find(['+', '-']) {
        let sign = if rest.as_bytes()[offset] == b'-' {
            -1
        } else {
            1
        };
        let tail = &rest[offset + 1..];
        if tail.len() >= 5 {
            if let (Ok(h), Ok(m)) = (tail[0..2].parse::<i64>(), tail[3..5].parse::<i64>()) {
                seconds -= sign * (h * 3600 + m * 60);
            }
        }
    }
    Some(seconds)
}

/// Days between 1970-01-01 and the given civil date (Howard Hinnant's algorithm).
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Civil date from days since the Unix epoch (Howard Hinnant's algorithm).
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    (if month <= 2 { y + 1 } else { y }, month, day)
}

/// UTC wall-clock parts for a Unix instant.
pub fn utc_parts(unix: i64) -> SystemTimeParts {
    let days = unix.div_euclid(86_400);
    let secs = unix.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    SystemTimeParts {
        year: year as u16,
        month: month as u16,
        day: day as u16,
        hour: (secs / 3600) as u16,
        minute: ((secs % 3600) / 60) as u16,
        second: (secs % 60) as u16,
    }
}

/// Local wall-clock parts for a Unix instant, via the Win32 timezone API.
pub fn local_parts(unix: i64) -> Option<SystemTimeParts> {
    to_local(utc_parts(unix))
}

pub fn to_local(utc: SystemTimeParts) -> Option<SystemTimeParts> {
    unsafe {
        let input: SYSTEMTIME = utc.into();
        let mut out = SYSTEMTIME::default();
        SystemTimeToTzSpecificLocalTime(None, &input, &mut out).ok()?;
        Some(SystemTimeParts {
            year: out.wYear,
            month: out.wMonth,
            day: out.wDay,
            hour: out.wHour,
            minute: out.wMinute,
            second: out.wSecond,
        })
    }
}

#[cfg(test)]
#[path = "tests/time.rs"]
mod tests;
