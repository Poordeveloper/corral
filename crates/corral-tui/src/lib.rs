#![forbid(unsafe_code)]

//! Corral's terminal surfaces: the session list a person lives in, and the
//! attachment it hands the terminal over to.
//!
//! Both live here because they are the same terminal. Open is a takeover — the
//! list leaves the screen and the existing full-terminal attachment runs in its
//! place (grill Q1) — so a person's raw mode, their window size and the one
//! thread allowed to read their keyboard have exactly one owner between them.
//! Splitting the two would have meant two owners of each.
//!
//! `corral` drives this crate: `corral tui` runs the list, and `corral attach`
//! and `corral new` reach a session through the same attachment the list uses.
//! What a session is allowed to *say* is `presentation`'s, and the CLI renders
//! from there too, so the two surfaces cannot contradict each other about the
//! same session (grill Q2).
//!
//! This crate is a client. It renders what `corrald` reports and derives no
//! state of its own (`AGENTS.md` §Runtime truth).

/// How long this crate waits for `corrald` to answer one question.
///
/// A client's own patience, not a wire contract, and the same for every
/// question because the reason is the same for all of them: raw mode holds
/// `Ctrl-C` and `Ctrl-\`, so a surface waiting on a daemon that will never
/// answer is one a person cannot leave from the terminal they are at. Every
/// wait this crate takes is bounded by it.
pub(crate) const ANSWER: std::time::Duration = std::time::Duration::from_secs(5);

mod attach;
mod daemon;
mod keys;
mod launch;
mod list;
mod presentation;
mod screen;

pub use attach::{LocalKeys, OpenFailed, RawMode, open};
pub use launch::start_session;
pub use list::{run, short_id};
pub use presentation::{MainState, SessionPresentation, present};
