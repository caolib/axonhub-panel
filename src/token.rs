//! JWT inspection.
//!
//! The panel only needs one thing from a token: when it expires. AxonHub issues
//! a 7-day token with no refresh flow, so expiry is a normal event rather than
//! an error — knowing it up front lets the panel say so plainly and skip a
//! request that is certain to fail.

/// Expiry as a Unix timestamp, if the token is a well-formed JWT carrying `exp`.
pub fn expiry(token: &str) -> Option<i64> {
    let payload = token.split('.').nth(1)?;
    let decoded = base64url_decode(payload)?;
    let text = String::from_utf8(decoded).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    value.get("exp")?.as_i64()
}

/// Whether the token has expired, using `now` as Unix seconds.
pub fn is_expired(token: &str, now: i64) -> bool {
    match expiry(token) {
        Some(exp) => exp <= now,
        // An unreadable token is not necessarily invalid; let the server decide.
        None => false,
    }
}

/// Coarse remaining validity, for a status line.
pub fn describe(token: &str, now: i64) -> String {
    let Some(exp) = expiry(token) else {
        return "无法解析有效期".into();
    };
    let remaining = exp - now;
    if remaining <= 0 {
        return "已过期".into();
    }
    let days = remaining / 86_400;
    let hours = (remaining % 86_400) / 3600;
    if days > 0 {
        format!("剩余 {days} 天 {hours} 小时")
    } else {
        let minutes = (remaining % 3600) / 60;
        format!("剩余 {hours} 小时 {minutes} 分钟")
    }
}

/// Cheap shape check so the form can reject obvious mistakes before a request.
pub fn looks_like_jwt(token: &str) -> bool {
    let parts: Vec<&str> = token.split('.').collect();
    parts.len() == 3 && parts.iter().all(|p| !p.is_empty())
}

/// Base64url without padding, as used by JWT segments.
fn base64url_decode(input: &str) -> Option<Vec<u8>> {
    fn value(byte: u8) -> Option<u8> {
        match byte {
            b'A'..=b'Z' => Some(byte - b'A'),
            b'a'..=b'z' => Some(byte - b'a' + 26),
            b'0'..=b'9' => Some(byte - b'0' + 52),
            // Base64url uses `-`/`_` only; the classic `+`/`/` are rejected so a
            // mistyped standard-base64 segment cannot silently decode.
            b'-' => Some(62),
            b'_' => Some(63),
            _ => None,
        }
    }

    let mut out = Vec::with_capacity(input.len() * 3 / 4);
    let mut buffer = 0u32;
    let mut bits = 0u32;

    for byte in input.bytes() {
        if byte == b'=' {
            break;
        }
        let v = value(byte)? as u32;
        buffer = (buffer << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buffer >> bits) as u8);
        }
    }
    Some(out)
}

#[cfg(test)]
#[path = "tests/token.rs"]
mod tests;
