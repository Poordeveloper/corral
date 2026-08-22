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
    pub compatibility: Compatibility,
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
mod tests {
    use super::*;

    fn peer(protocol_version: u32, min_compatible_peer_version: u32) -> PeerVersions {
        PeerVersions {
            protocol_version,
            min_compatible_peer_version,
        }
    }

    #[test]
    fn both_peers_always_reach_the_same_verdict() {
        for a_version in 1..8 {
            for a_min in 1..=a_version {
                for b_version in 1..8 {
                    for b_min in 1..=b_version {
                        let a = peer(a_version, a_min);
                        let b = peer(b_version, b_min);
                        assert_eq!(
                            compatible(a, b),
                            compatible(b, a),
                            "asymmetric verdict for {a:?} and {b:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn a_peer_below_the_local_floor_is_incompatible() {
        assert!(!compatible(peer(3, 2), peer(1, 1)));
    }

    #[test]
    fn a_peer_whose_floor_is_above_us_is_incompatible() {
        assert!(!compatible(peer(1, 1), peer(3, 2)));
    }

    #[test]
    fn overlapping_ranges_are_compatible() {
        assert!(compatible(peer(3, 1), peer(1, 1)));
    }

    #[test]
    fn a_hello_without_capabilities_means_the_empty_set() {
        let hello: ClientHello =
            serde_json::from_str(r#"{"protocol_version":1,"min_compatible_peer_version":1}"#)
                .expect("decode");

        assert!(hello.capabilities.is_empty());
    }

    #[test]
    fn a_hello_missing_a_required_version_does_not_decode() {
        let error = serde_json::from_str::<ClientHello>(r#"{"protocol_version":1}"#)
            .expect_err("required field");

        assert!(error.to_string().contains("min_compatible_peer_version"));
    }

    #[test]
    fn unknown_hello_fields_are_ignored() {
        let hello: ClientHello = serde_json::from_str(
            r#"{"protocol_version":1,"min_compatible_peer_version":1,"nickname":"future"}"#,
        )
        .expect("decode");

        assert_eq!(hello.protocol_version, 1);
    }
}
