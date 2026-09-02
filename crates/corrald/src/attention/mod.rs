//! The attention engine: what the daemon says a Session needs (ADR 0015).
//!
//! `engine` is the pure derivation, `session` remembers one Session's last
//! state and item, `ledger` holds every Session's claims for the daemon,
//! `journal` records what changed, and `tick` is the daemon's clock over all
//! of it. Clients render; nothing here is theirs to compute.

mod engine;
mod journal;
mod ledger;
mod sealing;
mod session;
mod tick;

pub use engine::{Derived, Horizons, Observed, derive};
pub use journal::{Budget, DisputeRecord, Journal, Record, Report, TransitionRecord, report};
pub use ledger::{Change, Ledger};
pub use sealing::hook_fact_claim;
pub use session::{Acknowledgement, Item, ItemEnd, SessionAttention, Transition};
pub use tick::tick_until_shutdown;
