//! Panel state: what is on screen and how user input mutates it.
//!
//! Scroll clamping lives in the UI layer because it depends on the live window
//! metrics; this module only records intent.

use crate::config::{Config, Credentials, Stored};
use crate::model::{self, Row};
use crate::worker::{Command, Update, Worker};
use crate::client::ApiError;
use crate::ui::login::{LoginForm, Method};

/// Which surface the panel is showing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    List,
    Login,
}

pub struct App {
    pub view: View,
    pub config: Config,
    pub rows: Vec<Row>,
    pub total: i64,
    /// Unclamped scroll offset in pixels; the UI clamps against the metrics.
    pub scroll: f32,
    pub hover: Option<usize>,
    pub selected: Option<usize>,
    pub paused: bool,
    pub user_name: Option<String>,
    pub last_refresh: Option<String>,
    /// Transient status line; `true` marks an error presentation.
    pub status: Option<(String, bool)>,
    pub login: Option<LoginForm>,
    pub token: Option<String>,
    /// What the last successful sign-in should be persisted as.
    pub stored: Stored,
}

impl App {
    pub fn new(config: Config, token: Option<String>, prefill: Option<Credentials>) -> Self {
        let view = if token.is_some() { View::List } else { View::Login };
        let login = (view == View::Login)
            .then(|| LoginForm::new(&config.endpoint, prefill.as_ref(), config.credential_mode));
        App {
            view,
            rows: Vec::new(),
            total: 0,
            scroll: 0.0,
            hover: None,
            selected: None,
            paused: false,
            user_name: None,
            last_refresh: None,
            status: None,
            login,
            token,
            stored: Stored::default(),
            config,
        }
    }

    pub fn open_login(&mut self, message: Option<String>, prefill: Option<Credentials>) {
        let mut form =
            LoginForm::new(&self.config.endpoint, prefill.as_ref(), self.config.credential_mode);
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
                    let project = crate::client::resolve_project(&user.user, &self.config.project_id);
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
                Update::Snapshot { rows, total, at } => {
                    self.rows = rows;
                    self.total = total;
                    self.last_refresh = Some(at);
                    if let Some(sel) = self.selected {
                        if sel >= self.rows.len() {
                            self.selected = None;
                        }
                    }
                    self.status = None;
                }
                Update::Failed(err) => {
                    // A token the server rejects is worse than useless: drop any
                    // stored copy so the next launch does not retry it silently.
                    if matches!(err, ApiError::Expired | ApiError::Unauthorized) {
                        if self.login.as_ref().is_some_and(|f| f.method == Method::Token) {
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

    /// Record a click on the request list, returning the URL to open.
    pub fn click_row(&mut self, index: usize) -> Option<String> {
        self.selected = Some(index);
        let row = self.rows.get(index)?;
        Some(model::request_url(&self.config.endpoint, &row.id))
    }

    /// Persist anything that changed while running.
    pub fn save(&self) {
        self.config.save();
    }
}
