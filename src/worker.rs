//! Background polling and authentication.
//!
//! All network work happens off the UI thread. Results come back as plain
//! values that the UI applies from its own timer, so painting never blocks on a
//! socket and the panel stays responsive when AxonHub is slow or unreachable.

use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::time::{Duration, Instant};

use crate::client::{ApiError, Client, SignInResponse};
use crate::config::{Config, Credentials};
use crate::model::Row;
use crate::octopus;

/// Upper bound on a requests query before the panel gives up on it.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(12);
const SIGNIN_TIMEOUT: Duration = Duration::from_secs(15);
/// Longest sleep, so queued commands are noticed promptly without spinning.
const TICK: Duration = Duration::from_millis(200);

/// Connection state of the Octopus source, surfaced in the context menu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OctopusHealth {
    /// No endpoint configured; the second source is off.
    Disabled,
    /// Endpoint configured but no cookie pasted yet.
    MissingToken,
    /// The last read completed.
    Connected,
    /// The last read failed; the message is already user-facing.
    Error(String),
}

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
    /// The whole Octopus list; its stream always carries complete records, so
    /// each read replaces the previous set.
    OctopusRows(Vec<Row>),
    /// Octopus connection state changed; sent on transitions only.
    OctopusHealth(OctopusHealth),
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
    /// Adopt a pasted Octopus cookie (or clear it with `None`).
    SetOctopusToken(Option<String>),
    Pause(bool),
    RefreshNow,
    /// Adopt a new row count (the UI derives it from the window height).
    SetRowLimit(i64),
    Shutdown,
}

pub struct Worker {
    commands: Sender<Command>,
    updates: Receiver<Update>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl Worker {
    pub fn spawn(config: Config, token: Option<String>, octopus_token: Option<String>) -> Self {
        let (cmd_tx, cmd_rx) = std::sync::mpsc::channel::<Command>();
        let (upd_tx, upd_rx) = std::sync::mpsc::channel::<Update>();

        let handle = std::thread::Builder::new()
            .name("ah-panel-poll".into())
            .spawn(move || run(config, token, octopus_token, cmd_rx, upd_tx))
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
    mut octopus_token: Option<String>,
    commands: Receiver<Command>,
    updates: Sender<Update>,
) {
    let client = Client::new(REQUEST_TIMEOUT);
    let signin_client = Client::new(SIGNIN_TIMEOUT);
    let octopus_client = octopus::Client::new(octopus::WINDOW);

    let mut paused = false;
    let mut backoff = Duration::ZERO;
    let mut force = true;
    let mut pending_creds: Option<Credentials> = None;
    let mut next_poll = Instant::now();
    // Emit the signed-out notice once per transition rather than every idle
    // tick, which would otherwise force a pointless repaint each second.
    let mut announced_signed_out = false;
    // Same for the Octopus connection state: report transitions only.
    let mut octopus_health: Option<OctopusHealth> = None;

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
                Command::SetOctopusToken(value) => {
                    octopus_token = value;
                    // Clearing must also clear the stale rows; a replacement
                    // token just gets re-read on the next poll.
                    if octopus_token.is_none() {
                        let _ = updates.send(Update::OctopusRows(Vec::new()));
                    }
                    octopus_health = None;
                    next_poll = Instant::now();
                    force = true;
                }
                Command::SignIn { creds } => {
                    pending_creds = Some(creds);
                    next_poll = Instant::now();
                    force = true;
                }
            }
        }
        if shutdown {
            return;
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
            // Recomputed from both sources every cycle: a busy gateway is
            // polled faster, and going idle must return to the slow cadence.
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

            // Octopus is independent of AxonHub's token and health: it runs on
            // the same cadence but its failures never gate the main source.
            let next_health = if config.octopus_endpoint.trim().is_empty() {
                Some(OctopusHealth::Disabled)
            } else {
                match octopus_token.as_deref() {
                    None => Some(OctopusHealth::MissingToken),
                    Some(cookie) => {
                        match octopus_client.fetch_overview(&config.octopus_endpoint, cookie) {
                            Ok(rows) => {
                                // A running request keeps the faster cadence
                                // even when AxonHub's page has gone quiet.
                                any_active |= rows.iter().any(|r| r.status.is_active());
                                let _ = updates.send(Update::OctopusRows(rows));
                                Some(OctopusHealth::Connected)
                            }
                            Err(err) => {
                                let message = match err {
                                    ApiError::Expired | ApiError::Unauthorized => {
                                        "令牌无效或已过期,请重新粘贴".to_string()
                                    }
                                    other => other.message(),
                                };
                                Some(OctopusHealth::Error(message))
                            }
                        }
                    }
                }
            };
            if let Some(health) = next_health {
                announce_octopus(&updates, &mut octopus_health, health);
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

/// Report an Octopus health change, skipping repeats so a persistent failure
/// does not repaint the panel every tick.
fn announce_octopus(
    updates: &Sender<Update>,
    last: &mut Option<OctopusHealth>,
    next: OctopusHealth,
) {
    if last.as_ref() != Some(&next) {
        *last = Some(next.clone());
        let _ = updates.send(Update::OctopusHealth(next));
    }
}
