//! The managed sessions a daemon is running, and the truth about their Runs.
//!
//! Every session's screen lives on its own thread, because the emulator holds
//! raw pointers and cannot cross one. So this type never holds a screen: it
//! holds the handle to the thread that does, and asks it questions by message.
//! That is the shape the runtime wanted anyway — one owner of the PTY, one
//! owner of the screen, no lock a slow reader could hold.
//!
//! What this type does own is the lifecycle claim: whether a Run is running,
//! ended, or something the daemon can no longer establish. Those are the words
//! ADR 0002 fixed, and PR3 adds no new ones.

use std::collections::HashMap;
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::time::Instant;

use corral_core::{CorralSessionId, RunId};
use corral_protocol::terminal::Epoch;

use super::launch::LaunchRequest;
use super::snapshot::{Snapshot, SnapshotError};
use super::spawn::{PtyGeometry, SpawnError};

/// What the daemon can currently claim about a session's execution.
///
/// Not an assurance vocabulary and not attention vocabulary: this is the
/// execution dimension's own value set (grill Q3). `Unknown` means Corral
/// cannot make a reliable claim right now — never that a process is gone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExecutionState {
    Running,
    Exited,
    Unknown,
}

/// What one managed session is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ManagedSession {
    pub session: CorralSessionId,
    pub run: RunId,
    pub title: String,
    pub execution_state: ExecutionState,
}

/// A question for the thread that owns a session's screen.
enum Ask {
    Snapshot(SyncSender<Result<Snapshot, SnapshotError>>),
    Resize(PtyGeometry, SyncSender<Epoch>),
    Input(Vec<u8>),
    Geometry(SyncSender<PtyGeometry>),
    Title(SyncSender<Option<Vec<u8>>>),
    Execution(SyncSender<ExecutionState>),
}

/// The handle a daemon holds to one running session.
pub struct SessionHandle {
    session: CorralSessionId,
    run: RunId,
    title: String,
    asks: SyncSender<Ask>,
    started_at: Instant,
}

/// Why a session could not be started.
#[derive(Debug)]
pub enum StartError {
    Spawn(SpawnError),
}

/// Why a question to a session went unanswered.
///
/// One variant: the thread that owns the screen is gone. Distinguishing how it
/// went would invite a caller to treat some endings as recoverable, and none
/// of them are — the screen a question was about no longer exists.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SessionGone;

impl SessionHandle {
    pub fn session(&self) -> CorralSessionId {
        self.session
    }

    pub fn run(&self) -> RunId {
        self.run
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    pub fn started_at(&self) -> Instant {
        self.started_at
    }

    pub fn snapshot(&self) -> Result<Result<Snapshot, SnapshotError>, SessionGone> {
        let (reply, answer) = sync_channel(1);
        self.ask(Ask::Snapshot(reply))?;
        answer.recv().map_err(|_| SessionGone)
    }

    /// Apply a client's desired geometry, returning the epoch it opened.
    ///
    /// The last explicit resize wins. A client must never call this because it
    /// observed someone else's resize, or two viewers of different sizes would
    /// reassert forever (grill Q6).
    pub fn resize(&self, geometry: PtyGeometry) -> Result<Epoch, SessionGone> {
        let (reply, answer) = sync_channel(1);
        self.ask(Ask::Resize(geometry, reply))?;
        answer.recv().map_err(|_| SessionGone)
    }

    /// Write bytes a client's replica encoded.
    ///
    /// The daemon does not interpret them: the client owns input encoding
    /// because only it knows its replica's live mode bits (`ARCHITECTURE.md`
    /// §3).
    pub fn write_input(&self, bytes: Vec<u8>) -> Result<(), SessionGone> {
        self.ask(Ask::Input(bytes))
    }

    pub fn geometry(&self) -> Result<PtyGeometry, SessionGone> {
        let (reply, answer) = sync_channel(1);
        self.ask(Ask::Geometry(reply))?;
        answer.recv().map_err(|_| SessionGone)
    }

    pub fn title_from_screen(&self) -> Result<Option<Vec<u8>>, SessionGone> {
        let (reply, answer) = sync_channel(1);
        self.ask(Ask::Title(reply))?;
        answer.recv().map_err(|_| SessionGone)
    }

    /// What the daemon can currently claim about this Run's execution.
    ///
    /// `Exited` is returned only once the child has actually been reaped.
    /// A screen that has stopped answering yields `Unknown` at the caller,
    /// because the daemon losing the ability to manage a runtime is not
    /// evidence that the process died (ADR 0002, grill Q5).
    pub fn execution_state(&self) -> Result<ExecutionState, SessionGone> {
        let (reply, answer) = sync_channel(1);
        self.ask(Ask::Execution(reply))?;
        answer.recv().map_err(|_| SessionGone)
    }

    fn ask(&self, ask: Ask) -> Result<(), SessionGone> {
        self.asks.send(ask).map_err(|_| SessionGone)
    }

    /// Drop the only path to this session's screen.
    ///
    /// What a lost runtime looks like from the daemon's side, without asking a
    /// real process to disappear: the point under test is that Corral reports
    /// what it can no longer establish, not how the runtime was lost.
    #[cfg(test)]
    pub(crate) fn sever_for_test(&mut self) {
        let (severed, _) = sync_channel(1);
        self.asks = severed;
    }
}

/// Start a managed session: spawn the process, own its screen, serve it.
pub fn start(
    request: &LaunchRequest,
    geometry: PtyGeometry,
    session: CorralSessionId,
    run: RunId,
) -> Result<SessionHandle, StartError> {
    let runtime = super::spawn::spawn(request, geometry).map_err(StartError::Spawn)?;
    // Bounded so a client that floods input cannot make the daemon allocate
    // without limit; the bound is generous because the reader drains it in a
    // tight loop.
    let (asks, questions) = sync_channel(256);
    let title = request.display_title();

    std::thread::spawn(move || serve(runtime, geometry, questions));

    Ok(SessionHandle {
        session,
        run,
        title,
        asks,
        started_at: Instant::now(),
    })
}

/// The thread that owns one session's screen for its whole life.
fn serve(
    mut runtime: super::spawn::SpawnedRuntime,
    geometry: PtyGeometry,
    questions: Receiver<Ask>,
) {
    let mut terminal = super::terminal::AuthoritativeTerminal::new(geometry);
    let mut stream = super::stream::TerminalStream::new();
    let Ok(mut reader) = runtime.reader() else {
        return;
    };
    let Ok(mut writer) = runtime.writer() else {
        return;
    };

    // The PTY read blocks, so it runs on a second thread and hands bytes over.
    // Bounded, because an unbounded queue in front of a screen is just the
    // screen's memory growth wearing a different name.
    let (bytes, output) = sync_channel::<Vec<u8>>(64);
    std::thread::spawn(move || {
        let mut buffer = [0_u8; 8192];
        loop {
            match std::io::Read::read(&mut reader, &mut buffer) {
                Ok(0) | Err(_) => return,
                Ok(read) => {
                    if bytes.send(buffer[..read].to_vec()).is_err() {
                        return;
                    }
                }
            }
        }
    });

    // The screen outlives the process: a person who attaches after an agent
    // finished still needs to read what it left on screen, so this loop keeps
    // answering until every handle is gone rather than until the child is.
    let mut execution = ExecutionState::Running;

    loop {
        // Output first: a screen that lags its process would answer every
        // question about a past that has already moved.
        loop {
            match output.try_recv() {
                Ok(chunk) => {
                    let reply = terminal.consume(&chunk);
                    stream.advance();
                    if !reply.is_empty() {
                        let _ = std::io::Write::write_all(&mut writer, reply.as_bytes());
                        let _ = std::io::Write::flush(&mut writer);
                    }
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                // The terminal closed, so the process is finishing. Reap it:
                // an exit is a fact only once it has been observed, and
                // without this the daemon could never say more than Unknown.
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    if execution == ExecutionState::Running && runtime.wait().is_ok() {
                        execution = ExecutionState::Exited;
                    }
                    break;
                }
            }
        }

        match questions.recv_timeout(std::time::Duration::from_millis(4)) {
            Ok(Ask::Snapshot(reply)) => {
                let _ = reply.send(super::snapshot::encode(&terminal));
            }
            Ok(Ask::Resize(geometry, reply)) => {
                let _ = runtime.resize(geometry);
                terminal.resize(geometry);
                let _ = reply.send(stream.open_epoch());
            }
            Ok(Ask::Input(input)) => {
                let _ = std::io::Write::write_all(&mut writer, &input);
                let _ = std::io::Write::flush(&mut writer);
            }
            Ok(Ask::Geometry(reply)) => {
                // A screen that may no longer be read has no geometry to
                // state, and the caller's channel simply goes unanswered —
                // which its own error path already means "gone".
                if let Some(geometry) = terminal.geometry() {
                    let _ = reply.send(geometry);
                }
            }
            Ok(Ask::Title(reply)) => {
                let _ = reply.send(terminal.title().map(<[u8]>::to_vec));
            }
            Ok(Ask::Execution(reply)) => {
                let _ = reply.send(execution);
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            // Every handle is gone: nobody can ask this screen anything again.
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return,
        }
    }
}

/// The sessions one daemon is running.
#[derive(Default)]
pub struct ManagedSessions {
    handles: HashMap<CorralSessionId, SessionHandle>,
}

impl ManagedSessions {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, handle: SessionHandle) {
        self.handles.insert(handle.session, handle);
    }

    pub fn get(&self, session: CorralSessionId) -> Option<&SessionHandle> {
        self.handles.get(&session)
    }

    pub fn len(&self) -> usize {
        self.handles.len()
    }

    pub fn is_empty(&self) -> bool {
        self.handles.is_empty()
    }

    /// What this daemon can currently say about each session it runs.
    ///
    /// A session whose screen thread no longer answers is `Unknown`, never
    /// `Exited`: the thread being gone says the daemon lost the ability to
    /// manage the runtime, not that the process died (ADR 0002, grill Q5).
    pub fn describe(&self) -> Vec<ManagedSession> {
        let mut described: Vec<ManagedSession> = self
            .handles
            .values()
            .map(|handle| ManagedSession {
                session: handle.session,
                run: handle.run,
                title: handle.title.clone(),
                execution_state: handle.execution_state().unwrap_or(ExecutionState::Unknown),
            })
            .collect();
        // A stable order so a list is not a different list each time it is
        // asked; the daemon has no opinion about ranking yet.
        described.sort_by_key(|session| session.session.to_string());
        described
    }
}

impl ExecutionState {
    /// The wire spelling. Additive: a peer that meets a value it does not know
    /// keeps working, because absence and unknown are the same answer here.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Exited => "exited",
            Self::Unknown => "unknown",
        }
    }
}

impl std::fmt::Display for StartError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Spawn(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for StartError {}

impl std::fmt::Display for SessionGone {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("the session's runtime is no longer answering")
    }
}

impl std::error::Error for SessionGone {}

#[cfg(test)]
#[path = "session_tests.rs"]
mod tests;
