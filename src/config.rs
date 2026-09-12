//! On-disk settings plus a DPAPI-protected credential store.
//!
//! Three credential modes, all stored under DPAPI (current Windows user):
//!
//! * `None`     — nothing is written; the password is asked for on every launch.
//! * `Token`    — only the JWT is stored, never the password. The token is valid
//!                for the lifetime AxonHub issued it for (7 days), after which
//!                the sign-in form appears again.
//! * `Password` — the email and password are stored so sign-in is silent and
//!                never expires. Convenient, but the password is on disk.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CredentialMode {
    None,
    Token,
    Password,
}

impl CredentialMode {
    pub fn label(self) -> &'static str {
        match self {
            CredentialMode::None => "不保存",
            CredentialMode::Token => "保存令牌",
            CredentialMode::Password => "保存密码",
        }
    }

    /// One-line explanation shown under the selector.
    pub fn hint(self) -> &'static str {
        match self {
            CredentialMode::None => "每次启动都需要输入密码",
            CredentialMode::Token => "仅加密保存访问令牌,不保存密码;7 天后需重新登录",
            CredentialMode::Password => "加密保存邮箱和密码,自动登录,不会过期",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Config {
    pub endpoint: String,
    pub project_id: String,
    /// Poll cadence while every visible request is finished.
    pub poll_seconds: u64,
    /// Faster cadence used while at least one request is still in flight.
    pub active_poll_seconds: u64,
    pub row_limit: i64,
    pub credential_mode: CredentialMode,
    /// Environment variable read for a token instead of using the sign-in form.
    /// Empty disables it. Lets the token stay out of the config file entirely.
    pub token_env_var: String,
    pub always_on_top: bool,
    pub window: WindowState,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowState {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

/// Rows the panel shows by default. Twelve cards plus chrome is about 567px,
/// which fits a 1080p work area with room to spare.
pub const DEFAULT_ROWS: usize = 12;

impl Default for WindowState {
    fn default() -> Self {
        WindowState {
            // Negative coordinates mean "not positioned yet"; the window is
            // then parked at the top-right of the work area on first launch.
            x: -1,
            y: -1,
            width: 452,
            height: crate::ui::layout::height_for_rows(DEFAULT_ROWS, 1.0),
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Config {
            endpoint: "http://localhost:8090".into(),
            project_id: "gid://axonhub/Project/1".into(),
            poll_seconds: 5,
            active_poll_seconds: 2,
            row_limit: DEFAULT_ROWS as i64,
            credential_mode: CredentialMode::Token,
            token_env_var: "AXONHUB_ACCESS_TOKEN".into(),
            always_on_top: true,
            window: WindowState::default(),
        }
    }
}

/// What is written to the encrypted blob. Which fields are populated depends on
/// the configured mode; unused fields are simply absent.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Stored {
    /// Kept in every mode that stores anything, so the sign-in form can be
    /// prefilled after the token expires.
    pub email: String,
    pub password: Option<String>,
    pub token: Option<String>,
}

impl Stored {
    pub fn credentials(&self) -> Option<Credentials> {
        Some(Credentials {
            email: self.email.clone(),
            password: self.password.clone()?,
        })
    }
}

/// A sign-in attempt: email plus password.
#[derive(Debug, Clone)]
pub struct Credentials {
    pub email: String,
    pub password: String,
}

/// Directory holding the config and the encrypted credential blob.
///
/// `AH_PANEL_HOME` overrides it, for pointing the panel at a different profile.
/// Defaults to `%APPDATA%\ah-panel`.
fn base_dir() -> PathBuf {
    if let Some(dir) = std::env::var("AH_PANEL_HOME").ok().filter(|s| !s.is_empty()) {
        return PathBuf::from(dir);
    }
    let root = std::env::var("APPDATA")
        .ok()
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."));
    root.join("ah-panel")
}

pub fn config_path() -> PathBuf {
    base_dir().join("config.json")
}

fn credentials_path_in(dir: &Path) -> PathBuf {
    dir.join("credentials.bin")
}

impl Config {
    pub fn load() -> Self {
        std::fs::read_to_string(config_path())
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) {
        let dir = base_dir();
        if std::fs::create_dir_all(&dir).is_err() {
            return;
        }
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(config_path(), json);
        }
    }

    pub fn graphql_url(&self) -> String {
        format!("{}/admin/graphql", self.endpoint.trim_end_matches('/'))
    }

    pub fn signin_url(&self) -> String {
        format!("{}/admin/auth/signin", self.endpoint.trim_end_matches('/'))
    }

    /// Persist exactly what the current mode allows, dropping anything else so a
    /// mode change cannot leave stale secrets behind.
    pub fn store(&self, stored: &Stored) {
        store_in(&base_dir(), self.credential_mode, stored);
    }
}

/// What a given mode is permitted to write, if anything.
fn filtered(mode: CredentialMode, stored: &Stored) -> Option<Stored> {
    match mode {
        CredentialMode::None => None,
        CredentialMode::Token => Some(Stored {
            email: stored.email.clone(),
            password: None,
            token: stored.token.clone(),
        }),
        CredentialMode::Password => Some(stored.clone()),
    }
}

/// Write the record a mode allows into `dir`, or clear what is there.
///
/// Takes the directory explicitly rather than reading the environment, so tests
/// can target a scratch directory without racing each other.
fn store_in(dir: &Path, mode: CredentialMode, stored: &Stored) {
    let Some(record) = filtered(mode, stored) else {
        clear_stored_in(dir);
        return;
    };
    if record.token.is_none() && record.password.is_none() {
        // Nothing worth keeping; do not leave an empty blob around.
        clear_stored_in(dir);
        return;
    }
    let Ok(plain) = serde_json::to_vec(&record) else {
        return;
    };
    let Some(blob) = dpapi::protect(&plain) else {
        return;
    };
    if std::fs::create_dir_all(dir).is_err() {
        return;
    }
    let _ = std::fs::write(credentials_path_in(dir), blob);
}

/// Read a token from the configured environment variable, if it is set and
/// non-empty. Checked before the stored blob so an environment override always
/// wins, which is what a user exporting a fresh token expects.
pub fn token_from_env(var: &str) -> Option<String> {
    let name = var.trim();
    if name.is_empty() {
        return None;
    }
    let value = std::env::var(name).ok()?;
    let trimmed = value.trim().trim_start_matches("Bearer ").trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// Decrypt the stored record. `None` covers "nothing stored" and "unreadable",
/// which are equivalent from the caller's point of view.
pub fn load_stored() -> Option<Stored> {
    read_stored_in(&base_dir())
}

fn read_stored_in(dir: &Path) -> Option<Stored> {
    let blob = std::fs::read(credentials_path_in(dir)).ok()?;
    let plain = dpapi::unprotect(&blob)?;
    serde_json::from_slice(&plain).ok()
}

/// Forget the stored credentials. Deliberately does not touch `config.json`:
/// clearing a credential must not reset the endpoint, window or poll settings.
pub fn clear_stored() {
    clear_stored_in(&base_dir());
}

fn clear_stored_in(dir: &Path) {
    let _ = std::fs::remove_file(credentials_path_in(dir));
}

/// Bring the window back on screen if a display change orphaned it.
pub fn clamp_to_virtual_screen(state: WindowState, screen: (i32, i32, i32, i32)) -> WindowState {
    let (left, top, right, bottom) = screen;
    let width = state.width.clamp(320, (right - left).max(320));
    let height = state.height.clamp(280, (bottom - top).max(280));
    let x = state.x.clamp(left - 8, (right - width).max(left));
    let y = state.y.clamp(top, (bottom - height).max(top));
    WindowState {
        x,
        y,
        width,
        height,
    }
}

/// Minimal DPAPI wrapper: current-user scope, no UI prompts.
mod dpapi {
    use windows::Win32::Foundation::{HLOCAL, LocalFree};
    use windows::Win32::Security::Cryptography::{
        CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData,
    };
    use windows::core::PCWSTR;

    fn blob(data: &[u8]) -> CRYPT_INTEGER_BLOB {
        CRYPT_INTEGER_BLOB {
            cbData: data.len() as u32,
            pbData: data.as_ptr() as *mut u8,
        }
    }

    fn take(out: CRYPT_INTEGER_BLOB) -> Option<Vec<u8>> {
        if out.pbData.is_null() {
            return None;
        }
        let bytes = unsafe { std::slice::from_raw_parts(out.pbData, out.cbData as usize).to_vec() };
        unsafe {
            let _ = LocalFree(Some(HLOCAL(out.pbData as *mut _)));
        }
        Some(bytes)
    }

    pub fn protect(plain: &[u8]) -> Option<Vec<u8>> {
        unsafe {
            let input = blob(plain);
            let mut out = CRYPT_INTEGER_BLOB::default();
            CryptProtectData(
                &input,
                PCWSTR::null(),
                None,
                None,
                None,
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut out,
            )
            .ok()?;
            take(out)
        }
    }

    pub fn unprotect(blob_bytes: &[u8]) -> Option<Vec<u8>> {
        unsafe {
            let input = blob(blob_bytes);
            let mut out = CRYPT_INTEGER_BLOB::default();
            CryptUnprotectData(
                &input,
                None,
                None,
                None,
                None,
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut out,
            )
            .ok()?;
            take(out)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A scratch directory per test, so nothing touches a real profile and the
    /// tests can run in parallel without sharing process state.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "ah-panel-test-{tag}-{}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Scratch(dir)
        }

        fn store(&self, mode: CredentialMode, stored: &Stored) {
            store_in(&self.0, mode, stored);
        }

        fn load(&self) -> Option<Stored> {
            read_stored_in(&self.0)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn sample() -> Stored {
        Stored {
            email: "a@b".into(),
            password: Some("secret".into()),
            token: Some("tok".into()),
        }
    }

    #[test]
    fn token_mode_keeps_the_token_and_drops_the_password() {
        let dir = Scratch::new("token");
        dir.store(CredentialMode::Token, &sample());

        let got = dir.load().expect("token should have been stored");
        assert_eq!(got.token.as_deref(), Some("tok"));
        assert_eq!(got.password, None, "token mode must not write a password");
    }

    #[test]
    fn password_mode_keeps_both() {
        let dir = Scratch::new("password");
        dir.store(CredentialMode::Password, &sample());

        let got = dir.load().expect("credentials should have been stored");
        assert_eq!(got.token.as_deref(), Some("tok"));
        assert_eq!(got.password.as_deref(), Some("secret"));
    }

    #[test]
    fn none_mode_writes_nothing_and_clears_what_was_there() {
        let dir = Scratch::new("none");
        dir.store(CredentialMode::Token, &sample());
        assert!(dir.load().is_some());

        dir.store(CredentialMode::None, &sample());
        assert!(dir.load().is_none(), "none mode must clear what was stored");
    }

    #[test]
    fn switching_mode_purges_what_it_may_no_longer_keep() {
        let dir = Scratch::new("downgrade");
        dir.store(CredentialMode::Password, &sample());
        assert!(dir.load().unwrap().password.is_some());

        // Downgrading must not leave the old password behind.
        let current = dir.load().unwrap();
        dir.store(CredentialMode::Token, &current);
        assert_eq!(dir.load().unwrap().password, None);
    }

    #[test]
    fn a_token_mode_without_a_token_does_not_leave_a_stale_blob() {
        let dir = Scratch::new("empty");
        dir.store(CredentialMode::Password, &sample());
        assert!(dir.load().is_some());

        // Nothing worth keeping in token mode: the old blob must go, not linger.
        dir.store(
            CredentialMode::Token,
            &Stored {
                email: String::new(),
                password: None,
                token: None,
            },
        );
        assert!(dir.load().is_none());
    }

    #[test]
    fn config_round_trips_through_disk() {
        // Config::save/load use the process-wide directory, so drive the serde
        // layer directly here to keep the test isolated.
        let mut config = Config::default();
        config.endpoint = "http://example:1234".into();
        config.row_limit = 7;
        config.credential_mode = CredentialMode::Password;

        let json = serde_json::to_string(&config).unwrap();
        let back: Config = serde_json::from_str(&json).unwrap();
        assert_eq!(back.endpoint, "http://example:1234");
        assert_eq!(back.row_limit, 7);
        assert_eq!(back.credential_mode, CredentialMode::Password);
    }
}
