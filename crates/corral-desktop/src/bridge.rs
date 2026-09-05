//! The Desktop's road to `corrald`: a tokio thread the window never waits on.
//!
//! `corral-client` is async on tokio; gpui runs its own executor on the main
//! thread. The bridge owns one tokio runtime on a thread of its own, holds
//! the daemon connection there, and answers the window's questions through
//! channels both sides can drive: a request goes in, a `oneshot` comes back,
//! and a terminal channel's frames arrive on a stream the foreground awaits.
//!
//! Activation is `corral-client`'s, exactly as the CLI and TUI do it, so this
//! surface can never start a daemon on terms of its own (ADR 0001; round 2,
//! Q7). Dropping the bridge disconnects; it never stops corrald.

use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, Instant};

use corral_client::launch::{
    Continued, LaunchSite, Requested, Shown, continue_session, serves, start_session,
};
use corral_client::sessions::Listing;
use corral_client::{
    ClientActivationPolicy, Connection, EndpointSelection, RequestError, activate_at,
};
use corral_protocol::capability;
use corral_protocol::method::{AttentionSummaryResult, SessionNewResult};
use corral_protocol::terminal::{
    Epoch, FrameKind, MAX_CLIENT_FRAME_BYTES, Sequence, TerminalFrame,
};
use futures::SinkExt;
use futures::channel::{mpsc as foreground, oneshot};
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};
use tokio::sync::mpsc as background;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use crate::replica::{Geometry, Promised};

/// How long the Desktop waits for `corrald` to answer one question about
/// state that already exists. A client's own patience, not a wire contract.
pub const ANSWER: Duration = Duration::from_secs(5);

/// How long it waits for a session to start or continue. Longer, because the
/// daemon builds a PTY and spawns a child, which a loaded machine can stretch;
/// still bounded, because the connection cannot be asked anything else while
/// the answer is out.
pub const LAUNCH: Duration = Duration::from_secs(30);

/// A question's answer, arriving once. Cancelled if the bridge is gone.
pub type Reply<T> = oneshot::Receiver<T>;

/// What a question reports when the thread behind its [`Reply`] is gone.
pub const BRIDGE_STOPPED: &str = "the bridge to corrald stopped";

/// How much of the daemon's output may wait for the window before the socket
/// stops draining.
///
/// The window's room is the only bound on this side: reading past it turned
/// corrald's per-viewer budget into unbounded Desktop memory (post-merge
/// review of #51, finding 1). Once the socket stops draining, that budget is
/// what supersedes the backlog with a fresh prefix; a window that takes
/// nothing for the daemon's 2 s no-progress deadline loses the channel, as
/// any client does.
///
/// In bytes, because a frame is anything from a one-byte delta to a 16 MiB
/// snapshot, and a count of frames bounds nothing. The daemon's own number
/// (`SUBSCRIBER_QUEUE_BYTES`), for the same reason on its side of the
/// socket. A frame larger than the whole budget is admitted alone, so what
/// waits is never more than this budget or one frame past it, plus the one
/// frame the reader is still assembling.
const INBOUND_QUEUE_BYTES: u32 = 4 * 1024 * 1024;

/// How many frames may wait, whatever their size: a backstop under the byte
/// budget so a stream of one-byte deltas cannot queue four million of them.
/// Any frame of 256 bytes or more runs out of bytes first.
const INBOUND_QUEUE_FRAMES: usize = (INBOUND_QUEUE_BYTES / 256) as usize;

/// What the daemon's hello said it serves, as the actions read it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Capabilities {
    pub managed_sessions: bool,
    pub history_sessions: bool,
    pub attention: bool,
    pub geometry: bool,
    pub palette: bool,
}

impl Capabilities {
    fn of(connection: &Connection) -> Self {
        Self {
            managed_sessions: serves(connection, capability::MANAGED_SESSIONS),
            history_sessions: serves(connection, capability::HISTORY_SESSIONS),
            attention: serves(connection, capability::ATTENTION),
            geometry: serves(connection, capability::TERMINAL_GEOMETRY),
            palette: serves(connection, capability::TERMINAL_PALETTE),
        }
    }

    /// What a replica may expect around a snapshot.
    #[must_use]
    pub fn promised(self) -> Promised {
        Promised {
            geometry: self.geometry,
            palette: self.palette,
        }
    }
}

/// One poll generation: the list and the counts, published together only
/// because both succeeded (round 1, #3).
#[derive(Debug)]
pub struct Polled {
    pub listing: Listing,
    pub summary: AttentionSummaryResult,
    pub capabilities: Capabilities,
}

/// Why there is no answer.
///
/// Three different claims about the daemon, and the surface must not make the
/// wrong one: one that refused answered, and is on the other end of a
/// connection that is fine; one whose answer this build could not read also
/// answered; only the third may not be there at all (`AGENTS.md` §Runtime
/// truth).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Unanswered {
    Refused(String),
    Unreadable(String),
    Silent(String),
}

impl Unanswered {
    #[must_use]
    pub fn line(&self) -> String {
        match self {
            Self::Refused(detail) => format!("corrald refused: {detail}"),
            Self::Unreadable(detail) => {
                format!("corrald answered with something this build cannot read: {detail}")
            }
            Self::Silent(detail) => format!("corrald did not answer: {detail}"),
        }
    }
}

fn about(error: &RequestError) -> Unanswered {
    match error {
        RequestError::Refused(_) => Unanswered::Refused(error.to_string()),
        RequestError::Protocol { .. } => Unanswered::Unreadable(error.to_string()),
        RequestError::DaemonConnectionLost { .. } => Unanswered::Silent(error.to_string()),
    }
}

/// A terminal channel, opened: what the daemon said about the session's size
/// and what it promised, the frames it will send, and the way to send ours.
pub struct Attached {
    /// The session's size at the grant. Under a daemon that sends `Geometry`
    /// the snapshot brings its own; this is what the daemon has now.
    pub geometry: Geometry,
    pub promised: Promised,
    /// Every frame the daemon sends, in order, each with the room it takes.
    /// Ends when the channel does.
    pub inbound: foreground::Receiver<Delivery>,
    pub outbound: Outbound,
}

/// One frame on its way to the window, carrying the accounting for its own
/// size: only the window knows when it is done with a frame, and dropping
/// this is what returns the room (`INBOUND_QUEUE_BYTES`).
pub struct Delivery {
    pub frame: TerminalFrame,
    _room: OwnedSemaphorePermit,
}

/// The Desktop's half of one terminal channel.
///
/// Dropping it closes the channel: that is detaching, which the run survives.
pub struct Outbound(background::UnboundedSender<TerminalFrame>);

impl Outbound {
    /// Queue a frame for the daemon. `false` once the channel is gone.
    pub fn send(&self, frame: TerminalFrame) -> bool {
        self.0.send(frame).is_ok()
    }

    /// Queue input for the daemon, in frames it accepts. `false` once the
    /// channel is gone.
    pub fn input(&self, epoch: Epoch, bytes: &[u8]) -> bool {
        input_frames(epoch, bytes)
            .into_iter()
            .all(|frame| self.send(frame))
    }
}

/// Input as the frames a client may send. Input is one byte stream the daemon
/// writes in frame order, so a paste past the client ceiling crosses as
/// several frames rather than as the one oversize frame the daemon ends the
/// channel over (`MAX_CLIENT_FRAME_BYTES`); where a chunk boundary falls is
/// immaterial to a PTY.
fn input_frames(epoch: Epoch, bytes: &[u8]) -> Vec<TerminalFrame> {
    bytes
        .chunks(MAX_CLIENT_FRAME_BYTES)
        .map(|chunk| TerminalFrame {
            kind: FrameKind::Input,
            epoch,
            sequence: Sequence(0),
            payload: chunk.to_vec(),
        })
        .collect()
}

enum Request {
    Poll(oneshot::Sender<Result<Polled, Unanswered>>),
    Attach {
        session_id: String,
        reply: oneshot::Sender<Result<Attached, String>>,
    },
    Start {
        requested: Requested,
        site: LaunchSite,
        reply: oneshot::Sender<Result<SessionNewResult, String>>,
    },
    Continue {
        session_id: String,
        shown: Shown,
        working_directory: Option<PathBuf>,
        reply: oneshot::Sender<Result<Continued, String>>,
    },
    Acknowledge {
        session_id: String,
        attention_item_id: String,
        reply: oneshot::Sender<Result<(), String>>,
    },
}

/// The window's handle on the tokio thread.
pub struct Bridge {
    requests: background::UnboundedSender<Request>,
}

impl Bridge {
    /// Start the thread. Nothing is connected until the first question.
    pub fn start(policy: ClientActivationPolicy, endpoint: EndpointSelection) -> Self {
        let (requests, received) = background::unbounded_channel();
        let started = std::thread::Builder::new()
            .name("corral-bridge".to_owned())
            .spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build();
                match runtime {
                    Ok(runtime) => runtime.block_on(serve(policy, endpoint, received)),
                    // The receiver drops with this thread; every question
                    // then comes back cancelled, which the window reports.
                    Err(error) => eprintln!("corral-desktop: the bridge did not start: {error}"),
                }
            });
        if let Err(error) = started {
            eprintln!("corral-desktop: the bridge thread did not start: {error}");
        }
        Self { requests }
    }

    pub fn poll(&self) -> Reply<Result<Polled, Unanswered>> {
        let (reply, answer) = oneshot::channel();
        let _ = self.requests.send(Request::Poll(reply));
        answer
    }

    pub fn attach(&self, session_id: String) -> Reply<Result<Attached, String>> {
        let (reply, answer) = oneshot::channel();
        let _ = self.requests.send(Request::Attach { session_id, reply });
        answer
    }

    pub fn start_session(
        &self,
        requested: Requested,
        site: LaunchSite,
    ) -> Reply<Result<SessionNewResult, String>> {
        let (reply, answer) = oneshot::channel();
        let _ = self.requests.send(Request::Start {
            requested,
            site,
            reply,
        });
        answer
    }

    pub fn continue_session(
        &self,
        session_id: String,
        shown: Shown,
        working_directory: Option<PathBuf>,
    ) -> Reply<Result<Continued, String>> {
        let (reply, answer) = oneshot::channel();
        let _ = self.requests.send(Request::Continue {
            session_id,
            shown,
            working_directory,
            reply,
        });
        answer
    }

    pub fn acknowledge(
        &self,
        session_id: String,
        attention_item_id: String,
    ) -> Reply<Result<(), String>> {
        let (reply, answer) = oneshot::channel();
        let _ = self.requests.send(Request::Acknowledge {
            session_id,
            attention_item_id,
            reply,
        });
        answer
    }
}

/// The thread's whole life: one question at a time, on one connection.
async fn serve(
    policy: ClientActivationPolicy,
    endpoint: EndpointSelection,
    mut requests: background::UnboundedReceiver<Request>,
) {
    let mut daemon = Daemon {
        policy,
        endpoint,
        connection: None,
        retry: None,
    };
    while let Some(request) = requests.recv().await {
        match request {
            Request::Poll(reply) => {
                let _ = reply.send(daemon.poll().await);
            }
            Request::Attach { session_id, reply } => {
                let _ = reply.send(daemon.attach(&session_id).await);
            }
            Request::Start {
                requested,
                site,
                reply,
            } => {
                let started = daemon
                    .ask(LAUNCH, |connection| {
                        Box::pin(start_session(connection, requested, site))
                    })
                    .await
                    .map_err(|unanswered| unanswered.line());
                let _ = reply.send(started);
            }
            Request::Continue {
                session_id,
                shown,
                working_directory,
                reply,
            } => {
                let continued = daemon
                    .ask(LAUNCH, |connection| {
                        Box::pin(async move {
                            continue_session(
                                connection,
                                &session_id,
                                shown,
                                working_directory.as_deref(),
                                // This surface never answers in advance; it
                                // shows the disclosure and asks.
                                &mut |_| {},
                            )
                            .await
                        })
                    })
                    .await
                    .map_err(|unanswered| unanswered.line());
                let _ = reply.send(continued);
            }
            Request::Acknowledge {
                session_id,
                attention_item_id,
                reply,
            } => {
                let acknowledged = daemon
                    .ask(ANSWER, |connection| {
                        Box::pin(async move {
                            connection
                                .attention_acknowledge(&session_id, &attention_item_id)
                                .await
                        })
                    })
                    .await
                    .map_err(|unanswered| unanswered.line());
                let _ = reply.send(acknowledged);
            }
        }
    }
}

type Question<'c, T> = Pin<Box<dyn Future<Output = Result<T, RequestError>> + 'c>>;

/// The connection to `corrald`, and the ability to get another one.
///
/// A lost daemon is not a dead Desktop: the list says it cannot be read,
/// keeps asking, and picks up again when one answers. The TUI's shape
/// (`corral-tui::daemon`): the poll keeps its cadence; only starting a daemon
/// backs off, because a `corrald` that dies on startup leaves no owner behind
/// and a poll that activated every second would start one every second.
struct Daemon {
    policy: ClientActivationPolicy,
    endpoint: EndpointSelection,
    connection: Option<Connection>,
    retry: Option<Backoff>,
}

struct Backoff {
    failures: u32,
    until: Instant,
}

impl Backoff {
    const CEILING: Duration = Duration::from_secs(30);

    fn after(failures: u32) -> Self {
        let seconds = 1_u64 << failures.min(5);
        Self {
            failures: failures.saturating_add(1),
            until: Instant::now() + Duration::from_secs(seconds).min(Self::CEILING),
        }
    }

    fn waiting(&self) -> Option<Duration> {
        self.until.checked_duration_since(Instant::now())
    }
}

impl Daemon {
    /// A connection, activating one if there is none and the last attempt is
    /// far enough behind.
    async fn activated(&mut self) -> Result<Connection, Unanswered> {
        if let Some(waiting) = self.retry.as_ref().and_then(Backoff::waiting) {
            return Err(Unanswered::Silent(format!(
                "no corrald is running; trying again in {} seconds",
                waiting.as_secs().max(1)
            )));
        }
        let failures = self.retry.as_ref().map_or(0, |backoff| backoff.failures);
        self.retry = Some(Backoff::after(failures));

        match activate_at(&self.endpoint, &self.policy).await {
            Ok(connection) => {
                self.retry = None;
                Ok(connection)
            }
            Err(error) => Err(Unanswered::Silent(error.to_string())),
        }
    }

    /// Ask one question, within a budget.
    ///
    /// The connection is taken out for the question and put back only when
    /// it can be asked another: a refusal came on a connection that is fine;
    /// a lost daemon and a protocol fault did not, and a question that ran
    /// out of time left a socket whose next read is an answer nobody is
    /// holding.
    async fn ask<T, F>(&mut self, budget: Duration, question: F) -> Result<T, Unanswered>
    where
        F: for<'c> FnOnce(&'c mut Connection) -> Question<'c, T>,
    {
        let mut connection = match self.connection.take() {
            Some(connection) => connection,
            None => self.activated().await?,
        };
        match tokio::time::timeout(budget, question(&mut connection)).await {
            Err(_) => Err(Unanswered::Silent(format!(
                "nothing within {} seconds",
                budget.as_secs()
            ))),
            Ok(Ok(answer)) => {
                self.connection = Some(connection);
                Ok(answer)
            }
            Ok(Err(error)) => {
                if matches!(error, RequestError::Refused(_)) {
                    self.connection = Some(connection);
                }
                Err(about(&error))
            }
        }
    }

    async fn poll(&mut self) -> Result<Polled, Unanswered> {
        let listed = self
            .ask(ANSWER, |connection| Box::pin(connection.session_list()))
            .await?;
        // The counts, asked only of a daemon that just answered. Their failure
        // is the poll's: half an answer is not published beside a stale other
        // half (round 1, #3).
        let summary = self
            .ask(ANSWER, |connection| {
                Box::pin(connection.attention_summary())
            })
            .await?;
        let capabilities = self
            .connection
            .as_ref()
            .map(Capabilities::of)
            .unwrap_or_default();
        Ok(Polled {
            listing: Listing::of(listed),
            summary,
            capabilities,
        })
    }

    async fn attach(&mut self, session_id: &str) -> Result<Attached, String> {
        let session_id = session_id.to_owned();
        let grant = self
            .ask(ANSWER, move |connection| {
                Box::pin(async move { connection.terminal_attach(&session_id).await })
            })
            .await
            .map_err(|unanswered| unanswered.line())?;
        let promised = self
            .connection
            .as_ref()
            .map(Capabilities::of)
            .unwrap_or_default()
            .promised();

        // A second connection, not the RPC one: losing the data channel never
        // costs the ability to ask for another (ADR 0003).
        let opened = tokio::time::timeout(
            ANSWER,
            Connection::open_terminal_channel(self.endpoint.endpoint(), &grant.attach_token),
        )
        .await;
        let channel = match opened {
            Ok(Ok(channel)) => channel,
            Ok(Err(error)) => return Err(format!("the terminal channel did not open: {error}")),
            Err(_) => {
                return Err(format!(
                    "the terminal channel did not open within {} seconds",
                    ANSWER.as_secs()
                ));
            }
        };

        let (inbound, frames) = foreground::channel(INBOUND_QUEUE_FRAMES);
        let room = Arc::new(Semaphore::new(INBOUND_QUEUE_BYTES as usize));
        let (outbound, queued) = background::unbounded_channel();
        let (from_daemon, to_daemon) = channel.stream.into_split();
        tokio::spawn(read_channel(from_daemon, channel.leftover, inbound, room));
        tokio::spawn(write_channel(to_daemon, queued));

        Ok(Attached {
            geometry: Geometry {
                rows: grant.rows,
                cols: grant.cols,
            },
            promised,
            inbound: frames,
            outbound: Outbound(outbound),
        })
    }
}

/// Read the daemon's frames for as long as the channel lasts and the window
/// has room for them (`INBOUND_QUEUE_BYTES`, `INBOUND_QUEUE_FRAMES`).
///
/// Off the UI thread, so a frame's worth of UI latency never stalls the
/// socket; not past the window's room, so a stalled window is the daemon's
/// slow viewer — superseded, then dropped — rather than this process's
/// memory.
async fn read_channel(
    mut from_daemon: OwnedReadHalf,
    leftover: Vec<u8>,
    mut inbound: foreground::Sender<Delivery>,
    room: Arc<Semaphore>,
) {
    use tokio::io::AsyncReadExt;

    // Bytes that arrived with the handshake are already terminal frames.
    let mut pending = leftover;
    let mut buffer = vec![0_u8; 65536];
    loop {
        loop {
            match TerminalFrame::decode_from_daemon(&pending) {
                Ok(Some((frame, consumed))) => {
                    pending.drain(..consumed);
                    // A frame past the whole budget charges all of it, and so
                    // waits for the queue to empty and is admitted alone.
                    let charge = u32::try_from(frame.payload.len())
                        .map_or(INBOUND_QUEUE_BYTES, |len| len.min(INBOUND_QUEUE_BYTES));
                    let Ok(room) = room.clone().acquire_many_owned(charge).await else {
                        return;
                    };
                    let delivery = Delivery { frame, _room: room };
                    if inbound.send(delivery).await.is_err() {
                        return;
                    }
                }
                // Not yet a whole frame: the rest is still on its way.
                Ok(None) => break,
                // A fault is never "wait for more": the offending header
                // would never be consumed. The channel ends here.
                Err(_) => return,
            }
        }
        match from_daemon.read(&mut buffer).await {
            Ok(0) | Err(_) => return,
            Ok(read) => pending.extend_from_slice(&buffer[..read]),
        }
    }
}

/// Write the window's frames until it drops its half, then close ours: the
/// daemon reads end-of-stream and ends the channel, and the run lives on.
async fn write_channel(
    mut to_daemon: OwnedWriteHalf,
    mut queued: background::UnboundedReceiver<TerminalFrame>,
) {
    use tokio::io::AsyncWriteExt;

    while let Some(frame) = queued.recv().await {
        let Ok(bytes) = frame.encode() else {
            break;
        };
        if to_daemon.write_all(&bytes).await.is_err() {
            return;
        }
    }
    let _ = to_daemon.shutdown().await;
}

#[cfg(test)]
#[path = "bridge_tests.rs"]
mod tests;
