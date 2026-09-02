//! The attention engine: what the daemon says a Session needs.

mod engine;
mod session;

pub use engine::{Derived, Horizons, Observed, derive};
pub use session::{Acknowledgement, Item, ItemEnd, SessionAttention, Transition};
