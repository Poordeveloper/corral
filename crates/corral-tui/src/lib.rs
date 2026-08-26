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

mod attach;
mod keys;
mod list;
mod presentation;
mod screen;

pub use attach::{Geometry, LocalKeys, OpenFailed, RawMode, open};
pub use list::{row_text, run, short_id};
pub use presentation::{MainState, SessionPresentation, present};
