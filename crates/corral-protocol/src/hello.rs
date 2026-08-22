use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

/// The identity half of a hello: what a peer speaks and what it can work with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PeerVersions {
    pub protocol_version: u32,
    pub min_compatible_peer_version: u32,
}

/// The first message on every connection; nothing else may precede it.
///
/// `protocol_version` and `min_compatible_peer_version` are required identity
/// fields. Missing or ill-typed makes the hello malformed rather than old:
/// an old peer states a version this build can compare, a malformed one states
/// nothing at all, and guessing a version for it would be inventing a fact.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ClientHello {
    pub protocol_version: u32,
    pub min_compatible_peer_version: u32,
    /// Optional feature contracts the client can honour. Absent means the
    /// empty set — feature eligibility, never protocol incompatibility.
    #[serde(default)]
    pub capabilities: BTreeSet<String>,
}

/// The daemon's half, including the verdict it reached independently.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ServerHello {
    pub protocol_version: u32,
    pub min_compatible_peer_version: u32,
    #[serde(default)]
    pub capabilities: BTreeSet<String>,
    /// Required: absence would be unknown, and neither verdict may be assumed
    /// from unknown.
    pub compatibility_result: Compatibility,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Compatibility {
    Compatible,
    Incompatible,
}

/// The one compatibility predicate, evaluated independently by both peers.
///
/// It is symmetric by construction: swapping the arguments cannot change the
/// verdict, so a disagreement between the two sides is an internal bug and the
/// connection fails rather than continue ambiguously compatible.
pub fn compatible(local: PeerVersions, remote: PeerVersions) -> bool {
    remote.protocol_version >= local.min_compatible_peer_version
        && local.protocol_version >= remote.min_compatible_peer_version
}

impl ClientHello {
    pub fn versions(&self) -> PeerVersions {
        PeerVersions {
            protocol_version: self.protocol_version,
            min_compatible_peer_version: self.min_compatible_peer_version,
        }
    }
}

impl ServerHello {
    pub fn versions(&self) -> PeerVersions {
        PeerVersions {
            protocol_version: self.protocol_version,
            min_compatible_peer_version: self.min_compatible_peer_version,
        }
    }
}

#[cfg(test)]
#[path = "hello_tests.rs"]
mod tests;
