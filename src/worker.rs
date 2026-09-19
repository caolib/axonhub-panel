//! Background polling and authentication.
//!
//! All network work happens off the UI thread. Results come back as plain
//! values that the UI applies from its own timer, so painting never blocks on a
//! socket and the panel stays responsive when AxonHub is slow or unreachable.
//!
//! Every saved account is polled in turn and their requests are merged into a
//! single list. Each request carries the account it came from, so the UI can
//! label it.

use std::sync::mpsc::{RecvTimeoutError, Receiver, Sender, TryRecvError};
use std::time::{Duration, Instant};

use crate::client::{ApiError, Client, SignInResponse};
use crate::config::{Config, Credentials};
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
        token: String,
        user: Box<SignInResponse>,
    },
    /// A fresh page of AxonHub requests, merged across every account.
    Snapshot {
        rows: Vec<Row>,
        total: i64,
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
}

impl Target {
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
    total: i64,
    /// Accounts that answered.
    open: Vec<String>,
    /// Accounts that failed, with a short reason.
    broken: Vec<AccountIssue>,
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
                Some(current) => {
                    let target = targets.iter().find(|t| t.id == account).expect("checked");
                    detail_client.fetch_request_executions(
                        &target.graphql_url(),
                        &current,
                        &target.project_id,
                        &id,
                    )
                }
                None => Err(ApiError::Unauthorized),
            };
            if let Err(e) = &result {
                warn!("获取请求详情失败 (id={}): {}", id, e.message());
            }
            let _ = updates.send(Update::Detail { id, result });
        }

        if let Some((account, creds)) = pending_creds.take() {
            let signin_url = targets
                .iter()
                .find(|t| t.id == account)
                .map(Target::signin_url);
            match signin_url {
                Some(url) => match signin_client.sign_in(&url, &creds.email, &creds.password) {
                    Ok(response) => {
                        if let Some(target) = targets.iter_mut().find(|t| t.id == account) {
                            target.token = Some(response.token.clone());
                        }
                        backoff = Duration::ZERO;
                        let _ = updates.send(Update::SignedIn {
                            token: response.token.clone(),
                            user: Box::new(response),
                        });
                    }
                    Err(err) => {
                        warn!("登录失败: {}", err.message());
                        let _ = updates.send(Update::Failed(err));
                    }
                },
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

            for target in targets.iter_mut() {
                let Some(token) = target.token.clone() else {
                    continue;
                };
                polled = true;
                match client.fetch_requests(
                    &target.graphql_url(),
                    &token,
                    &target.project_id,
                    config.row_limit,
                ) {
                    Ok((requests, total)) => {
                        cycle.total += total;
                        cycle.rows.extend(
                            crate::client::rows_from(&requests)
                                .into_iter()
                                .map(|row| row.with_account(&target.id, &target.name)),
                        );
                        cycle.open.push(target.id.clone());
                    }
                    Err(err) => {
                        warn!("轮询 AxonHub 请求失败 (账号 {}): {}", target.name, err.message());
                        let needs_signin = err.needs_signin();
                        if needs_signin {
                            // The token is worse than useless: drop it so the
                            // next cycle does not keep retrying it, and let the
                            // UI offer a re-sign-in for this account.
                            target.token = None;
                        }
                        cycle.broken.push(AccountIssue {
                            id: target.id.clone(),
                            name: target.name.clone(),
                            message: err.message(),
                            needs_signin,
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
                    total: cycle.total,
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

            let interval = if !backoff.is_zero() {
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
