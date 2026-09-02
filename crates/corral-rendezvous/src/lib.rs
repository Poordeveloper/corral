#![forbid(unsafe_code)]

//! Canonical rendezvous identity: where the one primary `corrald` of an OS
//! account lives, and the artifact rules that keep it single.
//!
//! `corral-client` and `corrald` must agree byte-for-byte on the canonical
//! endpoint, the singleton lock, and who may remove what — a disagreement
//! would partition one account into two primary daemons. That shared
//! agreement is why this crate exists rather than a copy on each side
//! (ADR 0001, crate ownership).
//!
//! The whole crate is the Unix boundary for that agreement: flock semantics,
//! `sun_path` limits, and account-database lookup live here and nothing
//! Unix-shaped leaks upward into the domain or the wire.

#[cfg(not(unix))]
compile_error!(
    "corral-rendezvous implements the Unix rendezvous; native Windows activation is a separate decision (ADR 0005)"
);

mod error;
mod lock;
mod paths;
mod socket;
#[cfg(test)]
mod test_scratch;

pub use error::{FileKind, InvalidEndpointReason, RendezvousError};
pub use lock::{OwnerProbe, SingletonClaim, probe_owner};
pub use paths::{RendezvousPaths, provider_home, validate_endpoint_path};
pub use socket::{SocketPathState, inspect_socket_path, remove_stale_socket};
