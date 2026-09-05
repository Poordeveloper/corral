#![forbid(unsafe_code)]

//! Corral's Desktop: the first graphical session, attention, and control
//! surface.
//!
//! A client of `corrald` like the CLI and the TUI, through the same
//! `corral-client`: it renders what the daemon reports and derives no state
//! of its own (`AGENTS.md` §Client / daemon boundary). What it adds is a
//! replica — a terminal emulator of its own, rebuilt from the daemon's
//! snapshots and deltas — and a window to paint it in.
//!
//! Everything that can be proved without a window lives in plain modules:
//! `replica` (the client half of ADR 0003 and ADR 0017), `input` (what a
//! keystroke means under the replica's modes), `bridge` (the tokio thread
//! the UI never waits on), `sessions` (the list model) and `actions` (what
//! the person may ask for, and of which daemon). The views — `app`,
//! `terminal`, `terminal_element`, `text_field`, `disclosure` — render them.

pub mod actions;
pub mod app;
pub mod bridge;
pub mod disclosure;
pub mod input;
pub mod quit;
pub mod replica;
pub mod sessions;
pub mod terminal;
pub mod terminal_element;
pub mod text_field;
pub mod theme;
pub mod tray;
