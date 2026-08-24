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

mod launch;
mod pump;
mod spawn;
mod terminal;

pub use launch::{LaunchRejection, LaunchRequest};
pub use pump::{PumpEnd, pump};
pub use spawn::{PtyGeometry, SpawnError, SpawnedRuntime, spawn};
pub use terminal::{AuthoritativeTerminal, DeviceReply, RETAINED_SCROLLBACK_BYTES};
