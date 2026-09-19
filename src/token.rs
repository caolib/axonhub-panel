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
mod tests {
    use super::*;

    /// A token with the same shape as one AxonHub issues, but the signature is
    /// a placeholder: never paste a real token here, a JWT is a live credential
    /// and this file is version controlled.
    const SAMPLE: &str = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.\
eyJleHAiOjE3ODkxODMzMDIsInVzZXJfaWQiOjF9.\
c2lnbmF0dXJlLXBsYWNlaG9sZGVyLW5vdC1hLXJlYWwta2V5";

    #[test]
    fn reads_expiry_from_a_real_axonhub_token() {
        assert_eq!(expiry(SAMPLE), Some(1_789_183_302));
    }

    #[test]
    fn detects_expiry_against_a_clock() {
        assert!(!is_expired(SAMPLE, 1_789_183_301));
        assert!(is_expired(SAMPLE, 1_789_183_302));
        assert!(is_expired(SAMPLE, 1_789_183_303));
    }

    #[test]
    fn rejects_malformed_input_without_panicking() {
        assert_eq!(expiry(""), None);
        assert_eq!(expiry("not-a-jwt"), None);
        assert_eq!(expiry("a.b"), None);
        assert_eq!(expiry("!!!.!!!.!!!"), None);
        // Not expired-by-default, so the server remains the authority.
        assert!(!is_expired("garbage", 1));
    }

    #[test]
    fn rejects_classic_base64_alphabet() {
        // JWT segments use base64url (`-`/`_`); the classic `+`/`/` must not
        // decode as if valid, or a mistyped token could be misread.
        assert_eq!(base64url_decode("a+b"), None);
        assert_eq!(base64url_decode("a/b"), None);
    }

    #[test]
    fn shape_check_matches_jwt_layout() {
        assert!(looks_like_jwt(SAMPLE));
        assert!(!looks_like_jwt("abc"));
        assert!(!looks_like_jwt("a..b"));
        assert!(!looks_like_jwt(""));
    }
}
