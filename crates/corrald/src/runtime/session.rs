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
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU8, AtomicU32, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::time::Instant;

use corral_core::{CorralSessionId, ExitCause, OccurrenceTime, RunEnd, RunId};
use corral_protocol::terminal::{Epoch, Sequence};

use super::launch::LaunchRequest;
use super::occurrence::{RunObservations, RunOccurrence};
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
    /// The PTY closed. Carries how the child ended, or `None` when the reaper
    /// could not establish that at all.
    Finished(Option<ExitCause>),
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
    /// Where the deltas after this snapshot arrive, or `None` when there will
    /// be none.
    ///
    /// A finished run's screen is a value, not an actor (ADR 0007 L2): the
    /// snapshot is the whole of it, and a viewer that waited on a stream which
    /// can never produce again would either hang or resync in a loop.
    pub viewer: Option<super::stream::Viewer>,
}

/// What the screen thread publishes for readers that cannot ask it.
///
/// Written by that thread alone and read by anyone. Listing sessions and
/// answering a finished one both happen on the daemon's one reactor thread,
/// where a round trip to a screen would put every other connection behind
/// whatever that session happens to be doing.
#[derive(Clone)]
struct Published {
    /// The screen thread's own view of execution.
    execution: Arc<AtomicU8>,
    /// The last geometry it applied, packed into one atomic so a reader never
    /// sees a row from one moment and a column from another.
    geometry: Arc<AtomicU32>,
    /// Set once, as that thread's last act (ADR 0007 L2).
    screen: Arc<OnceLock<FinalScreen>>,
}

/// The screen a run left behind, published when its screen thread ends.
///
/// Everything a finished session can still be asked, answered without a thread
/// to ask: the emulator, its scrollback, and the thread that owned them are
/// released at the moment the runtime ends, and this is what survives them
/// (ADR 0007 L1, L2).
struct FinalScreen {
    snapshot: Result<Snapshot, SnapshotError>,
    epoch: Epoch,
    sequence: Sequence,
    geometry: PtyGeometry,
    title: Option<Vec<u8>>,
}

/// The screen exists and cannot be read.
///
/// Its parser failed on provider output, so the structure behind it is not
/// something anyone may look at. A separate answer from `SessionGone`: the
/// runtime is still there.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScreenUnreadable;

/// Why a session did not take input.
///
/// Two answers, because a client must act on them differently: a run that
/// ended still has a screen worth looking at, and a runtime that is no longer
/// answering does not (ADR 0007 L3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputRefused {
    /// This run has ended. Its screen is a record now; nothing reads
    /// keystrokes.
    RunEnded,
    /// Corral can no longer reach this session's runtime and cannot say why.
    RuntimeGone,
}

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
    /// This run has ended, so there is no terminal left to give a size to.
    RunEnded,
}

/// The handle a daemon holds to one running session.
pub struct SessionHandle {
    session: CorralSessionId,
    run: RunId,
    title: String,
    asks: SyncSender<Ask>,
    started_at: Instant,
    /// Weak proof that the screen thread is still there.
    ///
    /// The published state below is only as current as the thread that writes
    /// it. If that thread is gone the value is a fact about a past nobody can
    /// extend, so this is checked first and the answer becomes Unknown —
    /// losing the ability to manage a runtime is not evidence about a process
    /// (ADR 0002, grill Q5).
    alive: std::sync::Weak<()>,
    published: Published,
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

    /// This session's screen, live or as its run left it.
    ///
    /// Answered from the record once the runtime has ended: the screen thread
    /// is gone by then, and a snapshot of a finished run is exactly what it
    /// published on its way out (ADR 0007 L2).
    pub fn snapshot(&self) -> Result<Result<Snapshot, SnapshotError>, SessionGone> {
        let (reply, answer) = sync_channel(1);
        match self
            .ask(Ask::Snapshot(reply))
            .and_then(|()| answer.recv().map_err(|_| SessionGone))
        {
            Ok(snapshot) => Ok(snapshot),
            Err(SessionGone) => Ok(self.recorded()?.snapshot.clone()),
        }
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
        match self
            .ask(Ask::Resize(geometry, reply))
            .and_then(|()| answer.recv().map_err(|_| SessionGone))
        {
            Ok(outcome) => Ok(outcome),
            // A run that ended has no terminal to give a size to, and saying
            // so is a different fact from a runtime that stopped answering.
            Err(SessionGone) => {
                self.recorded()?;
                Ok(Err(ResizeRefused::RunEnded))
            }
        }
    }

    /// Write bytes a client's replica encoded.
    ///
    /// The daemon does not interpret them: the client owns input encoding
    /// because only it knows its replica's live mode bits (`ARCHITECTURE.md`
    /// §3).
    pub fn write_input(&self, bytes: Vec<u8>) -> Result<(), InputRefused> {
        match self.ask(Ask::Input(bytes)) {
            Ok(()) => Ok(()),
            Err(SessionGone) if self.recorded().is_ok() => Err(InputRefused::RunEnded),
            Err(SessionGone) => Err(InputRefused::RuntimeGone),
        }
    }

    /// The last size the screen published, without asking it.
    ///
    /// For callers on the daemon's reactor thread, where waiting on a screen
    /// would block every other connection.
    pub fn last_geometry(&self) -> PtyGeometry {
        unpack_geometry(&self.published.geometry)
    }

    /// The screen's size, or why there is none to state.
    ///
    /// Three outcomes, deliberately distinct: a size, a screen that can no
    /// longer be read, and a runtime that is not answering at all.
    pub fn geometry(&self) -> Result<Result<PtyGeometry, ScreenUnreadable>, SessionGone> {
        let (reply, answer) = sync_channel(1);
        match self
            .ask(Ask::Geometry(reply))
            .and_then(|()| answer.recv().map_err(|_| SessionGone))
        {
            Ok(geometry) => Ok(geometry),
            Err(SessionGone) => Ok(Ok(self.recorded()?.geometry)),
        }
    }

    pub fn title_from_screen(&self) -> Result<Option<Vec<u8>>, SessionGone> {
        let (reply, answer) = sync_channel(1);
        match self
            .ask(Ask::Title(reply))
            .and_then(|()| answer.recv().map_err(|_| SessionGone))
        {
            Ok(title) => Ok(title),
            Err(SessionGone) => Ok(self.recorded()?.title.clone()),
        }
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

    /// Join this session's terminal stream, or read what its run left.
    ///
    /// While the run is live the snapshot and the delta stream are minted in
    /// one step on the thread that owns the screen, so no output can slip
    /// between them — a viewer that missed the bytes written while its
    /// snapshot was being encoded would render a screen that never existed.
    pub fn attach(&self) -> Result<Attachment, SessionGone> {
        let (reply, answer) = sync_channel(1);
        match self
            .ask(Ask::Attach(reply))
            .and_then(|()| answer.recv().map_err(|_| SessionGone))
        {
            Ok(attachment) => Ok(attachment),
            // A finished run's screen is the whole of what it has: there is
            // no stream to join, and saying so is what stops a viewer from
            // waiting on deltas that can never come (ADR 0007 L2).
            Err(SessionGone) => {
                let recorded = self.recorded()?;
                Ok(Attachment {
                    snapshot: recorded.snapshot.clone(),
                    epoch: recorded.epoch,
                    sequence: recorded.sequence,
                    viewer: None,
                })
            }
        }
    }

    /// What the daemon can currently claim about this Run's execution.
    ///
    /// `Exited` only once the child has actually been reaped, and it stays
    /// `Exited`: the screen thread retires the moment it publishes an end, and
    /// a fact that has been established is not unestablished by its publisher
    /// leaving (ADR 0007 L3).
    ///
    /// Reads a published value rather than asking, so listing sessions never
    /// waits on a screen that is busy.
    pub fn execution_state(&self) -> ExecutionState {
        match self.published.execution.load(Ordering::Acquire) {
            // Terminal facts. Nothing can un-exit, and nothing can make an
            // unestablished end establishable later, so no later event can
            // make either stale — including the screen thread retiring, which
            // is what publishing one of these leads to (ADR 0007 L3).
            EXECUTION_EXITED => ExecutionState::Exited,
            EXECUTION_UNKNOWN => ExecutionState::Unknown,
            // A claim about the present, extended only by the thread that
            // publishes it. If that thread is gone the value describes a past
            // nobody can extend, and losing the ability to manage a runtime is
            // not evidence about a process (ADR 0002, grill Q5).
            EXECUTION_RUNNING if self.alive.upgrade().is_some() => ExecutionState::Running,
            // Everything else: a live claim whose publisher is gone, and any
            // byte this module does not write. `Running` is the one answer a
            // fallthrough must never give — it is the only value here that
            // asserts a process exists, and a default that asserts is how an
            // unknown becomes a lie (AGENTS.md §Runtime truth).
            _ => ExecutionState::Unknown,
        }
    }

    /// The screen this run left behind, or why there is none.
    ///
    /// `Err` means the screen thread is gone without having published one: a
    /// loss rather than a retirement, and the one case that is genuinely
    /// `SessionGone` (ADR 0007 L3).
    fn recorded(&self) -> Result<&FinalScreen, SessionGone> {
        self.published.screen.get().ok_or(SessionGone)
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
        // proof it is there, exactly as its ending would. The record stays
        // empty, which is what makes this a loss and not a retirement — a
        // thread that ended on purpose publishes one first (ADR 0007 L3).
        self.alive = std::sync::Weak::new();
    }
}

/// A managed runtime that exists and whose Run is not yet a durable fact.
///
/// The gap between the two is deliberate and load-bearing. A concrete runtime
/// occurrence has to exist before `RunStarted` may be written at all, and that
/// start has to commit before anything can produce the `RunEnded` answering it
/// — so between the two there is a spawned session nobody is serving yet
/// (grill Q3, Q9).
///
/// Everything that can fail has already happened by the time one of these
/// exists. What is left is starting threads, which cannot fail, so a Run whose
/// start committed is always served.
pub struct PendingSession {
    screen: super::spawn::ManagedTerminal,
    reaper: super::spawn::ChildReaper,
    teardown: Arc<super::spawn::TeardownWindow>,
    reader: Box<dyn std::io::Read + Send>,
    writer: Box<dyn std::io::Write + Send>,
    geometry: PtyGeometry,
    title: String,
}

/// Create a managed runtime, without yet serving it.
pub fn spawn_session(
    request: &LaunchRequest,
    geometry: PtyGeometry,
) -> Result<PendingSession, StartError> {
    let runtime = super::spawn::spawn(request, geometry).map_err(StartError::Spawn)?;

    // Captured before the split, while the runtime still knows the pid it
    // created: after the child is reaped that number may belong to something
    // else, and before setsid the tty cannot say it at all.
    let teardown = Arc::new(super::spawn::TeardownWindow::open(runtime.child_group()));
    let (screen, mut reaper) = runtime.split();
    // The child is already running. If its own handles cannot be taken there
    // is no way to manage it, and leaving it is worse than never having
    // started it: it would be alive, unreachable, and never reaped.
    let handles = screen.reader().and_then(|reader| {
        let writer = screen.writer()?;
        Ok((reader, writer))
    });
    let (reader, writer) = match handles {
        Ok(handles) => handles,
        Err(error) => {
            teardown.hang_up();
            teardown.close();
            let _ = reaper.wait();
            return Err(StartError::Terminal(error));
        }
    };

    Ok(PendingSession {
        screen,
        reaper,
        teardown,
        reader,
        writer,
        geometry,
        title: request.display_title(),
    })
}

impl PendingSession {
    pub fn title(&self) -> &str {
        &self.title
    }

    /// End a runtime whose Run never became a durable fact.
    ///
    /// The child is already running, so it is hung up and reaped here rather
    /// than left alive, unreachable and unlistable. No occurrence is reported:
    /// with no durable `RunStarted` there is no Run in the durable model to
    /// end, and reporting one would ask the store to close an episode it never
    /// opened (grill Q9).
    pub fn abandon(self) {
        let Self {
            screen,
            mut reaper,
            teardown,
            reader,
            writer,
            ..
        } = self;

        teardown.hang_up();
        // Two endings, not one. The hang-up is a signal a child may choose to
        // ignore — the group teardown says so in as many words — and a child
        // that ignored it while Corral still held the pty master open would
        // never see its terminal close either. Dropping the master is the
        // second ending: a child reading its terminal reaches EOF, which is
        // the shape an interactive agent is almost always in. A child that
        // ignores the signal *and* never touches its terminal survives both,
        // and is left to the same limitation ADR 0007 L6 already states —
        // which is why nothing waits on this.
        drop(reader);
        drop(writer);
        drop(screen);
        // Closed before the wait, by the only party that waits (ADR 0007 L4).
        teardown.close();
        let _ = reaper.wait();
    }

    /// Own this runtime's screen and watch its end.
    pub fn serve(
        self,
        session: CorralSessionId,
        run: RunId,
        observations: RunObservations,
    ) -> SessionHandle {
        let Self {
            screen,
            reaper,
            teardown,
            reader,
            mut writer,
            geometry,
            title,
        } = self;

        // Bounded so a client that floods input cannot make the daemon
        // allocate without limit; generous because the screen thread drains it
        // in a tight loop and the PTY reader shares it.
        let (asks, questions) = sync_channel(256);
        let published = Published {
            execution: Arc::new(AtomicU8::new(EXECUTION_RUNNING)),
            geometry: Arc::new(AtomicU32::new(pack_geometry(geometry))),
            screen: Arc::new(OnceLock::new()),
        };

        // Writing to a PTY blocks when the child stops reading, and the child
        // stops reading when its output is not drained — so a write on the
        // thread that drains output can deadlock a session against itself. It
        // gets its own thread and its own bounded queue: a client that floods
        // a child that is not listening loses its keystrokes, and nothing else
        // stops.
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
        let reaping = Arc::clone(&teardown);
        // Reading the PTY blocks, and so does reaping the child, so both live
        // off the thread that answers questions about the screen. A child that
        // closes its terminal and keeps running would otherwise freeze the
        // session while Corral waited for an exit that had not happened.
        std::thread::spawn(move || {
            read_pty(reader, reaper, &reaping, from_pty, run, &observations)
        });

        // Dropped when the screen thread ends, however it ends — a normal
        // return or a panic — so nobody has to remember to publish that it is
        // gone.
        let alive = Arc::new(());
        let held = Arc::clone(&alive);
        let weak = Arc::downgrade(&alive);
        drop(alive);

        let serving = published.clone();
        std::thread::spawn(move || {
            let _alive = held;
            serve_screen(screen, &teardown, to_child, geometry, questions, &serving)
        });

        SessionHandle {
            session,
            run,
            title,
            asks,
            started_at: Instant::now(),
            alive: weak,
            published,
        }
    }
}

/// Carry PTY output to the screen, then establish how the child ended.
///
/// EINTR is retried rather than read as the end: a signal delivered to the
/// daemon is not the child closing its terminal, and treating it as one would
/// report an exit that never happened.
fn read_pty(
    mut reader: Box<dyn std::io::Read + Send>,
    mut reaper: super::spawn::ChildReaper,
    teardown: &super::spawn::TeardownWindow,
    asks: SyncSender<Ask>,
    run: RunId,
    observations: &RunObservations,
) {
    let mut buffer = [0_u8; 8192];
    let mut screen_gone = false;
    loop {
        match std::io::Read::read(&mut reader, &mut buffer) {
            Ok(0) => break,
            Ok(read) => {
                if asks.send(Ask::Output(buffer[..read].to_vec())).is_err() {
                    screen_gone = true;
                    break;
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        }
    }

    if screen_gone {
        // Nothing is left to render this session and nothing will drain its
        // terminal again, so a child left running would fill the pty buffer
        // and block on a write forever — alive, unreachable, unlistable. That
        // is the case already ruled on for a session the registry refused, and
        // it is the same fact arriving by another route (ADR 0007).
        teardown.hang_up();
    }

    // Closed before the wait, not after: from here the group number stops
    // being the child's, and this is the only party that can say so
    // (ADR 0007 L4).
    teardown.close();

    // The terminal closed, so the child is finishing. Reaping is what turns
    // that into a fact: without it the daemon could never say more than that
    // it stopped hearing anything — and the process would stay a zombie.
    let ended = reaper.wait().ok();

    // Reported here rather than from the screen thread, because this is the
    // one party that establishes the ending: the screen may already have been
    // lost, or retired on a shut-down ask, and an end nobody reported is a
    // durable Run that stays open forever.
    //
    // The instant is the reap's, and it is authoritative rather than merely
    // observed: `wait` returns when the child ends, and the kernel's own exit
    // status is the evidence. What Corral does not have is a time for an
    // ending it could not establish at all, and that is `Unknown` rather than
    // now (ADR 0002 D6).
    observations.report(match ended {
        Some(cause) => RunOccurrence::Exited {
            run,
            end: RunEnd::Exited(cause),
            at: OccurrenceTime::Authoritative(std::time::SystemTime::now()),
        },
        None => RunOccurrence::Exited {
            run,
            end: RunEnd::Unverifiable,
            at: OccurrenceTime::Unknown,
        },
    });

    if !screen_gone {
        let _ = asks.send(Ask::Finished(ended));
    }
}

/// The thread that owns one session's screen for the life of its runtime.
///
/// Everything reaches it as an `Ask` on one channel, so it blocks until there
/// is something to do. Reaping the child happens on the reader's thread rather
/// than here: a child that closes its terminal without exiting would otherwise
/// freeze every question about the session — and, because callers wait on the
/// daemon's single reactor thread, freeze the daemon with it.
fn serve_screen(
    screen: super::spawn::ManagedTerminal,
    teardown: &super::spawn::TeardownWindow,
    to_child: SyncSender<Vec<u8>>,
    geometry: PtyGeometry,
    questions: Receiver<Ask>,
    published: &Published,
) {
    let mut terminal = super::terminal::AuthoritativeTerminal::new(geometry);
    let mut stream = super::stream::TerminalStream::new();

    // Runs until the runtime ends, and no longer. Everything this thread
    // exists for — consuming output, answering device queries with nobody
    // attached, serialising reflow against writes — needs bytes that can still
    // arrive. When they cannot, the screen it holds is published as a value
    // and this returns, releasing the emulator, the stream, and the pty
    // (ADR 0007 L2). The registry keeps the record; nothing keeps the thread.
    while let Ok(ask) = questions.recv() {
        match ask {
            Ask::Output(chunk) => {
                let reply = terminal.consume(&chunk);
                let sequence = stream.advance();
                // Not delivered once the screen is poisoned: the daemon can no
                // longer say what these bytes did, so a viewer replaying them
                // would be building a screen nothing vouches for while every
                // attach and resync on the same session is refused.
                if terminal.poisoned().is_none() {
                    stream.deliver(sequence, &chunk);
                }
                if !reply.is_empty() {
                    // Queued, never written here: a child that has stopped
                    // reading must not be able to stop this loop.
                    let _ = to_child.try_send(reply.as_bytes().to_vec());
                }
                // The child can reshape the screen without anyone asking the
                // pty: DECCOLM (`ESC[?3h`) makes the emulator 132 columns
                // wide by itself. A shape change is an epoch boundary whoever
                // caused it — a viewer holding a snapshot at the old width
                // renders every wrapped line wrong, and `terminal.attach`
                // would answer a size the screen no longer has.
                if let Some(shape) = terminal.geometry()
                    && shape != unpack_geometry(&published.geometry)
                {
                    published
                        .geometry
                        .store(pack_geometry(shape), Ordering::Release);
                    stream.open_epoch();
                }
            }
            Ask::Finished(status) => {
                published.execution.store(
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
                // Published before this thread's liveness proof drops with its
                // stack, so a handle that finds the thread gone and a record
                // present is looking at a retirement and not a loss
                // (ADR 0007 L3).
                //
                // `set` cannot already have been taken: this arm is the only
                // writer and it returns.
                let _ = published.screen.set(FinalScreen {
                    snapshot: terminal.snapshot(),
                    epoch: stream.epoch(),
                    sequence: stream.next_sequence(),
                    // From the emulator while it can still be read; a poisoned
                    // screen has no size to state and the last one published
                    // is the one the child actually had.
                    geometry: terminal
                        .geometry()
                        .unwrap_or_else(|| unpack_geometry(&published.geometry)),
                    title: terminal.title().map(<[u8]>::to_vec),
                });
                // Returning is what releases everything: the emulator and the
                // delta stream with this stack, the pty master with `screen`,
                // and the writer thread with the last sender into it.
                return;
            }
            Ask::Snapshot(reply) => {
                let _ = reply.send(terminal.snapshot());
            }
            Ask::Resize(wanted, reply) => {
                let outcome = apply_resize(&screen, &mut terminal, &mut stream, wanted);
                if outcome.is_ok() {
                    published
                        .geometry
                        .store(pack_geometry(wanted), Ordering::Release);
                }
                let _ = reply.send(outcome);
            }
            Ask::Input(input) => {
                // Dropped rather than queued without bound when the child is
                // not reading: losing keystrokes a child refuses to take is
                // better than a session that answers nothing at all. A run
                // that has ended never reaches here — the handle answers that
                // from its record (ADR 0007 L2).
                let _ = to_child.try_send(input);
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
                // Whether the group is still the child's is not this thread's
                // to judge: `Finished` is observed when it is dequeued, not
                // when the reap happened, so a ShutDown queued in between
                // would otherwise signal a pid that had already been released.
                // The window itself knows (ADR 0007 L4).
                teardown.hang_up();
                return;
            }

            Ask::Attach(reply) => {
                let _ = reply.send(Attachment {
                    snapshot: terminal.snapshot(),
                    epoch: stream.epoch(),
                    sequence: stream.next_sequence(),
                    viewer: Some(stream.attach()),
                });
            }
        }

        // One owner for the consequence, whichever entrance poisoned the
        // screen (ADR 0007 L5): a screen nobody can vouch for serves no
        // viewers. Dropping them ends their streams, and they are told by the
        // refusal they get when they come back.
        if terminal.poisoned().is_some() {
            stream.drop_viewers();
        }
    }
}

/// Rows and columns in one word, so a reader never sees half of each.
fn pack_geometry(geometry: PtyGeometry) -> u32 {
    (u32::from(geometry.rows()) << 16) | u32::from(geometry.cols())
}

/// The inverse. Only ever reads a word this module packed from a validated
/// geometry, which is why it can state one rather than return an option.
fn unpack_geometry(packed: &AtomicU32) -> PtyGeometry {
    let packed = packed.load(Ordering::Acquire);
    PtyGeometry::expect_valid((packed >> 16) as u16, packed as u16)
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
        // asked; the daemon has no opinion about ranking yet. Cached because
        // `sort_by_key` calls its key on every comparison, not once per
        // element — an id formatted O(n log n) times for a list that is
        // answered on every `session.list`.
        described.sort_by_cached_key(|session| session.session.to_string());
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
            Self::RunEnded => {
                f.write_str("this run has ended; its screen keeps the size it finished at")
            }
        }
    }
}

impl std::error::Error for ResizeRefused {}

impl std::fmt::Display for InputRefused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::RunEnded => f.write_str("this run has ended; nothing is reading its input"),
            Self::RuntimeGone => f.write_str("the session's runtime is no longer answering"),
        }
    }
}

impl std::error::Error for InputRefused {}

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
