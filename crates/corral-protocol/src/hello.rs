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
    /// The role this connection claims.
    ///
    /// Absent is the semantic RPC role every connection has always had, so an
    /// older client's hello means exactly what it always meant. Present with
    /// the terminal-data role plus a token turns this connection into a
    /// terminal data channel — a one-way transition: it never carries RPC
    /// again (ADR 0003, grill Q2).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<ConnectionRole>,
}

/// Feature contracts a peer may advertise.
///
/// The mechanism the `capabilities` field exists for: a name means "this build
/// serves that contract", never a protocol version. A client asks before it
/// offers a person an action the daemon may not serve, so an older daemon is
/// reported as older rather than as having refused the request.
pub mod capability {
    /// `session.resume`, and `session.new` naming a provider rather than a
    /// command: the managed-agent surface PR5 added.
    ///
    /// One name for both because they are one contract — a daemon that
    /// composes a managed launch is the same daemon that can continue one, and
    /// two names would let a client believe in half of it.
    pub const MANAGED_SESSIONS: &str = "managed-sessions";

    /// `session.continuation`, the working directory and disclosure revision
    /// `session.resume` accepts, and history rows in `session.list`: the
    /// history-enumerated session surface of ADR 0016.
    ///
    /// One name for the whole surface, on [`MANAGED_SESSIONS`]'s reasoning — a
    /// client that may be shown a history row is the client that must ask a
    /// preflight before continuing one.
    ///
    /// `MANAGED_SESSIONS` cannot answer for it. That name was minted for the
    /// managed-agent surface and a daemon serving it may predate this one
    /// entirely, so a client that read it as permission to ask for a preflight
    /// would meet `method_not_found` in front of a person, for a continuation
    /// that worked before they upgraded.
    pub const HISTORY_SESSIONS: &str = "history-sessions.v1";

    /// The attention projection on `session.list`, `attention.summary`, and
    /// `attention.acknowledge`: the daemon derives the five-state main
    /// status and clients render it (ADR 0015).
    pub const ATTENTION: &str = "attention.v1";
}

/// What a connection is for.
///
/// An unknown kind decodes into `Unknown` rather than failing, so a client
/// that claims a role a newer build serves is answered — refused for the role
/// it asked for, not rejected as though its hello were malformed. A malformed
/// hello and an unsupported role are different facts and get different
/// answers (AGENTS.md §Protocol).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConnectionRole {
    /// A terminal data channel, redeeming a token from `terminal.attach`.
    TerminalData { attach_token: String },
    /// A role this build serves, claimed without what the claim requires.
    ///
    /// A separate answer from `Unknown`: telling a client its daemon lacks a
    /// feature, when the fault is a field the client left out, sends it
    /// looking for a problem it does not have.
    Malformed { kind: String },
    /// A role this build does not serve. The kind is kept because it is the
    /// only thing a diagnostic can report.
    Unknown { kind: String },
}

/// The wire form: a tagged object whose unknown tags survive decoding.
#[derive(Serialize, Deserialize)]
struct RoleOnTheWire {
    kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    attach_token: Option<String>,
}

impl ConnectionRole {
    /// The wire tag for the terminal-data role.
    ///
    /// One constant, used by both the encoder and the decoder, so a published
    /// spelling cannot drift from what serde actually emits.
    pub const TERMINAL_DATA: &'static str = "terminal_data";
}

impl Serialize for ConnectionRole {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let wire = match self {
            Self::TerminalData { attach_token } => RoleOnTheWire {
                kind: Self::TERMINAL_DATA.to_owned(),
                attach_token: Some(attach_token.clone()),
            },
            Self::Malformed { kind } | Self::Unknown { kind } => RoleOnTheWire {
                kind: kind.clone(),
                attach_token: None,
            },
        };
        wire.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for ConnectionRole {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let wire = RoleOnTheWire::deserialize(deserializer)?;
        if wire.kind == Self::TERMINAL_DATA {
            // A terminal-data role without a token is not that role: the token
            // is the whole claim. But it is not an *unknown* role either — the
            // kind is one this build serves, and telling a client otherwise
            // would send it looking for a missing feature instead of its own
            // missing field.
            return Ok(match wire.attach_token {
                Some(attach_token) => Self::TerminalData { attach_token },
                None => Self::Malformed { kind: wire.kind },
            });
        }
        Ok(Self::Unknown { kind: wire.kind })
    }
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
