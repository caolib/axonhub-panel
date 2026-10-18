use super::*;
use crate::client::{SignInResponse, UserInfo};

fn user() -> UserInfo {
    UserInfo {
        projects: Vec::new(),
    }
}

fn response(token: &str) -> SignInResponse {
    SignInResponse {
        token: token.into(),
        user: user(),
    }
}

fn creds() -> Credentials {
    Credentials {
        email: "a@b".into(),
        password: "secret".into(),
    }
}

/// A JWT-shaped token with a real `exp`, signed with a stub signature.
fn token(exp: i64) -> String {
    fn encode(bytes: &[u8]) -> String {
        const ALPHABET: &[u8; 64] =
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
        let mut out = String::new();
        for chunk in bytes.chunks(3) {
            let b = [
                chunk[0],
                chunk.get(1).copied().unwrap_or(0),
                chunk.get(2).copied().unwrap_or(0),
            ];
            let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
            // JWT segments are unpadded: a 1- or 2-byte tail yields 2 or 3
            // characters, and the zero padding must not be emitted.
            let chars = chunk.len() + 1;
            for shift in [18, 12, 6, 0].into_iter().take(chars) {
                out.push(ALPHABET[((n >> shift) & 63) as usize] as char);
            }
        }
        out
    }
    format!(
        "e30.{}.c3R1Yg",
        encode(format!("{{\"exp\":{exp}}}").as_bytes())
    )
}

fn target(token_value: Option<String>, with_password: bool) -> Target {
    Target {
        id: "a1".into(),
        name: "A".into(),
        endpoint: "http://localhost:8090".into(),
        project_id: "p1".into(),
        token: token_value,
        credentials: with_password.then(creds),
        allow_password: with_password,
        auth_blocked: false,
        auth_retry_at: None,
        auth_backoff: Duration::ZERO,
        auth_error: None,
    }
}

fn channel() -> (Sender<Update>, Receiver<Update>) {
    std::sync::mpsc::channel()
}

#[test]
fn a_valid_token_is_used_without_signing_in() {
    let (tx, rx) = channel();
    let mut target = target(Some(token(2_000_000_000)), true);
    let mut signins = 0;
    let result = target.authorized(
        |_| {
            signins += 1;
            Ok(response(&token(2_000_000_000)))
        },
        |_, used| {
            assert_eq!(used, token(2_000_000_000));
            Ok(7)
        },
        &tx,
        1_700_000_000,
        Instant::now(),
    );
    assert_eq!(result.unwrap(), 7);
    assert_eq!(signins, 0);
    assert!(
        rx.try_recv().is_err(),
        "nothing to report when nothing changed"
    );
}

#[test]
fn an_expired_token_is_renewed_before_the_query() {
    let (tx, rx) = channel();
    let mut target = target(Some(token(100)), true);
    let mut signins = 0;
    let result = target.authorized(
        |_| {
            signins += 1;
            Ok(response(&token(2_000_000_000)))
        },
        |_, used| {
            assert_eq!(used, token(2_000_000_000), "query uses the fresh token");
            Ok(9)
        },
        &tx,
        1_000,
        Instant::now(),
    );
    assert_eq!(result.unwrap(), 9);
    assert_eq!(signins, 1);
    assert!(matches!(
        rx.try_recv(),
        Ok(Update::TokenRefreshed { account, .. }) if account == "a1"
    ));
}

#[test]
fn a_rejected_token_is_renewed_and_the_query_retried_once() {
    let (tx, _rx) = channel();
    let mut target = target(Some("first".into()), true);
    let mut calls = 0;
    let result = target.authorized(
        |_| Ok(response("second")),
        |_, used| {
            calls += 1;
            if used == "first" {
                Err(ApiError::Expired)
            } else {
                Ok(5)
            }
        },
        &tx,
        1_700_000_000,
        Instant::now(),
    );
    assert_eq!(result.unwrap(), 5);
    assert_eq!(calls, 2);
}

#[test]
fn without_credentials_or_a_token_the_account_cannot_authenticate() {
    let (tx, _rx) = channel();
    let mut target = target(None, false);
    assert!(!target.can_authenticate());
    let result = target.authorized(
        |_| panic!("must not sign in without a password"),
        |_, _| Ok(1),
        &tx,
        1_700_000_000,
        Instant::now(),
    );
    assert!(matches!(result, Err(ApiError::Unauthorized)));
}

#[test]
fn a_rejected_password_blocks_further_attempts() {
    let (tx, _rx) = channel();
    let mut target = target(None, true);
    let mut signins = 0;
    let mut signin = |_: &Credentials| {
        signins += 1;
        Err(ApiError::Unauthorized)
    };
    let mut query = |_: &Target, _: &str| Ok(1);
    let now = Instant::now();

    assert!(
        target
            .authorized(&mut signin, &mut query, &tx, 1_000, now)
            .is_err()
    );
    assert!(target.auth_blocked);
    // Even long after the backoff, a rejected password is not retried.
    assert!(
        target
            .authorized(
                &mut signin,
                &mut query,
                &tx,
                1_000,
                now + Duration::from_secs(600)
            )
            .is_err()
    );
    assert_eq!(signins, 1);
}

#[test]
fn a_transient_signin_failure_is_retried_after_a_backoff() {
    let (tx, _rx) = channel();
    let mut target = target(None, true);
    let signins = std::cell::Cell::new(0);
    let now = Instant::now();
    let mut signin = |_: &Credentials| {
        signins.set(signins.get() + 1);
        if signins.get() == 1 {
            Err(ApiError::Unreachable("down".into()))
        } else {
            Ok(response("fresh"))
        }
    };
    let mut query = |_: &Target, _: &str| Ok(1);

    assert!(
        target
            .authorized(&mut signin, &mut query, &tx, 1_000, now)
            .is_err()
    );
    assert!(
        !target.auth_blocked,
        "a network error is not a bad password"
    );
    // Inside the backoff window the worker does not hammer the endpoint.
    assert!(
        target
            .authorized(
                &mut signin,
                &mut query,
                &tx,
                1_000,
                now + Duration::from_secs(1)
            )
            .is_err()
    );
    assert_eq!(signins.get(), 1);
    // After it, the next attempt succeeds and clears the backoff.
    assert_eq!(
        target
            .authorized(
                &mut signin,
                &mut query,
                &tx,
                1_000,
                now + Duration::from_secs(3)
            )
            .unwrap(),
        1
    );
    assert_eq!(signins.get(), 2);
    assert!(target.auth_backoff.is_zero());
}

#[test]
fn a_query_rejected_after_a_fresh_signin_blocks_the_account() {
    let (tx, _rx) = channel();
    let mut target = target(Some(token(100)), true);
    let mut signins = 0;
    let result = target.authorized(
        |_| {
            signins += 1;
            Ok(response("still-bad"))
        },
        |_, _| Err::<i64, _>(ApiError::Expired),
        &tx,
        1_000,
        Instant::now(),
    );
    assert!(result.is_err());
    assert_eq!(signins, 1, "a fresh token is tried, then the account stops");
    assert!(target.auth_blocked);
    assert_eq!(target.token, None);
}

#[test]
fn token_mode_never_uses_a_stored_password() {
    let account = Account {
        id: "a1".into(),
        name: "A".into(),
        endpoint: "http://localhost:8090".into(),
        project_id: "p1".into(),
    };
    let mut stored = Stored::default();
    stored.set_token("a1", "tok");
    stored.set_credentials("a1", &creds());

    let password_mode = Target::from_account(&account, &stored, CredentialMode::Password);
    assert_eq!(
        password_mode.credentials.as_ref().unwrap().password,
        "secret"
    );
    assert!(password_mode.can_authenticate());

    let token_mode = Target::from_account(&account, &stored, CredentialMode::Token);
    assert!(token_mode.credentials.is_none());
    assert!(!token_mode.allow_password);

    let none_mode = Target::from_account(&account, &stored, CredentialMode::None);
    assert!(none_mode.credentials.is_none());
}

#[test]
fn a_downgrade_drops_the_in_memory_password() {
    let mut target = target(None, true);
    assert!(target.can_authenticate());
    target.set_allow_password(false);
    assert!(target.credentials.is_none());
    assert!(!target.can_authenticate());
}

#[test]
fn backoff_doubles_up_to_the_cap() {
    let mut wait = Duration::ZERO;
    let mut seen = Vec::new();
    for _ in 0..6 {
        wait = next_backoff(wait);
        seen.push(wait.as_secs());
    }
    assert_eq!(seen, vec![2, 4, 8, 16, 30, 30]);
}
