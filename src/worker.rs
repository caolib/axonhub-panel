//! Background polling and authentication.
//!
//! All network work happens off the UI thread. Results come back as plain
//! values that the UI applies from its own timer, so painting never blocks on a
//! socket and the panel stays responsive when AxonHub is slow or unreachable.

use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::time::{Duration, Instant};

use crate::client::{ApiError, Client, SignInResponse};
use crate::config::{Config, Credentials};
use crate::model::{ExecutionDetail, Row};

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
    /// A fresh page of AxonHub requests.
    Snapshot {
        rows: Vec<Row>,
        total: i64,
    },
    /// Answer to `FetchDetail`: the attempts recorded for one request.
    Detail {
        id: String,
        result: Result<Vec<ExecutionDetail>, ApiError>,
    },
    Failed(ApiError),
    /// No usable token; the UI must show the sign-in form.
    SignedOut,
}

/// Signals the worker thread accepts.
#[derive(Debug, Clone)]
pub enum Command {
    SignIn {
        creds: Credentials,
    },
    SetToken(String),
    Pause(bool),
    RefreshNow,
    /// Adopt a new row count (the UI derives it from the window height).
    SetRowLimit(i64),
    /// Fetch one request's executions, for its detail popup.
    FetchDetail(String),
    Shutdown,
}

pub struct Worker {
    commands: Sender<Command>,
    updates: Receiver<Update>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl Worker {
    pub fn spawn(config: Config, token: Option<String>) -> Self {
        let (cmd_tx, cmd_rx) = std::sync::mpsc::channel::<Command>();
        let (upd_tx, upd_rx) = std::sync::mpsc::channel::<Update>();

        let handle = std::thread::Builder::new()
            .name("ah-panel-poll".into())
            .spawn(move || run(config, token, cmd_rx, upd_tx))
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

fn run(
    mut config: Config,
    mut token: Option<String>,
    commands: Receiver<Command>,
    updates: Sender<Update>,
) {
    let client = Client::new(REQUEST_TIMEOUT);
    let signin_client = Client::new(SIGNIN_TIMEOUT);
    let detail_client = Client::new(DETAIL_TIMEOUT);

    let mut paused = false;
    let mut backoff = Duration::ZERO;
    let mut force = true;
    let mut pending_creds: Option<Credentials> = None;
    let mut pending_detail: Option<String> = None;
    let mut next_poll = Instant::now();
    // Emit the signed-out notice once per transition rather than every idle
    // tick, which would otherwise force a pointless repaint each second.
    let mut announced_signed_out = false;

    loop {
        let mut shutdown = false;
        while let Ok(command) = commands.try_recv() {
            match command {
                Command::Shutdown => shutdown = true,
                Command::Pause(value) => {
                    paused = value;
                    next_poll = Instant::now();
                    force = true;
                }
                Command::RefreshNow => {
                    next_poll = Instant::now();
                    force = true;
                }
                Command::SetRowLimit(limit) => {
                    config.row_limit = limit;
                    next_poll = Instant::now();
                    force = true;
                }
                Command::SetToken(value) => {
                    token = if value.is_empty() { None } else { Some(value) };
                    next_poll = Instant::now();
                    force = true;
                }
                Command::SignIn { creds } => {
                    pending_creds = Some(creds);
                    next_poll = Instant::now();
                    force = true;
                }
                Command::FetchDetail(id) => pending_detail = Some(id),
            }
        }
        if shutdown {
            return;
        }

        // Served inline, ahead of the poll: the user is looking at the popup,
        // and a click deserves the answer even if the list refresh slips a
        // cycle. One request at a time, so a click storm cannot stack up.
        if let Some(id) = pending_detail.take() {
            let result = match token.clone() {
                Some(current) => detail_client.fetch_request_executions(
                    &config.graphql_url(),
                    &current,
                    &config.project_id,
                    &id,
                ),
                None => Err(ApiError::Unauthorized),
            };
            let _ = updates.send(Update::Detail { id, result });
        }

        if let Some(creds) = pending_creds.take() {
            match signin_client.sign_in(&config.signin_url(), &creds.email, &creds.password) {
                Ok(response) => {
                    token = Some(response.token.clone());
                    backoff = Duration::ZERO;
                    let _ = updates.send(Update::SignedIn {
                        token: response.token.clone(),
                        user: Box::new(response),
                    });
                }
                Err(err) => {
                    let _ = updates.send(Update::Failed(err));
                }
            }
            next_poll = Instant::now();
            force = true;
        }

        if !paused && (force || Instant::now() >= next_poll) {
            force = false;
            // Recomputed every cycle: a busy gateway is polled faster, and
            // going idle must return to the slow cadence.
            let mut any_active = false;
            match token.clone() {
                None => {
                    // Nothing to poll until credentials arrive; wake up soon so
                    // a sign-in command is picked up promptly.
                    if !announced_signed_out {
                        let _ = updates.send(Update::SignedOut);
                        announced_signed_out = true;
                    }
                }
                Some(current) => {
                    announced_signed_out = false;
                    match client.fetch_requests(
                        &config.graphql_url(),
                        &current,
                        &config.project_id,
                        config.row_limit,
                    ) {
                        Ok((requests, total)) => {
                            let rows = crate::client::rows_from(&requests);
                            any_active = rows.iter().any(|r| r.status.is_active());
                            let _ = updates.send(Update::Snapshot { rows, total });
                            backoff = Duration::ZERO;
                        }
                        Err(err) => {
                            let needs_auth = err.needs_signin();
                            let _ = updates.send(Update::Failed(err));
                            if needs_auth {
                                token = None;
                                let _ = updates.send(Update::SignedOut);
                                announced_signed_out = true;
                            } else {
                                // Cap the backoff so a down server cannot
                                // produce a request storm.
                                backoff = if backoff.is_zero() {
                                    Duration::from_secs(2)
                                } else {
                                    (backoff * 2).min(Duration::from_secs(30))
                                };
                            }
                        }
                    }
                }
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
        std::thread::sleep(wait);
    }
}
