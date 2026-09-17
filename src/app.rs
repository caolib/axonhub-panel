//! Panel state: what is on screen and how user input mutates it.
//!
//! Scroll clamping lives in the UI layer because it depends on the live window
//! metrics; this module only records intent.

use crate::client::ApiError;
use crate::config::{Config, Credentials, Stored};
use crate::model::{self, Filter, FilterMask, Row, Source};
use crate::ui::login::{LoginForm, Method};
use crate::worker::{Command, OctopusHealth, Update, Worker};

/// Which surface the panel is showing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    List,
    Login,
}

pub struct App {
    pub view: View,
    pub config: Config,
    /// The merged, truncated list the UI renders. AxonHub and Octopus rows are
    /// kept apart and re-merged on every update, so each source keeps updating
    /// when the other is down.
    pub rows: Vec<Row>,
    axon_rows: Vec<Row>,
    axon_total: i64,
    octo_rows: Vec<Row>,
    /// Octopus connection state, shown in the context menu.
    pub octopus_health: OctopusHealth,
    pub total: i64,
    /// Selected status-filter bits applied on top of the merged list;
    /// `FILTER_NONE` shows every row.
    pub filter: FilterMask,
    /// Unclamped scroll offset in pixels; the UI clamps against the metrics.
    pub scroll: f32,
    pub hover: Option<usize>,
    pub selected: Option<usize>,
    pub paused: bool,
    pub user_name: Option<String>,
    /// Transient status line; `true` marks an error presentation.
    pub status: Option<(String, bool)>,
    pub login: Option<LoginForm>,
    pub token: Option<String>,
    /// What the last successful sign-in should be persisted as.
    pub stored: Stored,
}

impl App {
    pub fn new(config: Config, token: Option<String>, prefill: Option<Credentials>) -> Self {
        let view = if token.is_some() {
            View::List
        } else {
            View::Login
        };
        let login = (view == View::Login)
            .then(|| LoginForm::new(&config.endpoint, prefill.as_ref(), config.credential_mode));
        App {
            view,
            rows: Vec::new(),
            axon_rows: Vec::new(),
            axon_total: 0,
            octo_rows: Vec::new(),
            octopus_health: OctopusHealth::Disabled,
            total: 0,
            filter: model::FILTER_NONE,
            scroll: 0.0,
            hover: None,
            selected: None,
            paused: false,
            user_name: None,
            status: None,
            login,
            token,
            stored: Stored::default(),
            config,
        }
    }

    pub fn open_login(&mut self, message: Option<String>, prefill: Option<Credentials>) {
        let mut form = LoginForm::new(
            &self.config.endpoint,
            prefill.as_ref(),
            self.config.credential_mode,
        );
        form.error = message;
        self.login = Some(form);
        self.view = View::Login;
        self.scroll = 0.0;
        self.hover = None;
    }

    pub fn is_login(&self) -> bool {
        self.view == View::Login
    }

    /// Apply everything the worker produced.
    pub fn apply_updates(&mut self, updates: Vec<Update>, worker: &Worker) {
        for update in updates {
            match update {
                Update::SignedIn { token, user } => {
                    let project =
                        crate::client::resolve_project(&user.user, &self.config.project_id);
                    if !project.is_empty() {
                        self.config.project_id = project;
                    }
                    self.token = Some(token.clone());
                    self.user_name = Some(user.user.display_name());
                    self.view = View::List;
                    self.status = Some(("已登录".into(), false));

                    // Record the freshly issued token under the current mode,
                    // keeping whatever the form already established about which
                    // method and storage the user chose.
                    let (email, password) = self
                        .login
                        .as_ref()
                        .map(|f| (f.email.trim().to_string(), f.password.clone()))
                        .unwrap_or_default();
                    let password = (!password.is_empty()).then_some(password);
                    let mut stored = self.stored.clone();
                    stored.email = email;
                    stored.password = password;
                    stored.token = Some(token.clone());
                    self.stored = stored.clone();
                    self.config.store(&stored);

                    worker.send(Command::SetToken(token));
                }
                Update::Snapshot { rows, total } => {
                    self.axon_rows = rows;
                    self.axon_total = total;
                    self.rebuild();
                    self.status = None;
                }
                Update::OctopusRows(rows) => {
                    self.octo_rows = rows;
                    self.rebuild();
                    // Deliberately leaves `status` alone: Octopus health must
                    // not mask a red AxonHub status line.
                }
                Update::OctopusHealth(health) => {
                    self.octopus_health = health;
                }
                Update::Failed(err) => {
                    // A token the server rejects is worse than useless: drop any
                    // stored copy so the next launch does not retry it silently.
                    if matches!(err, ApiError::Expired | ApiError::Unauthorized) {
                        if self
                            .login
                            .as_ref()
                            .is_some_and(|f| f.method == Method::Token)
                        {
                            crate::config::clear_stored();
                        }
                    }
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
                        self.open_login(Some("登录已过期,请重新登录".into()), None);
                    }
                }
            }
        }
    }

    /// Re-merge both sources into the display list, newest first, capped at the
    /// configured row count, then drop rows that fail the status mask. Both
    /// sources stay live independently: a successful update from one never
    /// discards the other's rows.
    fn rebuild(&mut self) {
        let mut rows = Vec::with_capacity(self.axon_rows.len() + self.octo_rows.len());
        rows.extend(self.axon_rows.iter().cloned());
        rows.extend(self.octo_rows.iter().cloned());
        rows.sort_by_key(|row| std::cmp::Reverse(row.age_key()));
        if self.filter != model::FILTER_NONE {
            rows.retain(|row| model::mask_matches(self.filter, row.status));
        }
        rows.truncate(self.config.row_limit.max(1) as usize);
        self.rows = rows;
        self.total = self.axon_total + self.octo_rows.len() as i64;
        if self.selected.is_some_and(|sel| sel >= self.rows.len()) {
            self.selected = None;
        }
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

    /// One-line Octopus state for the context menu. `None` when the second
    /// source is disabled, which also hides its menu group.
    pub fn octopus_summary(&self) -> Option<String> {
        match &self.octopus_health {
            OctopusHealth::Disabled => None,
            OctopusHealth::MissingToken => Some("Octopus: 未设置令牌".into()),
            OctopusHealth::Connected => Some("Octopus: 已连接".into()),
            OctopusHealth::Error(message) => Some(format!("Octopus: {message}")),
        }
    }

    /// Record a click on the request list, returning the URL to open. Octopus
    /// has no per-request routes, so its rows open the dashboard instead.
    pub fn click_row(&mut self, index: usize) -> Option<String> {
        self.selected = Some(index);
        let row = self.rows.get(index)?;
        Some(match row.source {
            Source::AxonHub => model::request_url(&self.config.endpoint, &row.id),
            Source::Octopus => model::octopus_url(&self.config.octopus_endpoint),
        })
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
            source: Source::AxonHub,
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
        }
    }

    fn seeded() -> App {
        let mut app = App::new(Config::default(), None, None);
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
}
