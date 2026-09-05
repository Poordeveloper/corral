#![forbid(unsafe_code)]

//! How every Corral surface reaches `corrald`.
//!
//! Activation is one state machine with one overall deadline: connect to the
//! canonical endpoint, and only on a failure that activation could repair ask
//! the singleton lock whether a primary daemon already exists. Absence of a
//! socket is never evidence that starting one is safe; only absence of a lock
//! owner is (ADR 0001 D3, D4).
//!
//! Surfaces consume this crate rather than reimplementing any part of it, so
//! that CLI, TUI, Desktop and Tray can never disagree about who is allowed to
//! start the user's daemon.
//!
//! The same holds for what a surface says. `presentation` is what any surface
//! may claim about a session, `sessions` how it reads the daemon's list, and
//! `launch` how it asks for a session: one copy of the vocabulary, in the
//! crate every surface already depends on, so two surfaces cannot contradict
//! each other about the same session or start subtly different ones from the
//! same words (PR9 plan, D1). None of it derives state: the daemon says what a
//! session is, and these say it in the words `PRODUCT.md` allows.

mod activation;
mod connection;
mod endpoint;
mod error;
pub mod launch;
mod policy;
pub mod presentation;
pub mod sessions;
mod spawn;

pub use activation::{activate, activate_at};
pub use connection::{Connection, TerminalChannel};
pub use endpoint::{ENDPOINT_OVERRIDE, EndpointSelection};
pub use error::{
    ActivationContext, ActivationError, HandshakeFault, OwnerEvidence, RequestError, SpawnOutcome,
};
pub use policy::ClientActivationPolicy;
