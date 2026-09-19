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
mod tests {
    use super::*;
    use crate::model::Status;

    fn row(status: Status) -> Row {
        Row {
            id: String::new(),
            account_id: String::new(),
            account_name: String::new(),
            created_at: None,
            status,
            model: String::new(),
            routed_model: None,
            channel: None,
            caller: None,
            format: None,
            reasoning_effort: None,
            upstream_format: None,
            pass_through: false,
            stream: false,
            latency_ms: None,
            first_token_ms: None,
            prompt_tokens: 0,
            total_tokens: 0,
            cached_tokens: 0,
            attempt_count: 1,
            failed_attempts: 0,
            attempts_truncated: false,
        }
    }

    fn seeded() -> App {
        let mut app = App::new(Config::default());
        app.axon_total = 5;
        app.axon_rows = vec![
            row(Status::Completed),
            row(Status::Failed),
            row(Status::Processing),
            row(Status::Pending),
            row(Status::Canceled),
        ];
        app.rebuild();
        app
    }

    #[test]
    fn filter_keeps_only_matching_rows() {
        let mut app = seeded();
        assert_eq!(app.rows.len(), 5);

        app.click_filter(Filter::Completed);
        assert_eq!(app.rows.len(), 1);
        assert_eq!(app.rows[0].status, Status::Completed);

        app.click_filter(Filter::All);
        assert_eq!(app.rows.len(), 5);

        // 进行 covers both Processing and Pending, mirroring `is_active`.
        app.click_filter(Filter::Active);
        assert_eq!(app.rows.len(), 2);
        assert!(app.rows.iter().all(|r| r.status.is_active()));

        app.click_filter(Filter::All);
        app.click_filter(Filter::Failed);
        assert_eq!(app.rows.len(), 1);
        assert_eq!(app.rows[0].status, Status::Failed);

        app.click_filter(Filter::All);
        assert_eq!(app.rows.len(), 5);
    }

    #[test]
    fn chips_combine_and_toggle_independently() {
        let mut app = seeded();

        // 成功 + 进行 selected at once.
        app.click_filter(Filter::Completed);
        app.click_filter(Filter::Active);
        assert_eq!(app.rows.len(), 3);
        assert!(
            app.rows
                .iter()
                .all(|r| { r.status == Status::Completed || r.status.is_active() })
        );

        // Toggling one off keeps the other.
        app.click_filter(Filter::Active);
        assert_eq!(app.rows.len(), 1);
        assert_eq!(app.rows[0].status, Status::Completed);

        // Toggling the last one off returns to the full list.
        app.click_filter(Filter::Completed);
        assert_eq!(app.rows.len(), 5);
        assert_eq!(app.filter, model::FILTER_NONE);
    }

    #[test]
    fn total_always_counts_every_source_row() {
        let mut app = seeded();
        let total_was = app.total;
        app.click_filter(Filter::Failed);
        assert_eq!(app.total, total_was);
        assert_eq!(app.rows.len(), 1);
    }

    #[test]
    fn hiding_a_channel_only_drops_that_accounts_rows() {
        let mut app = App::new(Config::default());
        // The same channel name on both accounts: A forwards to B through it,
        // so the request is listed by both.
        let mut from_a = row(Status::Completed);
        from_a.account_id = "a1".into();
        from_a.account_name = "A".into();
        from_a.channel = Some("relay".into());
        let mut from_b = row(Status::Completed);
        from_b.account_id = "a2".into();
        from_b.account_name = "B".into();
        from_b.channel = Some("relay".into());
        app.axon_rows = vec![from_a, from_b];
        app.axon_total = 2;
        app.rebuild();
        assert_eq!(app.rows.len(), 2);

        app.config.hidden_channels.push(HiddenChannel {
            account_id: "a1".into(),
            channel: "relay".into(),
        });
        app.rebuild();
        assert_eq!(app.rows.len(), 1);
        assert_eq!(app.rows[0].account_id, "a2");

        // And the filters the menu offers keep the hidden pair listed, so it
        // can be switched back on.
        let filters = app.channel_filters();
        assert!(filters.contains(&HiddenChannel {
            account_id: "a1".into(),
            channel: "relay".into(),
        }));
    }
}
