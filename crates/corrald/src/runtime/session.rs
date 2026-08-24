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
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, AtomicU32, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::time::Instant;

use corral_core::{CorralSessionId, RunId};
use corral_protocol::terminal::{Epoch, Sequence};

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

/// Anything the thread that owns a session's screen must act on.
///
/// PTY output is one of these rather than a second channel polled beside
/// them: with one queue the thread blocks until something happens instead of
/// waking hundreds of times a second to find nothing, and the order of output
/// against questions is the order they arrived rather than something the loop
/// decides.
enum Ask {
    /// Bytes the PTY produced.
    Output(Vec<u8>),
    /// The PTY closed. Carries the exit status the reaper established, or
    /// `None` when it could not be established at all.
    Finished(Option<u32>),
    Snapshot(SyncSender<Result<Snapshot, SnapshotError>>),
    Resize(PtyGeometry, SyncSender<Result<Epoch, ResizeRefused>>),
    Input(Vec<u8>),
    Geometry(SyncSender<Result<PtyGeometry, ScreenUnreadable>>),
    Title(SyncSender<Option<Vec<u8>>>),
    /// End this session: hang up the terminal and stop serving.
    ShutDown,
    /// A viewer wants the stream. Answered with a snapshot and the end it
    /// reads deltas from, minted together so nothing can arrive between them.
    Attach(SyncSender<Attachment>),
}

/// What a viewer receives when it joins: the screen, and everything after it.
pub struct Attachment {
    pub snapshot: Result<Snapshot, SnapshotError>,
    pub epoch: Epoch,
    /// Where in the epoch this snapshot sits.
    ///
    /// Carried because the deltas that follow carry their real positions: a
    /// snapshot stamped zero after eight thousand chunks tells any client that
    /// checks for gaps that it just missed eight thousand frames.
    pub sequence: Sequence,
    pub viewer: super::stream::Viewer,
}

/// The screen exists and cannot be read.
///
/// Its parser failed on provider output, so the structure behind it is not
/// something anyone may look at. A separate answer from `SessionGone`: the
/// runtime is still there.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScreenUnreadable;

/// Why a terminal did not take a new size.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResizeRefused {
    /// The kernel would not resize the pty. The emulator is deliberately left
    /// alone: reflowing Corral's screen while the child keeps drawing to the
    /// old width would make the authoritative screen disagree with the
    /// terminal the child actually has.
    TerminalRefused,
    /// The screen may no longer be read or written at all.
    ScreenPoisoned,
}

/// The handle a daemon holds to one running session.
pub struct SessionHandle {
    session: CorralSessionId,
    run: RunId,
    title: String,
    asks: SyncSender<Ask>,
    started_at: Instant,
    /// The last geometry the screen thread published.
    ///
    /// Packed into one atomic so a reader gets a size that existed rather than
    /// a row from one moment and a column from another.
    published_geometry: Arc<AtomicU32>,
    /// Weak proof that the screen thread is still there.
    ///
    /// The published state below is only as current as the thread that writes
    /// it. If that thread is gone the value is a fact about a past nobody can
    /// extend, so this is checked first and the answer becomes Unknown —
    /// losing the ability to manage a runtime is not evidence about a process
    /// (ADR 0002, grill Q5).
    alive: std::sync::Weak<()>,
    /// The screen thread's own view of execution, written by it and read by
    /// anyone.
    ///
    /// A shared cell rather than a question, because listing sessions happens
    /// on the daemon's single reactor thread: asking each screen thread and
    /// waiting for its answer would put one blocking round trip per session in
    /// front of every other connection.
    execution: Arc<AtomicU8>,
}

/// `ExecutionState` as a byte, so the screen thread can publish it without a
/// lock anyone could hold.
const EXECUTION_RUNNING: u8 = 0;
const EXECUTION_EXITED: u8 = 1;
const EXECUTION_UNKNOWN: u8 = 2;

/// Why a session could not be started.
#[derive(Debug)]
pub enum StartError {
    Spawn(SpawnError),
    /// The pty was created and its own handles could not be taken.
    Terminal(std::io::Error),
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
    pub fn resize(
        &self,
        geometry: PtyGeometry,
    ) -> Result<Result<Epoch, ResizeRefused>, SessionGone> {
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

    /// The last size the screen published, without asking it.
    ///
    /// For callers on the daemon's reactor thread, where waiting on a screen
    /// would block every other connection.
    pub fn last_geometry(&self) -> PtyGeometry {
        let packed = self.published_geometry.load(Ordering::Acquire);
        PtyGeometry::expect_valid((packed >> 16) as u16, packed as u16)
    }

    /// The screen's size, or why there is none to state.
    ///
    /// Three outcomes, deliberately distinct: a size, a screen that can no
    /// longer be read, and a runtime that is not answering at all.
    pub fn geometry(&self) -> Result<Result<PtyGeometry, ScreenUnreadable>, SessionGone> {
        let (reply, answer) = sync_channel(1);
        self.ask(Ask::Geometry(reply))?;
        answer.recv().map_err(|_| SessionGone)
    }

    pub fn title_from_screen(&self) -> Result<Option<Vec<u8>>, SessionGone> {
        let (reply, answer) = sync_channel(1);
        self.ask(Ask::Title(reply))?;
        answer.recv().map_err(|_| SessionGone)
    }

    /// End this session's runtime.
    ///
    /// For the one path where a session was started and the daemon cannot take
    /// responsibility for it: a live child with no registry entry is
    /// unreachable and unlistable, which is worse than not having started it.
    pub fn shut_down(&self) {
        // Never waits for room. A queue full of pending output would otherwise
        // park whoever is tearing the session down — and the one caller runs
        // on the daemon's reactor thread. A screen too busy to hear this is a
        // screen still serving; the session outlives the failed registration
        // rather than the daemon stalling behind it.
        let _ = self.asks.try_send(Ask::ShutDown);
    }

    /// Join this session's terminal stream.
    ///
    /// The snapshot and the delta stream are minted in one step on the thread
    /// that owns the screen, so no output can slip between them — a viewer
    /// that missed the bytes written while its snapshot was being encoded
    /// would render a screen that never existed.
    pub fn attach(&self) -> Result<Attachment, SessionGone> {
        let (reply, answer) = sync_channel(1);
        self.ask(Ask::Attach(reply))?;
        answer.recv().map_err(|_| SessionGone)
    }

    /// What the daemon can currently claim about this Run's execution.
    ///
    /// `Exited` only once the child has actually been reaped. A screen thread
    /// that is gone leaves whatever it last published, and a handle whose
    /// channel is closed reads `Unknown` — because the daemon losing the
    /// ability to manage a runtime is not evidence that the process died
    /// (ADR 0002, grill Q5).
    ///
    /// Reads a published value rather than asking, so listing sessions never
    /// waits on a screen that is busy.
    pub fn execution_state(&self) -> ExecutionState {
        if self.alive.upgrade().is_none() {
            return ExecutionState::Unknown;
        }
        match self.execution.load(Ordering::Acquire) {
            EXECUTION_RUNNING => ExecutionState::Running,
            EXECUTION_EXITED => ExecutionState::Exited,
            _ => ExecutionState::Unknown,
        }
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
        // The screen thread being unreachable is the whole point: drop the
        // proof it is there, exactly as its ending would.
        self.alive = std::sync::Weak::new();
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
    // without limit; generous because the screen thread drains it in a tight
    // loop and the PTY reader shares it.
    let (asks, questions) = sync_channel(256);
    let execution = Arc::new(AtomicU8::new(EXECUTION_RUNNING));
    let published_geometry = Arc::new(AtomicU32::new(pack_geometry(geometry)));
    let title = request.display_title();

    // Captured before the split, while the runtime still knows the pid it
    // created: after the child is reaped that number may belong to something
    // else, and before setsid the tty cannot say it at all.
    let group = runtime.child_group();
    let (screen, mut reaper) = runtime.split();
    // The child is already running. If its own handles cannot be taken there
    // is no way to manage it, and leaving it is worse than never having
    // started it: it would be alive, unreachable, and never reaped.
    let handles = screen.reader().and_then(|reader| {
        let writer = screen.writer()?;
        Ok((reader, writer))
    });
    let (reader, mut writer) = match handles {
        Ok(handles) => handles,
        Err(error) => {
            if let Some(group) = group {
                screen.hang_up(group);
            }
            let _ = reaper.wait();
            return Err(StartError::Terminal(error));
        }
    };

    // Writing to a PTY blocks when the child stops reading, and the child
    // stops reading when its output is not drained — so a write on the thread
    // that drains output can deadlock a session against itself. It gets its
    // own thread and its own bounded queue: a client that floods a child that
    // is not listening loses its keystrokes, and nothing else stops.
    let (to_child, outbound) = sync_channel::<Vec<u8>>(64);
    std::thread::spawn(move || {
        while let Ok(bytes) = outbound.recv() {
            if std::io::Write::write_all(&mut writer, &bytes).is_err() {
                return;
            }
            let _ = std::io::Write::flush(&mut writer);
        }
    });

    let from_pty = asks.clone();
    // Reading the PTY blocks, and so does reaping the child, so both live off
    // the thread that answers questions about the screen. A child that closes
    // its terminal and keeps running would otherwise freeze the session while
    // Corral waited for an exit that had not happened.
    std::thread::spawn(move || read_pty(reader, reaper, from_pty));

    // Dropped when the screen thread ends, however it ends — a normal return
    // or a panic — so nobody has to remember to publish that it is gone.
    let alive = Arc::new(());
    let held = Arc::clone(&alive);
    let weak = Arc::downgrade(&alive);
    drop(alive);

    let published = Arc::clone(&execution);
    let sizes = Arc::clone(&published_geometry);
    std::thread::spawn(move || {
        let _alive = held;
        serve_screen(
            Some(screen),
            group,
            Some(to_child),
            geometry,
            questions,
            published,
            sizes,
        )
    });

    Ok(SessionHandle {
        session,
        run,
        title,
        asks,
        started_at: Instant::now(),
        published_geometry,
        alive: weak,
        execution,
    })
}

/// Carry PTY output to the screen, then establish how the child ended.
///
/// EINTR is retried rather than read as the end: a signal delivered to the
/// daemon is not the child closing its terminal, and treating it as one would
/// report an exit that never happened.
fn read_pty(
    mut reader: Box<dyn std::io::Read + Send>,
    mut reaper: super::spawn::ChildReaper,
    asks: SyncSender<Ask>,
) {
    let mut buffer = [0_u8; 8192];
    let mut screen_gone = false;
    loop {
        match std::io::Read::read(&mut reader, &mut buffer) {
            Ok(0) => break,
            Ok(read) => {
                if asks.send(Ask::Output(buffer[..read].to_vec())).is_err() {
                    // Nobody is left to show output to. The child still has to
                    // be reaped: returning here would leave it a zombie for the
                    // daemon's whole life.
                    screen_gone = true;
                    break;
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        }
    }

    // The terminal closed, so the child is finishing. Reaping is what turns
    // that into a fact: without it the daemon could never say more than that
    // it stopped hearing anything — and the process would stay a zombie.
    let ended = reaper.wait().ok();
    if !screen_gone {
        let _ = asks.send(Ask::Finished(ended));
    }
}

/// The thread that owns one session's screen for its whole life.
///
/// Everything reaches it as an `Ask` on one channel, so it blocks until there
/// is something to do. Reaping the child happens on the reader's thread rather
/// than here: a child that closes its terminal without exiting would otherwise
/// freeze every question about the session — and, because callers wait on the
/// daemon's single reactor thread, freeze the daemon with it.
fn serve_screen(
    mut screen: Option<super::spawn::ManagedTerminal>,
    group: Option<super::spawn::ChildGroup>,
    mut to_child: Option<SyncSender<Vec<u8>>>,
    geometry: PtyGeometry,
    questions: Receiver<Ask>,
    execution: Arc<AtomicU8>,
    published_geometry: Arc<AtomicU32>,
) {
    let mut terminal = super::terminal::AuthoritativeTerminal::new(geometry);
    let mut stream = super::stream::TerminalStream::new();

    // The screen outlives the process — a person who attaches after an agent
    // finished still needs to read what it left — but not forever. Once the
    // child has ended and nobody is watching, this thread returns, its handle
    // stops answering, and the registry drops the session. Without that a
    // daemon that ran one command never idles again, which is the
    // zero-background-by-default rule broken by the fix that kept it alive
    // while work was running.
    while let Ok(ask) = questions.recv() {
        match ask {
            Ask::Output(chunk) => {
                let reply = terminal.consume(&chunk);
                let sequence = stream.advance();
                // Not delivered once the screen is poisoned: the daemon can no
                // longer say what these bytes did, so a viewer replaying them
                // would be building a screen nothing vouches for while every
                // attach and resync on the same session is refused. Dropping
                // the viewers ends their streams, and they are told by the
                // refusal they get when they come back.
                if terminal.poisoned().is_none() {
                    stream.deliver(sequence, &chunk);
                } else {
                    stream.drop_viewers();
                }
                if !reply.is_empty() {
                    // Queued, never written here: a child that has stopped
                    // reading must not be able to stop this loop.
                    if let Some(to_child) = to_child.as_ref() {
                        let _ = to_child.try_send(reply.as_bytes().to_vec());
                    }
                }
            }
            Ask::Finished(status) => {
                // The child is gone, so the terminal it was on has no further
                // use: dropping the screen returns the master, and dropping
                // the writer's end lets that thread finish and return its dup
                // of the same descriptor. Both, or the fd stays open.
                screen = None;
                to_child = None;
                execution.store(
                    match status {
                        // An exit Corral watched happen.
                        Some(_) => EXECUTION_EXITED,
                        // The terminal closed and the exit could not be
                        // established. Not Exited: losing the ability to
                        // observe a process is not evidence that it ended
                        // (ADR 0002).
                        None => EXECUTION_UNKNOWN,
                    },
                    Ordering::Release,
                );
            }
            Ask::Snapshot(reply) => {
                let _ = reply.send(super::snapshot::encode(&terminal));
            }
            Ask::Resize(wanted, reply) => {
                let outcome = match screen.as_ref() {
                    Some(screen) => apply_resize(screen, &mut terminal, &mut stream, wanted),
                    // A terminal whose child has gone cannot be resized, and
                    // saying so is better than reflowing a screen nothing will
                    // redraw.
                    None => Err(ResizeRefused::TerminalRefused),
                };
                if outcome.is_ok() {
                    published_geometry.store(pack_geometry(wanted), Ordering::Release);
                }
                let _ = reply.send(outcome);
            }
            Ask::Input(input) => {
                // Dropped rather than queued without bound when the child is
                // not reading: losing keystrokes a child refuses to take is
                // better than a session that answers nothing at all.
                if let Some(to_child) = to_child.as_ref() {
                    let _ = to_child.try_send(input);
                }
            }
            Ask::Geometry(reply) => {
                // Always answered. Dropping the channel would report
                // SessionGone — "the runtime is no longer answering" — for a
                // runtime that is answering perfectly well and simply has no
                // readable screen. Those are different facts (AGENTS.md
                // §Runtime truth).
                let _ = reply.send(terminal.geometry().ok_or(ScreenUnreadable));
            }
            Ask::Title(reply) => {
                let _ = reply.send(terminal.title().map(<[u8]>::to_vec));
            }
            Ask::ShutDown => {
                // Only while the child is still ours to end: once it has been
                // reaped, its old group number may name something else.
                if let (Some(screen), Some(group)) = (screen.as_ref(), group) {
                    screen.hang_up(group);
                }
                return;
            }
            Ask::Attach(reply) => {
                let _ = reply.send(Attachment {
                    snapshot: super::snapshot::encode(&terminal),
                    epoch: stream.epoch(),
                    sequence: stream.next_sequence(),
                    viewer: stream.attach(),
                });
            }
        }
    }
}

/// Rows and columns in one word, so a reader never sees half of each.
fn pack_geometry(geometry: PtyGeometry) -> u32 {
    (u32::from(geometry.rows()) << 16) | u32::from(geometry.cols())
}

/// Resize the pty first, and reflow only if it took.
///
/// The order matters: if the kernel refuses, Corral's screen must keep the
/// size the child still believes in. A reflowed authoritative screen paired
/// with a child drawing to the old width disagrees on every line that wraps,
/// and the client would be told a new epoch began.
fn apply_resize(
    screen: &super::spawn::ManagedTerminal,
    terminal: &mut super::terminal::AuthoritativeTerminal,
    stream: &mut super::stream::TerminalStream,
    wanted: PtyGeometry,
) -> Result<Epoch, ResizeRefused> {
    if terminal.poisoned().is_some() {
        return Err(ResizeRefused::ScreenPoisoned);
    }
    screen
        .resize(wanted)
        .map_err(|_| ResizeRefused::TerminalRefused)?;
    terminal.resize(wanted);
    if terminal.poisoned().is_some() {
        return Err(ResizeRefused::ScreenPoisoned);
    }
    Ok(stream.open_epoch())
}

/// The sessions one daemon is running.
///
/// Handles are behind `Arc` so a caller can take one out from under the
/// registry lock before doing anything that waits. Asking a screen thread a
/// question while holding that lock, on the daemon's one reactor thread, would
/// put every other connection behind whatever that session happens to be
/// doing.
#[derive(Default)]
pub struct ManagedSessions {
    handles: HashMap<CorralSessionId, Arc<SessionHandle>>,
}

impl ManagedSessions {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, handle: SessionHandle) {
        self.handles.insert(handle.session, Arc::new(handle));
    }

    /// A handle that outlives the lock this was called under.
    pub fn get(&self, session: CorralSessionId) -> Option<Arc<SessionHandle>> {
        self.handles.get(&session).map(Arc::clone)
    }

    pub fn len(&self) -> usize {
        self.handles.len()
    }

    /// Sessions whose runtime is still running.
    ///
    /// This, not the total, is what holds the daemon open: the plan says live
    /// runs keep it busy, and counting finished ones would mean a daemon that
    /// ran one command never idles again — zero-background-by-default broken
    /// by the fix that stopped it exiting under live work.
    pub fn live(&self) -> usize {
        self.handles
            .values()
            .filter(|handle| handle.execution_state() == ExecutionState::Running)
            .count()
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
                execution_state: handle.execution_state(),
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
            Self::Terminal(error) => write!(f, "the pty could not be used: {error}"),
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

impl std::fmt::Display for ResizeRefused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TerminalRefused => {
                f.write_str("the terminal would not take that size; the session keeps the one it has")
            }
            Self::ScreenPoisoned => f.write_str(
                "this terminal's parser failed on provider output and its screen can no longer be read",
            ),
        }
    }
}

impl std::error::Error for ResizeRefused {}

impl std::fmt::Display for ScreenUnreadable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(
            "this terminal's parser failed on provider output and its screen can no longer be read",
        )
    }
}

impl std::error::Error for ScreenUnreadable {}

#[cfg(test)]
#[path = "session_tests.rs"]
mod tests;
