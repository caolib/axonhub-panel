use super::*;

/// A synthetic token with the same shape as one AxonHub issues: valid
/// base64url segments carrying `exp`, but a stub signature. Never paste a
/// real token here — a JWT is a live credential and this file is version
/// controlled.
const SAMPLE: &str =
    "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJleHAiOjE3ODkxODMzMDJ9.bm90LWEtcmVhbC1zaWduYXR1cmU";

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
