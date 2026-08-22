use std::path::PathBuf;

use corral_rendezvous::{RendezvousPaths, validate_endpoint_path};

use crate::error::ActivationError;

/// Redirects this client to an endpoint someone else manages.
///
/// It is not an instance namespace: it never becomes a second place where a
/// primary daemon may be started, so test and development instances stay an
/// explicit future feature rather than something a stray variable creates.
pub const ENDPOINT_OVERRIDE: &str = "CORRAL_ENDPOINT";

/// Which endpoint this activation is about, and whether it may start a daemon.
#[derive(Clone, Debug)]
pub enum EndpointSelection {
    /// The account's canonical rendezvous. Auto-activation happens only here.
    Canonical(RendezvousPaths),
    /// An externally managed endpoint. Connect only.
    Explicit(PathBuf),
}

impl EndpointSelection {
    pub fn from_environment() -> Result<Self, ActivationError> {
        match std::env::var_os(ENDPOINT_OVERRIDE) {
            Some(raw) => Ok(Self::Explicit(validate_endpoint_path(&raw)?)),
            None => Ok(Self::Canonical(RendezvousPaths::canonical()?)),
        }
    }

    pub fn endpoint(&self) -> &std::path::Path {
        match self {
            Self::Canonical(paths) => paths.socket(),
            Self::Explicit(path) => path,
        }
    }
}
