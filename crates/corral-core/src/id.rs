use std::fmt;
use std::str::FromStr;

use uuid::Uuid;

/// A Corral-minted identity that carries no meaning.
///
/// Every identity below is opaque on purpose: no provider id, pid, path,
/// terminal, or timestamp is recoverable from it, so nothing downstream can
/// reconstruct a binding from an identity or order two of them by age. That is
/// why the minted form is a random UUID rather than a time-ordered one
/// (ADR 0002 D1) — ordering is the event log's job, not identity's.
macro_rules! corral_id {
    ($(#[$doc:meta])* $name:ident, $label:literal) => {
        $(#[$doc])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub struct $name(Uuid);

        impl $name {
            /// Mint a new identity. Nothing outside Corral can produce one.
            #[must_use]
            pub fn mint() -> Self {
                Self(Uuid::new_v4())
            }

            /// Rebuild an identity Corral already minted, when reading it back
            /// out of durable state.
            #[must_use]
            pub fn from_uuid(uuid: Uuid) -> Self {
                Self(uuid)
            }

            #[must_use]
            pub fn as_uuid(&self) -> Uuid {
                self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}", self.0.as_hyphenated())
            }
        }

        impl FromStr for $name {
            type Err = MalformedId;

            fn from_str(raw: &str) -> Result<Self, Self::Err> {
                Uuid::parse_str(raw)
                    .map(Self)
                    .map_err(|_| MalformedId {
                        expected: $label,
                        raw: raw.to_owned(),
                    })
            }
        }
    };
}

corral_id!(
    /// The primary key of a Session. Never a provider session id, pane id,
    /// terminal id, cwd, or `(node, provider_session_id)` (`ARCHITECTURE.md`
    /// §1).
    CorralSessionId,
    "a Corral session id"
);

corral_id!(
    /// One concrete runtime occurrence of a Session. A Run's ordinal is for
    /// display; this is what durable facts and bindings name (ADR 0002 D1).
    RunId,
    "a run id"
);

corral_id!(
    /// One edge from a Session to an external identity.
    BindingId,
    "a binding id"
);

corral_id!(
    /// A machine running `corrald`. Scopes external bindings; never part of
    /// Session identity.
    NodeId,
    "a node id"
);

corral_id!(
    /// One specific blocked interaction awaiting an answer.
    NeedsInputRequestId,
    "a needs-input request id"
);

corral_id!(
    /// One attention item, for the life of the daemon that minted it.
    ///
    /// Ephemeral by decision (grill Q19): never persisted, never rebuilt across
    /// a restart, and never guessed to be the same as an earlier one. It exists
    /// so an acknowledgement names the item it saw rather than whatever item is
    /// current when the acknowledgement arrives.
    AttentionItemId,
    "an attention item id"
);

/// Text that was expected to be a Corral-minted identity and is not.
///
/// Reaching this from durable state means the store holds something Corral
/// did not write, which is why the caller may not repair it into a fresh
/// identity: a repaired identity silently renames a recorded fact.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MalformedId {
    pub expected: &'static str,
    pub raw: String,
}

impl fmt::Display for MalformedId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?} is not {}", self.raw, self.expected)
    }
}

impl std::error::Error for MalformedId {}

#[cfg(test)]
#[path = "id_tests.rs"]
mod tests;
