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

mod activation;
mod connection;
mod endpoint;
mod error;
mod policy;
mod spawn;

pub use activation::activate;
pub use connection::Connection;
pub use endpoint::{ENDPOINT_OVERRIDE, EndpointSelection};
pub use error::{
    ActivationContext, ActivationError, HandshakeFault, OwnerEvidence, RequestError, SpawnOutcome,
};
pub use policy::ClientActivationPolicy;
