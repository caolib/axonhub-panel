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
use tracing::warn;

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
            CredentialMode::None => "每次启动都需要重新输入访问令牌",
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
    pub pin_position: bool,
    pub always_on_bottom: bool,
    /// Whether `window.width`/`height` are logical (96-DPI) units. Configs
    /// written before this existed stored device pixels, so an older file has
    /// this false and its size is converted once, on load, against the scale of
    /// the monitor it was saved on.
    pub logical_window: bool,
    /// Body font size in pixels at the 96-DPI baseline (the authored value is
    /// 12.5). The whole panel — fonts, cards and the window itself — scales in
    /// proportion with it, so this is the single knob for how large the text
    /// reads. Adjustable from the settings field and the right-click menu, and
    /// clamped to `FONT_SIZE_MIN..=FONT_SIZE_MAX` on load.
    pub font_size: f32,
    /// Installed font family; empty uses the built-in fallback chain.
    pub font_family: String,
    /// Draw every card as a single line instead of two, halving the card
    /// height. Toggled from the right-click menu; the window is resized to keep
    /// the same row count, so the panel gets shorter rather than denser.
    pub single_line: bool,
    /// Panel opacity in percent (10–100). 100 is fully opaque; the value is
    /// applied as a Win32 layered-window alpha. Adjustable from the settings
    /// field and clamped to `OPACITY_MIN..=OPACITY_MAX` on load.
    pub opacity: u8,
    /// Saved logins. All of them are polled and their requests merged into one
    /// list; the secrets live in the encrypted blob under the same ids.
    pub accounts: Vec<Account>,
    /// (account, channel) pairs whose requests are hidden from the merged list.
    pub hidden_channels: Vec<HiddenChannel>,
    /// Card cells the user switched off in the settings window. Empty means
    /// everything is shown, so a cell added in a later version starts visible.
    pub hidden_fields: Vec<DisplayField>,
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

/// Body font size bounds in px at the 96-DPI baseline. The settings field, the
/// menu and the load-time clamp all share these, so the range moves in one
/// place.
pub const FONT_SIZE_MIN: f32 = 6.0;
pub const FONT_SIZE_MAX: f32 = 50.0;
/// Panel opacity bounds in percent at the 96-DPI baseline. 100 is fully
/// opaque; the floor keeps the panel legible rather than vanishing.
pub const OPACITY_MIN: u8 = 10;
pub const OPACITY_MAX: u8 = 100;

impl Default for WindowState {
    fn default() -> Self {
        WindowState {
            // Negative coordinates mean "not positioned yet"; the window is
            // then parked at the top-right of the work area on first launch.
            x: -1,
            y: -1,
            width: 452,
            height: crate::ui::layout::height_for_rows(DEFAULT_ROWS, 1.0, false),
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
            font_size: 12.5,
            font_family: String::new(),
            single_line: false,
            opacity: 100,
            always_on_top: true,
            pin_position: false,
            always_on_bottom: false,
            logical_window: true,
            accounts: Vec::new(),
            hidden_channels: Vec::new(),
            hidden_fields: Vec::new(),
            window: WindowState::default(),
        }
    }
}

/// What is written to the encrypted blob. Which fields are populated depends on
/// the configured mode; unused fields are simply absent.
///
/// The single-account fields are the original format. They are still read (and
/// folded into `accounts` on load) so an install from before multiple accounts
/// keeps its token, and they are written as empty so a fresh file carries only
/// the account list.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Stored {
    /// Kept in every mode that stores anything, so the sign-in form can be
    /// prefilled after the token expires.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub email: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    /// Secrets per account, keyed by `Account::id`.
    pub accounts: Vec<StoredAccount>,
}

/// The secrets of one account. Same shape as `Stored`, minus the list.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct StoredAccount {
    pub id: String,
    pub email: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
}

/// One saved AxonHub login. Non-secret: the token lives in the encrypted blob
/// under the same `id`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Account {
    pub id: String,
    /// Label the user typed when adding the account.
    pub name: String,
    pub endpoint: String,
    pub project_id: String,
}

/// A channel of one account whose requests the panel does not show.
///
/// AxonHub chains accounts: a channel of account A can forward a request to
/// account B, and the request is then recorded on both sides — the same call
/// shows up twice in a merged list. Hiding one side's channel removes the
/// duplicate without changing anything on either server.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct HiddenChannel {
    pub account_id: String,
    pub channel: String,
}

/// One informational cell a request card can show. Listed in the order the
/// settings window offers them: the status marks first, then the request's
/// identity and routing, and finally the timestamp.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DisplayField {
    /// 流: whether the client asked for a streamed answer.
    Stream,
    /// 转: whether the protocol was converted on the way out.
    Conversion,
    /// 透: whether pass-through was applied.
    PassThrough,
    Model,
    /// 协议: the interface type the caller spoke.
    Protocol,
    Caller,
    Channel,
    Tokens,
    /// 缓存: prompt cache hit rate.
    Cache,
    /// 速度: tokens per second.
    Speed,
    /// 重试: attempts beyond the first.
    Retry,
    /// 账号: which saved login the request came from.
    Account,
    /// 创建时间: relative timestamp.
    CreatedAt,
}

impl DisplayField {
    /// Every cell, in settings order.
    pub const ALL: [DisplayField; 13] = [
        DisplayField::Stream,
        DisplayField::Conversion,
        DisplayField::PassThrough,
        DisplayField::Model,
        DisplayField::Protocol,
        DisplayField::Caller,
        DisplayField::Channel,
        DisplayField::Tokens,
        DisplayField::Cache,
        DisplayField::Speed,
        DisplayField::Retry,
        DisplayField::Account,
        DisplayField::CreatedAt,
    ];

    /// Name shown on the settings checkbox: the card's mark, followed by what
    /// the cell actually holds.
    pub fn label(self) -> &'static str {
        match self {
            DisplayField::Stream => "流 · 流式传输",
            DisplayField::Conversion => "转 · 协议转换",
            DisplayField::PassThrough => "透 · 透传",
            DisplayField::Model => "模型",
            DisplayField::Protocol => "协议 · 接口类型",
            DisplayField::Caller => "调用方 · API Key",
            DisplayField::Channel => "渠道",
            DisplayField::Tokens => "词元 · Token 数",
            DisplayField::Cache => "缓存 · 命中率",
            DisplayField::Speed => "速度 · tok/s",
            DisplayField::Retry => "重试次数",
            DisplayField::Account => "账号标签",
            DisplayField::CreatedAt => "创建时间",
        }
    }
}

impl Stored {
    /// Secrets for one account, if any were stored.
    pub fn account(&self, id: &str) -> Option<&StoredAccount> {
        self.accounts.iter().find(|a| a.id == id)
    }

    /// The stored token for one account.
    pub fn token_for(&self, id: &str) -> Option<String> {
        self.account(id).and_then(|a| a.token.clone())
    }

    /// Insert or replace one account's entry, keeping the list ordered by id of
    /// arrival (the newest account is appended).
    pub fn put_account(&mut self, entry: StoredAccount) {
        match self.accounts.iter_mut().find(|a| a.id == entry.id) {
            Some(slot) => *slot = entry,
            None => self.accounts.push(entry),
        }
    }

    /// Store a token for an account, creating its entry if needed. An empty
    /// token clears it instead.
    pub fn set_token(&mut self, id: &str, token: &str) {
        let entry = match self.accounts.iter_mut().find(|a| a.id == id) {
            Some(slot) => slot,
            None => {
                self.accounts.push(StoredAccount {
                    id: id.to_string(),
                    ..Default::default()
                });
                self.accounts.last_mut().expect("just pushed")
            }
        };
        entry.token = (!token.is_empty()).then(|| token.to_string());
    }

    /// Forget one account's secrets.
    pub fn forget_account(&mut self, id: &str) {
        self.accounts.retain(|a| a.id != id);
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
/// Defaults to `%APPDATA%\ah-panel`. Public so the binary can place its log
/// file next to the config (design issue #7).
pub fn base_dir() -> PathBuf {
    if let Some(dir) = std::env::var("AH_PANEL_HOME")
        .ok()
        .filter(|s| !s.is_empty())
    {
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

/// Outcome of a config load, exposed so a future UI can surface a
/// "your config was corrupt and replaced with defaults" notice.
///
/// `Ok` covers both a clean read and the first launch (no file yet). `Corrupt`
/// means the on-disk file existed but was unreadable, and a copy of it has been
/// preserved under `<base>.json.corrupt` so the bytes are not lost silently.
pub enum ConfigLoadStatus {
    Ok,
    Corrupt,
}

impl Config {
    /// Load the config, additionally reporting whether a corrupt file had to be
    /// discarded (and preserved) so callers can warn the user.
    pub fn load_with_status() -> (Config, ConfigLoadStatus) {
        match std::fs::read_to_string(config_path()) {
            // First launch: no file to read. Nothing is corrupt, nothing to back
            // up, so defaults with a clean status.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                (Config::default(), ConfigLoadStatus::Ok)
            }
            // The file exists but cannot be read (permissions, locked, etc.).
            // There are no readable bytes to preserve, so fall back with a
            // `Corrupt` status but do not create a backup.
            Err(_) => {
                warn!(
                    "配置文件存在但无法读取,回退默认配置: {}",
                    config_path().display()
                );
                (Config::default(), ConfigLoadStatus::Corrupt)
            }
            Ok(raw) => match serde_json::from_str::<Config>(&raw) {
                Ok(mut config) => {
                    // A hand-edited file may hold NaN, infinity or a value
                    // outside the supported range; fall back to the authored
                    // 12.5 so a bad number can never shrink the panel to
                    // nothing or blow it up.
                    if !config.font_size.is_finite()
                        || !(FONT_SIZE_MIN..=FONT_SIZE_MAX).contains(&config.font_size)
                    {
                        config.font_size = 12.5;
                    }
                    if !(OPACITY_MIN..=OPACITY_MAX).contains(&config.opacity) {
                        config.opacity = 100;
                    }
                    (config, ConfigLoadStatus::Ok)
                }
                Err(_) => {
                    // The file existed but would not parse — hand-edited into
                    // invalid JSON, or a write was interrupted mid-flush. Keep
                    // the bytes before dropping to defaults so the user can
                    // recover them.
                    warn!(
                        "配置文件损坏,已备份并回退默认配置: {}",
                        config_path().display()
                    );
                    preserve_corrupt(&config_path(), &raw);
                    (Config::default(), ConfigLoadStatus::Corrupt)
                }
            },
        }
    }

    /// Load the config. Return type is unchanged so existing call sites need no
    /// edits; internally this is just `load_with_status().0`.
    pub fn load() -> Self {
        Self::load_with_status().0
    }

    /// Save, returning the underlying I/O error instead of swallowing it. Lets a
    /// caller report a failure rather than lose settings with no signal.
    pub fn try_save(&self) -> std::io::Result<()> {
        let dir = base_dir();
        std::fs::create_dir_all(&dir)?;
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        std::fs::write(config_path(), json)?;
        Ok(())
    }

    /// Save the config. Public signature is unchanged; errors are no longer
    /// silently dropped, but we still avoid panicking here — a write failure is
    /// surfaced to the log (design issue #7) and otherwise ignored so the caller
    /// does not need to handle it.
    pub fn save(&self) {
        if let Err(e) = self.try_save() {
            warn!("配置保存失败: {e}");
        }
    }

    /// Persist exactly what the current mode allows, dropping anything else so a
    /// mode change cannot leave stale secrets behind.
    pub fn store(&self, stored: &Stored) {
        store_in(&base_dir(), self.credential_mode, stored);
    }

    /// The saved account with this id.
    pub fn account(&self, id: &str) -> Option<&Account> {
        self.accounts.iter().find(|a| a.id == id)
    }

    /// Insert or replace an account, keeping the list in arrival order.
    pub fn upsert_account(&mut self, account: Account) {
        match self.accounts.iter_mut().find(|a| a.id == account.id) {
            Some(slot) => *slot = account,
            None => self.accounts.push(account),
        }
    }

    /// Drop one account's metadata.
    pub fn remove_account(&mut self, id: &str) {
        self.accounts.retain(|a| a.id != id);
    }

    /// An id no account is using, derived from the account's name.
    pub fn fresh_account_id(&self) -> String {
        let mut n = self.accounts.len() + 1;
        loop {
            let id = format!("a{n}");
            if self.account(&id).is_none() {
                return id;
            }
            n += 1;
        }
    }

    /// Whether one account's channel is hidden from the merged list.
    pub fn hides_channel(&self, account_id: &str, channel: &str) -> bool {
        self.hidden_channels
            .iter()
            .any(|h| h.account_id == account_id && h.channel == channel)
    }

    /// Hide a channel, or show it again if it was already hidden.
    pub fn toggle_hidden_channel(&mut self, account_id: &str, channel: &str) {
        let entry = HiddenChannel {
            account_id: account_id.to_string(),
            channel: channel.to_string(),
        };
        match self.hidden_channels.iter().position(|h| *h == entry) {
            Some(index) => {
                self.hidden_channels.remove(index);
            }
            None => self.hidden_channels.push(entry),
        }
    }

    /// Whether a card shows this cell. Everything starts visible, so only the
    /// hidden list is stored.
    pub fn shows(&self, field: DisplayField) -> bool {
        !self.hidden_fields.contains(&field)
    }

    /// Switch a card cell off, or back on.
    pub fn toggle_field(&mut self, field: DisplayField) {
        match self.hidden_fields.iter().position(|f| *f == field) {
            Some(index) => {
                self.hidden_fields.remove(index);
            }
            None => self.hidden_fields.push(field),
        }
    }

    /// Show every card cell again.
    pub fn show_all_fields(&mut self) {
        self.hidden_fields.clear();
    }
}

/// Fold the single-account credentials of an older install into the account
/// list, so the token that was already on disk keeps working. Returns whether
/// anything changed and therefore needs saving.
pub fn migrate_single_account(config: &mut Config, stored: &mut Stored) -> bool {
    if !config.accounts.is_empty() {
        return false;
    }
    let token = stored.token.take();
    let email = std::mem::take(&mut stored.email);
    let password = stored.password.take();
    if token.is_none() && password.is_none() {
        return false;
    }
    let name = if email.is_empty() {
        endpoint_host(&config.endpoint)
    } else {
        email.clone()
    };
    let id = "a1".to_string();
    config.upsert_account(Account {
        id: id.clone(),
        name,
        endpoint: config.endpoint.clone(),
        project_id: config.project_id.clone(),
    });
    stored.put_account(StoredAccount {
        id,
        email,
        password,
        token,
    });
    true
}

/// The host part of an endpoint URL, for naming an account the user has not
/// named themselves. Falls back to the whole string when it is not a URL.
pub fn endpoint_host(endpoint: &str) -> String {
    let trimmed = endpoint.trim().trim_end_matches('/');
    let after_scheme = trimmed.split("://").last().unwrap_or(trimmed);
    let host = after_scheme.split('/').next().unwrap_or(after_scheme);
    if host.is_empty() {
        trimmed.to_string()
    } else {
        host.to_string()
    }
}

/// Keep a byte-for-byte copy of a config file we failed to parse, so the user's
/// hand edits or a half-written file are not silently destroyed.
///
/// Strategy, in order: rename to `<base>.json.corrupt`; if a backup already
/// exists there, append a Unix timestamp to avoid clobbering it; if rename fails
/// (most likely a cross-volume move, where `rename` cannot relocate data), fall
/// back to writing a copy of the raw bytes and removing the original. The goal
/// is to preserve the bad bytes whenever physically possible.
fn preserve_corrupt(path: &Path, raw: &str) {
    let corrupt_path = path.with_extension("json.corrupt");
    // A previous corrupt backup already sits at the default name — keep it and
    // timestamp this one instead of overwriting the earlier copy.
    let target = if corrupt_path.exists() {
        path.with_extension(format!("json.corrupt.{}", unix_now()))
    } else {
        corrupt_path
    };

    // Prefer a cheap rename on the same volume.
    if std::fs::rename(path, &target).is_ok() {
        warn!("已备份损坏的配置文件到: {}", target.display());
        return;
    }
    // Cross-volume (or otherwise refused) rename: write the bytes out to the
    // target and, on success, drop the original so the panel's next save starts
    // from a clean slate.
    if std::fs::write(&target, raw.as_bytes()).is_ok() {
        warn!("已备份损坏的配置文件到: {}", target.display());
        let _ = std::fs::remove_file(path);
        return;
    }
    // Both attempts failed: the bad bytes are unrecoverable. Log so the loss is
    // at least visible in the audit trail.
    warn!("无法备份损坏的配置文件(字节已丢失): {}", path.display());
}

/// Seconds since the Unix epoch, used to disambiguate successive corrupt
/// backups. Falls back to 0 if the clock is somehow before the epoch.
fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// What a given mode is permitted to write, if anything.
fn filtered(mode: CredentialMode, stored: &Stored) -> Option<Stored> {
    match mode {
        CredentialMode::None => None,
        CredentialMode::Token => {
            // Tokens only: a password is never written in this mode, not even
            // for an account that signed in with one.
            let mut record = stored.clone();
            record.password = None;
            for account in &mut record.accounts {
                account.password = None;
            }
            Some(record)
        }
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
    let empty = record.token.is_none()
        && record.password.is_none()
        && record
            .accounts
            .iter()
            .all(|a| a.token.is_none() && a.password.is_none());
    if empty {
        // Nothing worth keeping; do not leave an empty blob around.
        clear_stored_in(dir);
        return;
    }
    let Ok(plain) = serde_json::to_vec(&record) else {
        return;
    };
    let Some(blob) = dpapi::protect(&plain) else {
        // DPAPI 加密失败：绝对不能把明文凭据落到磁盘，因此直接放弃写入。
        warn!("凭据加密(DPAPI)失败,未写入凭据文件");
        return;
    };
    if std::fs::create_dir_all(dir).is_err() {
        warn!("无法创建凭据目录,未写入凭据: {}", dir.display());
        return;
    }
    if let Err(e) = std::fs::write(credentials_path_in(dir), blob) {
        warn!("凭据文件写入失败: {e}");
    }
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
    let plain = match dpapi::unprotect(&blob) {
        Some(p) => p,
        // DPAPI 解密失败（换了 Windows 用户、或 blob 损坏）：视作无凭据，不崩。
        None => {
            warn!("凭据解密(DPAPI)失败,忽略已保存凭据");
            return None;
        }
    };
    match serde_json::from_slice(&plain) {
        Ok(s) => Some(s),
        Err(_) => {
            warn!("凭据文件内容无法解析,忽略已保存凭据");
            None
        }
    }
}

/// Forget the stored credentials. Deliberately does not touch `config.json`:
/// clearing a credential must not reset the endpoint, window or poll settings.
pub fn clear_stored() {
    clear_stored_in(&base_dir());
}

fn clear_stored_in(dir: &Path) {
    let _ = std::fs::remove_file(credentials_path_in(dir));
}

/// Bring the window back on screen if a display change orphaned it, shrinking it
/// to fit the work area when the display got smaller. `state.width`/`height` are
/// device pixels; the minimums stay usable by scaling the logical floor.
///
/// The result is always *wholly* inside `screen`: a borderless panel that hangs
/// over an edge has no frame to grab back, and one straddling two monitors
/// reads badly however it is moved. Callers therefore use this both for display
/// changes and for clamping a move in flight.
///
/// The floor is lower in single-line mode: there the five-row preset is only
/// ~176 logical pixels tall, so a 200-pixel minimum would stretch the window
/// back out on the next resize or launch.
pub fn clamp_to_virtual_screen(
    state: WindowState,
    screen: (i32, i32, i32, i32),
    scale: f32,
    single_line: bool,
) -> WindowState {
    let (left, top, right, bottom) = screen;
    let min_w = crate::ui::layout::device_px(320, scale);
    let min_h = crate::ui::layout::device_px(if single_line { 110 } else { 200 }, scale);
    let width = state.width.clamp(min_w, (right - left).max(min_w));
    let height = state.height.clamp(min_h, (bottom - top).max(min_h));
    let x = state.x.clamp(left, (right - width).max(left));
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
#[path = "tests/config.rs"]
mod tests;
