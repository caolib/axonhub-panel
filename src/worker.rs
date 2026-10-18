//! Background polling and authentication.
//!
//! All network work happens off the UI thread. Results come back as plain
//! values that the UI applies from its own timer, so painting never blocks on a
//! socket and the panel stays responsive when AxonHub is slow or unreachable.
//!
//! Every saved account is polled in turn and their requests are merged into a
//! single list. Each request carries the account it came from, so the UI can
//! label it.

use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::time::{Duration, Instant};

use crate::client::{ApiError, Client, SignInResponse};
use crate::config::{Account, Config, CredentialMode, Credentials, Stored};
use crate::model::{ExecutionDetail, Row};
use tracing::warn;

/// Upper bound on a requests query before the panel gives up on it.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(12);
/// A detail query is clicked, not polled: it gets its own shorter leash so a
/// gateway that hangs cannot keep the popup spinning for long.
const DETAIL_TIMEOUT: Duration = Duration::from_secs(10);
const SIGNIN_TIMEOUT: Duration = Duration::from_secs(15);
/// Longest sleep, so queued commands are noticed promptly without spinning.
const TICK: Duration = Duration::from_millis(200);

pub enum Update {
    /// Sign-in succeeded; the token and identity are already validated.
    SignedIn {
        account: String,
        creds: Credentials,
        token: String,
        user: Box<SignInResponse>,
    },
    /// Silent renewal only updates credentials; it must not dismiss a form.
    TokenRefreshed {
        account: String,
        token: String,
        project_id: String,
    },
    /// A fresh page of AxonHub requests, merged across every account.
    Snapshot {
        rows: Vec<Row>,
    },
    /// Which accounts answered and which are broken, sent when that changes.
    Accounts {
        open: Vec<String>,
        broken: Vec<AccountIssue>,
    },
    /// Answer to `FetchDetail`: the attempts recorded for one request.
    Detail {
        id: String,
        result: Result<Vec<ExecutionDetail>, ApiError>,
    },
    /// Answer to `TestAccount`: the account's total request count on success,
    /// or why it could not be reached.
    Probe {
        id: String,
        result: Result<i64, String>,
    },
    Failed(ApiError),
    /// No account has a usable token; the UI must show the sign-in form.
    SignedOut,
}

/// One account that failed to answer, and why. `needs_signin` marks the
/// failures that a fresh token would fix.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountIssue {
    pub id: String,
    pub name: String,
    pub message: String,
    pub needs_signin: bool,
}

/// One saved login, as the worker polls it.
#[derive(Debug, Clone)]
pub struct Target {
    pub id: String,
    pub name: String,
    pub endpoint: String,
    pub project_id: String,
    pub token: Option<String>,
    credentials: Option<Credentials>,
    allow_password: bool,
    auth_blocked: bool,
    auth_retry_at: Option<Instant>,
    auth_backoff: Duration,
    auth_error: Option<ApiError>,
}

impl Target {
    pub fn from_account(account: &Account, stored: &Stored, mode: CredentialMode) -> Self {
        // The mode is the permission, not the presence of a field: a blob left
        // from password mode must not be used after the user downgrades.
        let allow_password = mode == CredentialMode::Password;
        Target {
            id: account.id.clone(),
            name: account.name.clone(),
            endpoint: account.endpoint.clone(),
            project_id: account.project_id.clone(),
            token: stored.token_for(&account.id).filter(|t| !t.is_empty()),
            credentials: allow_password
                .then(|| stored.credentials_for(&account.id))
                .flatten(),
            allow_password,
            auth_blocked: false,
            auth_retry_at: None,
            auth_backoff: Duration::ZERO,
            auth_error: None,
        }
    }

    pub fn can_authenticate(&self) -> bool {
        !self.auth_blocked && (self.token.is_some() || self.credentials.is_some())
    }

    /// A password typed into the form is usable right away, even when the
    /// persisted mode is token-only; it just is not written to disk, so it only
    /// enables renewal for this session.
    pub fn set_credentials(&mut self, creds: Credentials) {
        self.credentials = Some(creds);
        self.allow_password = true;
        self.auth_blocked = false;
        self.auth_retry_at = None;
        self.auth_backoff = Duration::ZERO;
        self.auth_error = None;
    }

    /// A downgrade to token-only drops the in-memory password and stops any
    /// silent renewal that was in flight.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn set_allow_password(&mut self, allow: bool) {
        self.allow_password = allow;
        if !allow {
            self.credentials = None;
            self.auth_blocked = false;
            self.auth_retry_at = None;
            self.auth_error = None;
        }
    }

    fn accept_signin(&mut self, response: &SignInResponse) {
        self.token = Some(response.token.clone());
        self.project_id = crate::client::resolve_project(&response.user, &self.project_id);
        self.auth_blocked = false;
        self.auth_retry_at = None;
        self.auth_backoff = Duration::ZERO;
        self.auth_error = None;
    }

    fn renew(
        &mut self,
        signin: &mut impl FnMut(&Credentials) -> Result<SignInResponse, ApiError>,
        updates: &Sender<Update>,
        now: Instant,
    ) -> Result<(), ApiError> {
        if self.auth_blocked {
            return Err(ApiError::Unauthorized);
        }
        if self.auth_retry_at.is_some_and(|retry| now < retry) {
            return Err(self.auth_error.clone().unwrap_or(ApiError::Unauthorized));
        }
        if !self.allow_password {
            return Err(ApiError::Unauthorized);
        }
        let creds = self.credentials.as_ref().ok_or(ApiError::Unauthorized)?;
        match signin(creds) {
            Ok(response) => {
                self.accept_signin(&response);
                let _ = updates.send(Update::TokenRefreshed {
                    account: self.id.clone(),
                    token: response.token,
                    project_id: self.project_id.clone(),
                });
                Ok(())
            }
            Err(err) => {
                // Only a rejected password is permanent. A timeout or a 5xx is
                // transient, so retry it later with a growing delay.
                self.auth_blocked = err.needs_signin();
                self.auth_backoff = next_backoff(self.auth_backoff);
                self.auth_retry_at = Some(now + self.auth_backoff);
                self.auth_error = Some(err.clone());
                Err(err)
            }
        }
    }

    /// Check expiry before a query, then retry a rejected token exactly once.
    /// Sign-in and query closures keep this policy testable without a network.
    fn authorized<T>(
        &mut self,
        mut signin: impl FnMut(&Credentials) -> Result<SignInResponse, ApiError>,
        mut query: impl FnMut(&Target, &str) -> Result<T, ApiError>,
        updates: &Sender<Update>,
        now_unix: i64,
        now: Instant,
    ) -> Result<T, ApiError> {
        if self.auth_blocked {
            return Err(ApiError::Unauthorized);
        }
        let needs_token = self
            .token
            .as_ref()
            .is_none_or(|token| crate::token::is_expired(token, now_unix));
        let mut renewed = false;
        if needs_token {
            self.token = None;
            self.renew(&mut signin, updates, now)?;
            renewed = true;
        }
        let token = self.token.clone().ok_or(ApiError::Unauthorized)?;
        let result = query(self, &token);
        if result.as_ref().is_err_and(|err| err.needs_signin()) {
            self.token = None;
            if renewed || self.credentials.is_none() {
                self.auth_blocked = true;
                return result;
            }
            self.renew(&mut signin, updates, now)?;
            let token = self.token.clone().ok_or(ApiError::Unauthorized)?;
            let result = query(self, &token);
            if result.as_ref().is_err_and(|err| err.needs_signin()) {
                self.token = None;
                self.auth_blocked = true;
            }
            return result;
        }
        result
    }

    fn graphql_url(&self) -> String {
        format!("{}/admin/graphql", self.endpoint.trim_end_matches('/'))
    }

    fn signin_url(&self) -> String {
        format!("{}/admin/auth/signin", self.endpoint.trim_end_matches('/'))
    }
}

/// Signals the worker thread accepts.
#[derive(Debug, Clone)]
pub enum Command {
    /// Password sign-in for one account (the token method signs in directly,
    /// without a round trip).
    SignIn {
        account: String,
        creds: Credentials,
    },
    /// Replace the whole polled set: accounts were added, removed or re-signed.
    SetTargets(Vec<Target>),
    RefreshNow,
    /// Adopt a new row count (the UI derives it from the window height).
    SetRowLimit(i64),
    /// Fetch one request's executions, for its detail popup. The request is
    /// fetched from the account that listed it.
    FetchDetail {
        account: String,
        id: String,
    },
    /// Ask one account for a single request, purely to report whether its
    /// credentials still work. The answer never reaches the list.
    TestAccount {
        account: String,
    },
    /// Restrict the polled list with the console's `modelID` filter. `None`
    /// clears it and the next poll is the unfiltered page again.
    SetModelFilter(Option<String>),
    /// Restrict the polled list with the console's `statusIn` filter. `None`
    /// omits the field, so every status comes back.
    SetStatusFilter(Option<Vec<String>>),
    Shutdown,
}

pub struct Worker {
    commands: Sender<Command>,
    updates: Receiver<Update>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl Worker {
    pub fn spawn(config: Config, targets: Vec<Target>) -> Self {
        let (cmd_tx, cmd_rx) = std::sync::mpsc::channel::<Command>();
        let (upd_tx, upd_rx) = std::sync::mpsc::channel::<Update>();

        let handle = std::thread::Builder::new()
            .name("ah-panel-poll".into())
            .spawn(move || run(config, targets, cmd_rx, upd_tx))
            .ok();

        Worker {
            commands: cmd_tx,
            updates: upd_rx,
            handle,
        }
    }

    pub fn send(&self, command: Command) {
        let _ = self.commands.send(command);
    }

    /// Non-blocking drain of everything produced since the last call.
    pub fn drain(&self) -> Vec<Update> {
        let mut out = Vec::new();
        loop {
            match self.updates.try_recv() {
                Ok(update) => out.push(update),
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => break,
            }
        }
        out
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.commands.send(Command::Shutdown);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

/// What one poll cycle produced, before it is turned into updates.
#[derive(Default)]
struct Cycle {
    rows: Vec<Row>,
    /// Accounts that answered.
    open: Vec<String>,
    /// Accounts that failed, with a short reason.
    broken: Vec<AccountIssue>,
}

/// The worker's poll list: one target per saved account, with whatever token
/// the encrypted blob holds for it and, in password mode, its saved password.
pub fn targets_from(config: &Config, stored: &Stored) -> Vec<Target> {
    config
        .accounts
        .iter()
        .map(|account| Target::from_account(account, stored, config.credential_mode))
        .collect()
}

/// Shared exponential retry cadence, bounded even when another account works.
fn next_backoff(previous: Duration) -> Duration {
    if previous.is_zero() {
        Duration::from_secs(2)
    } else {
        (previous * 2).min(Duration::from_secs(30))
    }
}

/// Apply one command to the worker's live state. Returns `true` when the
/// worker should exit (`Shutdown`).
#[allow(clippy::too_many_arguments)]
fn apply_command(
    command: Command,
    config: &mut Config,
    targets: &mut Vec<Target>,
    next_poll: &mut Instant,
    force: &mut bool,
    pending_creds: &mut Option<(String, Credentials)>,
    pending_detail: &mut Option<(String, String)>,
    pending_probe: &mut Option<String>,
    id_filter: &mut Option<String>,
    status_filter: &mut Option<Vec<String>>,
) -> bool {
    match command {
        Command::Shutdown => return true,
        Command::RefreshNow => {
            *next_poll = Instant::now();
            *force = true;
        }
        Command::SetRowLimit(limit) => {
            config.row_limit = limit;
            *next_poll = Instant::now();
            *force = true;
        }
        Command::SetTargets(next) => {
            *targets = next;
            *next_poll = Instant::now();
            *force = true;
        }
        Command::SignIn { account, creds } => {
            *pending_creds = Some((account, creds));
            *next_poll = Instant::now();
            *force = true;
        }
        Command::FetchDetail { account, id } => *pending_detail = Some((account, id)),
        Command::TestAccount { account } => *pending_probe = Some(account),
        Command::SetModelFilter(id) => {
            *id_filter = id;
            *next_poll = Instant::now();
            *force = true;
        }
        Command::SetStatusFilter(statuses) => {
            *status_filter = statuses;
            *next_poll = Instant::now();
            *force = true;
        }
    }
    false
}

fn run(
    mut config: Config,
    mut targets: Vec<Target>,
    commands: Receiver<Command>,
    updates: Sender<Update>,
) {
    let client = Client::new(REQUEST_TIMEOUT);
    let signin_client = Client::new(SIGNIN_TIMEOUT);
    let detail_client = Client::new(DETAIL_TIMEOUT);

    let mut backoff = Duration::ZERO;
    let mut force = true;
    let mut pending_creds: Option<(String, Credentials)> = None;
    let mut pending_detail: Option<(String, String)> = None;
    let mut pending_probe: Option<String> = None;
    let mut id_filter: Option<String> = None;
    let mut status_filter: Option<Vec<String>> = None;
    let mut next_poll = Instant::now();
    // Emit the signed-out notice once per transition rather than every idle
    // tick, which would otherwise force a pointless repaint each second.
    let mut announced_signed_out = false;
    // Last reported account health, so `Update::Accounts` only goes out when
    // something actually changed.
    let mut reported: Option<(Vec<String>, Vec<AccountIssue>)> = None;

    loop {
        let mut shutdown = false;
        while let Ok(command) = commands.try_recv() {
            if apply_command(
                command,
                &mut config,
                &mut targets,
                &mut next_poll,
                &mut force,
                &mut pending_creds,
                &mut pending_detail,
                &mut pending_probe,
                &mut id_filter,
                &mut status_filter,
            ) {
                shutdown = true;
            }
        }
        if shutdown {
            return;
        }

        // Served inline, ahead of the poll: the user is looking at the popup,
        // and a click deserves the answer even if the list refresh slips a
        // cycle. One request at a time, so a click storm cannot stack up.
        if let Some((account, id)) = pending_detail.take() {
            let result = match targets
                .iter()
                .find(|t| t.id == account)
                .and_then(|t| t.token.clone())
            {
                Some(_) => {
                    let target = targets
                        .iter_mut()
                        .find(|t| t.id == account)
                        .expect("checked");
                    let signin_url = target.signin_url();
                    let graphql_url = target.graphql_url();
                    let project_id = target.project_id.clone();
                    target.authorized(
                        |creds| signin_client.sign_in(&signin_url, &creds.email, &creds.password),
                        |_target, current| {
                            detail_client.fetch_request_executions(
                                &graphql_url,
                                current,
                                &project_id,
                                &id,
                            )
                        },
                        &updates,
                        crate::time::now_unix(),
                        Instant::now(),
                    )
                }
                None => Err(ApiError::Unauthorized),
            };
            if let Err(e) = &result {
                warn!("获取请求详情失败 (id={}): {}", id, e.message());
            }
            let _ = updates.send(Update::Detail { id, result });
        }

        // Served inline too: the user is watching the settings window for the
        // answer, and the test is a single request. A token the poll already
        // dropped counts as "no credentials" rather than a failed round trip.
        if let Some(account) = pending_probe.take() {
            let result = match targets.iter_mut().find(|t| t.id == account) {
                Some(target) if target.can_authenticate() => {
                    let signin_url = target.signin_url();
                    let graphql_url = target.graphql_url();
                    let project_id = target.project_id.clone();
                    target
                        .authorized(
                            |creds| {
                                signin_client.sign_in(&signin_url, &creds.email, &creds.password)
                            },
                            |_target, token| {
                                client
                                    .fetch_requests(&graphql_url, token, &project_id, 1)
                                    .map(|(_, total)| total)
                            },
                            &updates,
                            crate::time::now_unix(),
                            Instant::now(),
                        )
                        .map_err(|e| e.message())
                }
                _ => Err("该账号没有可用凭据，请重新登录".into()),
            };
            if let Err(message) = &result {
                warn!("测试账号失败 ({}): {}", account, message);
            }
            let _ = updates.send(Update::Probe {
                id: account,
                result,
            });
        }

        if let Some((account, creds)) = pending_creds.take() {
            let signin_url = targets
                .iter()
                .find(|t| t.id == account)
                .map(Target::signin_url);
            match signin_url {
                Some(url) => {
                    let result = signin_client.sign_in(&url, &creds.email, &creds.password);
                    match result {
                        Ok(response) => {
                            if let Some(target) = targets.iter_mut().find(|t| t.id == account) {
                                target.accept_signin(&response);
                                // A password entered by hand may sign in even
                                // when it is not persisted; use it for renewal.
                                target.set_credentials(creds.clone());
                            }
                            backoff = Duration::ZERO;
                            let _ = updates.send(Update::SignedIn {
                                account: account.clone(),
                                creds,
                                token: response.token.clone(),
                                user: Box::new(response),
                            });
                        }
                        Err(err) => {
                            warn!("登录失败: {}", err.message());
                            let _ = updates.send(Update::Failed(err));
                        }
                    }
                }
                None => {
                    let _ = updates.send(Update::Failed(ApiError::Unauthorized));
                }
            }
            next_poll = Instant::now();
            force = true;
        }

        if force || Instant::now() >= next_poll {
            force = false;
            let mut cycle = Cycle::default();
            let mut polled = false;

            let statuses: Option<Vec<&str>> = status_filter
                .as_ref()
                .map(|values| values.iter().map(String::as_str).collect());
            for target in targets.iter_mut() {
                if !target.can_authenticate() {
                    // Without credentials the account is skipped below and would
                    // fall out of the health report, wiping the red settings
                    // button while the account is still unusable. Keep it in
                    // the broken list until it signs in again.
                    cycle.broken.push(AccountIssue {
                        id: target.id.clone(),
                        name: target.name.clone(),
                        message: "缺少令牌,需重新登录".into(),
                        needs_signin: true,
                    });
                    continue;
                }
                polled = true;
                let signin_url = target.signin_url();
                let graphql_url = target.graphql_url();
                let project_id = target.project_id.clone();
                match target.authorized(
                    |creds| signin_client.sign_in(&signin_url, &creds.email, &creds.password),
                    |_target, token| {
                        client.fetch_requests_where(
                            &graphql_url,
                            token,
                            &project_id,
                            config.row_limit,
                            id_filter.as_deref(),
                            statuses.as_deref(),
                        )
                    },
                    &updates,
                    crate::time::now_unix(),
                    Instant::now(),
                ) {
                    Ok((requests, _total)) => {
                        cycle.rows.extend(
                            crate::client::rows_from(&requests)
                                .into_iter()
                                .map(|row| row.with_account(&target.id, &target.name)),
                        );
                        cycle.open.push(target.id.clone());
                    }
                    Err(err) => {
                        warn!(
                            "轮询 AxonHub 请求失败 (账号 {}): {}",
                            target.name,
                            err.message()
                        );
                        cycle.broken.push(AccountIssue {
                            id: target.id.clone(),
                            name: target.name.clone(),
                            message: err.message(),
                            needs_signin: err.needs_signin(),
                        });
                    }
                }
            }

            // Recomputed every cycle: a busy gateway is polled faster, and
            // going idle must return to the slow cadence.
            let any_active = cycle.rows.iter().any(|r| r.status.is_active());
            if cycle.open.is_empty() {
                // Nothing answered. Grow the backoff so a down server cannot
                // produce a request storm, and keep whatever is on screen —
                // an empty replacement list would look like "no requests".
                if polled {
                    backoff = if backoff.is_zero() {
                        Duration::from_secs(2)
                    } else {
                        (backoff * 2).min(Duration::from_secs(30))
                    };
                }
                if !polled && !announced_signed_out {
                    let _ = updates.send(Update::SignedOut);
                    announced_signed_out = true;
                }
            } else {
                announced_signed_out = false;
                backoff = Duration::ZERO;
                let _ = updates.send(Update::Snapshot {
                    rows: std::mem::take(&mut cycle.rows),
                });
            }

            let health = (cycle.open.clone(), cycle.broken.clone());
            if reported.as_ref() != Some(&health) {
                let _ = updates.send(Update::Accounts {
                    open: cycle.open,
                    broken: cycle.broken,
                });
                reported = Some(health);
            }

            // A failed renewal has its own (short) retry delay. Waiting the full
            // poll interval would leave the account dark for up to 30s after a
            // blip, even though the password was never rejected.
            let auth_retry = targets
                .iter()
                .filter(|t| !t.auth_blocked && t.credentials.is_some() && t.token.is_none())
                .filter_map(|t| t.auth_retry_at)
                .min()
                .map(|at| at.saturating_duration_since(Instant::now()));
            let interval = if let Some(retry) = auth_retry {
                retry.max(Duration::from_secs(1))
            } else if !backoff.is_zero() {
                backoff
            } else {
                let secs = if any_active {
                    config.active_poll_seconds
                } else {
                    config.poll_seconds
                };
                Duration::from_secs(secs.max(1))
            };
            next_poll = Instant::now() + interval;
        }

        let wait = next_poll
            .saturating_duration_since(Instant::now())
            .clamp(Duration::from_millis(20), TICK);
        let deadline = Instant::now() + wait;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break;
            }
            match commands.recv_timeout(remaining.min(TICK)) {
                Ok(command) => {
                    if apply_command(
                        command,
                        &mut config,
                        &mut targets,
                        &mut next_poll,
                        &mut force,
                        &mut pending_creds,
                        &mut pending_detail,
                        &mut pending_probe,
                        &mut id_filter,
                        &mut status_filter,
                    ) {
                        return;
                    }
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return,
            }
        }
    }
}

#[cfg(test)]
#[path = "tests/worker.rs"]
mod tests;
