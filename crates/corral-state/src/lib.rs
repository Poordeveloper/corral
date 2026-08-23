#![forbid(unsafe_code)]

//! The registry store: Corral-owned durable facts and the projections that
//! summarize them.
//!
//! Two things live here and nowhere else. The durable semantic event log — the
//! Corral-owned facts Corral must order, replay, and keep consistent — and the
//! encoding those facts are written in. The domain crate owns the meaning of a
//! fact; this crate owns what it looks like on disk, so a domain type can
//! change shape without silently changing what a stored fact means.
//!
//! What is not here is as fixed as what is. Provider history stays
//! provider-owned. Live runtime state stays runtime-owned and is never
//! persisted as fact. Derived status is computed, never stored
//! (AGENTS.md §Durable state).

mod encoding;
mod error;
mod event;
mod projection;
mod schema;
mod store;

pub use error::{FatalState, Refusal, StateError};
pub use event::SessionEvent;
pub use store::{
    BindingResolution, CommandAcceptance, Durability, RecordedEvent, RecordedRun,
    SessionResolution, Store,
};
