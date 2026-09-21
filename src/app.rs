//! Panel state: what is on screen and how user input mutates it.
//!
//! Scroll clamping lives in the UI layer because it depends on the live window
//! metrics; this module only records intent.

use crate::config::{Account, Config, HiddenChannel, Stored};
use crate::model::{self, Filter, FilterMask, Row};
use crate::ui::login::LoginForm;
use crate::worker::{AccountIssue, Update};
use tracing::info;

/// Which surface the panel is showing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    List,
    Login,
}

/// Which login the sign-in form is collecting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoginTarget {
    /// A new account: the name field starts empty.
    Add,
    /// An existing account, re-authenticating or being renamed.
    Account(String),
}

/// A "test this account" round trip the user asked for from the settings
/// window, and what came back.
#[derive(Debug, Clone, PartialEq)]
pub struct Probe {
    pub id: String,
    pub state: ProbeState,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ProbeState {
    /// Waiting for the worker's answer.
    Running,
    /// The account answered; the number is its total request count.
    Ok(i64),
    /// The account could not be reached, or has no credentials left.
    Failed(String),
}

pub struct App {
    pub view: View,
    pub config: Config,
    /// The truncated list the UI renders, rebuilt on every update.
    pub rows: Vec<Row>,
    axon_rows: Vec<Row>,
    axon_total: i64,
    pub total: i64,
    /// Selected status-filter bits applied on top of the merged list;
    /// `FILTER_NONE` shows every row.
    pub filter: FilterMask,
    /// Unclamped scroll offset in pixels; the UI clamps against the metrics.
    pub scroll: f32,
    pub hover: Option<usize>,
    pub selected: Option<usize>,
    pub user_name: Option<String>,
    /// Transient status line; `true` marks an error presentation.
    pub status: Option<(String, bool)>,
    pub login: Option<LoginForm>,
    /// Saved accounts, mirroring `config.accounts`.
    pub accounts: Vec<Account>,
    /// Accounts that answered the last poll.
    pub open_accounts: Vec<String>,
    /// Accounts that did not, with the reason.
    pub issues: Vec<AccountIssue>,
    /// The last account test, if one has been run.
    pub probe: Option<Probe>,
    /// What the last successful sign-in should be persisted as.
    pub stored: Stored,
}

impl App {
    pub fn new(config: Config) -> Self {
        App {
            view: View::List,
            accounts: config.accounts.clone(),
            rows: Vec::new(),
            axon_rows: Vec::new(),
            axon_total: 0,
            total: 0,
            filter: model::FILTER_NONE,
            scroll: 0.0,
            hover: None,
            selected: None,
            user_name: None,
            status: None,
            login: None,
            open_accounts: Vec::new(),
            issues: Vec::new(),
            probe: None,
            stored: Stored::default(),
            config,
        }
    }

    /// Show the sign-in form for one account (or for a new one), replacing
    /// whatever the form held before.
    pub fn open_login(&mut self, message: Option<String>, target: LoginTarget) {
        let form = match &target {
            LoginTarget::Account(id) => self.config.account(id).map(|account| {
                LoginForm::for_account(account, self.config.credential_mode)
            }),
            LoginTarget::Add => None,
        };
        let mode = self.config.credential_mode;
        let mut form = form.unwrap_or_else(|| LoginForm::adding(&self.config.endpoint, mode));
        form.error = message;
        self.login = Some(form);
        self.view = View::Login;
        self.scroll = 0.0;
        self.hover = None;
    }

    pub fn is_login(&self) -> bool {
        self.view == View::Login
    }

    /// How one account is doing, for the menu: `None` when it answered the last
    /// poll, otherwise a short label for what is wrong.
    pub fn account_state(&self, id: &str) -> Option<&'static str> {
        let issue = self.issues.iter().find(|i| i.id == id)?;
        Some(if issue.needs_signin {
            "需重新登录"
        } else {
            "连接失败"
        })
    }

    /// Whether any account is waiting on a fresh sign-in — an expired or
    /// rejected token. The settings button is drawn red while this holds.
    pub fn needs_signin(&self) -> bool {
        self.issues.iter().any(|i| i.needs_signin)
    }

    /// Start tracking a test for one account, replacing any previous result.
    pub fn begin_probe(&mut self, id: &str) {
        self.probe = Some(Probe {
            id: id.to_string(),
            state: ProbeState::Running,
        });
    }

    /// Apply everything the worker produced.
    pub fn apply_updates(&mut self, updates: Vec<Update>) {
        for update in updates {
            match update {
                Update::SignedIn { token, user } => {
                    let project =
                        crate::client::resolve_project(&user.user, &self.config.project_id);
                    if !project.is_empty() {
                        self.config.project_id = project;
                    }
                    self.user_name = Some(user.user.display_name());
                    self.view = View::List;
                    self.status = Some(("已登录".into(), false));
                    info!("登录成功");

                    // Record the freshly issued token: the account the form was
                    // editing, or the legacy single-account slot. The worker
                    // already holds it for the target it signed in.
                    let (email, password) = self
                        .login
                        .as_ref()
                        .map(|f| (f.email.trim().to_string(), f.password.clone()))
                        .unwrap_or_default();
                    let password = (!password.is_empty()).then_some(password);
                    let mut stored = self.stored.clone();
                    stored.email = email;
                    stored.password = password;
                    match self.login.as_ref().and_then(|f| f.editing.clone()) {
                        Some(id) => stored.set_token(&id, &token),
                        None => stored.token = Some(token),
                    }
                    self.stored = stored.clone();
                    self.config.store(&stored);
                    self.accounts = self.config.accounts.clone();
                    self.save();
                }
                Update::Snapshot { rows, total } => {
                    self.axon_rows = rows;
                    self.axon_total = total;
                    self.rebuild();
                    // A broken account is worth keeping on the status line,
                    // which a plain "everything is fine" poll would otherwise
                    // clear a moment later.
                    self.status = self.issue_status();
                }
                Update::Accounts { open, broken } => {
                    self.open_accounts = open;
                    // A rejected token is dropped for good, so a restart does
                    // not silently retry it. Other failures (an unreachable
                    // server) keep their token and recover on their own.
                    for issue in &broken {
                        if issue.needs_signin {
                            let mut stored = self.stored.clone();
                            stored.forget_account(&issue.id);
                            self.stored = stored.clone();
                            self.config.store(&stored);
                        }
                    }
                    self.issues = broken;
                    self.status = self.issue_status();
                }
                Update::Detail { .. } => {
                    // Consumed by the window layer, which owns the detail popup;
                    // `App` holds list state only.
                }
                Update::Probe { id, result } => {
                    // A late answer for an account the user has since tested
                    // again (or signed into) is dropped rather than painted over
                    // the newer state.
                    if let Some(probe) = self.probe.as_mut().filter(|p| p.id == id) {
                        probe.state = match result {
                            Ok(total) => ProbeState::Ok(total),
                            Err(message) => ProbeState::Failed(message),
                        };
                    }
                }
                Update::Failed(err) => {
                    // The login view does not render the status line, so a failed
                    // sign-in has to surface on the form itself. Clearing `busy`
                    // is what lets the user correct their input and retry.
                    if self.is_login() {
                        if let Some(form) = self.login.as_mut() {
                            form.busy = false;
                            form.error = Some(err.message());
                        }
                    }
                    self.status = Some((err.message(), true));
                }
                Update::SignedOut => {
                    if self.is_login() {
                        // Already collecting credentials; just surface the reason.
                        if let Some(form) = self.login.as_mut() {
                            form.busy = false;
                        }
                    } else {
                        // Nothing is left to poll: ask for a token again, for
                        // the first account that lost one.
                        let target = match self.accounts.first() {
                            Some(account) => LoginTarget::Account(account.id.clone()),
                            None => LoginTarget::Add,
                        };
                        self.open_login(Some("登录已过期,请重新登录".into()), target);
                    }
                }
            }
        }
    }

    /// The status line for a broken account, if any: one line, naming the
    /// account so it is clear which login needs attention.
    fn issue_status(&self) -> Option<(String, bool)> {
        let issue = self.issues.first()?;
        Some((format!("账号 {} {}", issue.name, issue.message), true))
    }

    /// Re-sort the display list, newest first, capped at the configured row
    /// count, then drop rows that fail the status mask or that the user asked
    /// to hide.
    pub fn rebuild(&mut self) {
        let mut rows = self.axon_rows.clone();
        rows.sort_by_key(|row| std::cmp::Reverse(row.age_key()));
        if self.filter != model::FILTER_NONE {
            rows.retain(|row| model::mask_matches(self.filter, row.status));
        }
        // Hidden channels drop out before the cap, so the visible count is
        // still the newest N rows.
        let hidden = &self.config.hidden_channels;
        if !hidden.is_empty() {
            rows.retain(|row| {
                !row.channel.as_deref().is_some_and(|channel| {
                    hidden
                        .iter()
                        .any(|h| h.account_id == row.account_id && h.channel == channel)
                })
            });
        }
        rows.truncate(self.config.row_limit.max(1) as usize);
        self.rows = rows;
        self.total = self.axon_total;
        if self.selected.is_some_and(|sel| sel >= self.rows.len()) {
            self.selected = None;
        }
    }

    /// The channel filters the menu offers: every (account, channel) pair seen
    /// in the fetched rows, plus the ones already hidden — which by definition
    /// are absent from those rows and still have to be listed, checked, so they
    /// can be shown again.
    pub fn channel_filters(&self) -> Vec<HiddenChannel> {
        let mut out: Vec<HiddenChannel> = self.config.hidden_channels.clone();
        for row in &self.axon_rows {
            let Some(channel) = row.channel.as_deref() else {
                continue;
            };
            if row.account_id.is_empty() {
                continue;
            }
            let entry = HiddenChannel {
                account_id: row.account_id.clone(),
                channel: channel.to_string(),
            };
            if !out.contains(&entry) {
                out.push(entry);
            }
        }
        // Grouped by account, then alphabetically, so the menu reads the same
        // way twice in a row.
        out.sort_by(|a, b| (&a.account_id, &a.channel).cmp(&(&b.account_id, &b.channel)));
        out
    }

    /// Apply a chip click: 全部 clears the mask, a status chip toggles its
    /// bit, and the merged list is re-filtered.
    pub fn click_filter(&mut self, chip: Filter) {
        let next = if chip == Filter::All {
            model::FILTER_NONE
        } else {
            self.filter ^ chip.mask()
        };
        if next == self.filter {
            return;
        }
        self.filter = next;
        self.rebuild();
    }

    /// Persist anything that changed while running.
    pub fn save(&self) {
        self.config.save();
    }
}

#[cfg(test)]
#[path = "tests/app.rs"]
mod tests;
