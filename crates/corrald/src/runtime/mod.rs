//! Managed runtime: the processes and PTYs `corrald` owns.
//!
//! The ownership split with the PTY backend is fixed by the PR3 plan grill
//! (`docs/decisions/2026-08-24-pr3-plan-grill.md`, Q1). `portable-pty` owns
//! the PTY platform mechanism — allocation, controlling-terminal setup, spawn
//! plumbing, I/O, resize, and the platform's EIO/EOF behaviour. This module
//! owns managed-runtime lifecycle semantics: what a launch request must
//! satisfy before a process is created, process-group identity capture,
//! reaping, teardown policy, and Run lifecycle truth.
//!
//! One distinction is load-bearing enough to name here, because losing it
//! silently corrupts durable facts: *a command that never exec'd* and *a
//! command that exec'd and later exited* are different outcomes. The vendored
//! backend patch keeps the second from impersonating the first
//! (`third_party/portable-pty/CORRAL_PATCHES.md`).

mod attach;
mod launch;
mod occurrence;
mod session;
mod snapshot;
mod spawn;
mod stream;
mod terminal;

pub use attach::{
    ATTACH_TOKEN_TTL, AttachGrant, AttachRefused, AttachToken, AttachTokens, NoRandomness,
};
pub use launch::{LaunchRejection, LaunchRequest};
pub use occurrence::{
    ADVISORY_SHARE, Integrity, OBSERVATION_QUEUE, ObservedRuns, RunObservations, RunOccurrence,
    Weight, observe_runs,
};
pub use session::{
    Attachment, ExecutionState, InputRefused, ManagedSession, ManagedSessions, PendingSession,
    ResizeRefused, ScreenUnreadable, SessionGone, SessionHandle, StartError, TerminalAccess,
    spawn_session,
};
pub use snapshot::{
    SNAPSHOT_CEILING_BYTES, SNAPSHOT_SCROLLBACK_ROWS, SNAPSHOT_TARGET_BYTES, Snapshot,
    SnapshotBudget, SnapshotError, encode, encode_within,
};
pub use spawn::{
    ChildReaper, ImpossibleGeometry, MAX_TERMINAL_COLS, MAX_TERMINAL_ROWS, ManagedTerminal,
    PtyGeometry, SpawnError, SpawnedRuntime, TeardownWindow, spawn,
};
pub use stream::{
    Delivery, Desynchronised, SUBSCRIBER_QUEUE_BYTES, SUBSCRIBER_QUEUE_FRAMES, TerminalStream,
    Viewer,
};
pub use terminal::{AuthoritativeTerminal, DeviceReply, Poisoned, RETAINED_SCROLLBACK_BYTES};
