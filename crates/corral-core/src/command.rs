use std::collections::BTreeMap;
use std::fmt;
use std::time::SystemTime;

use crate::external_name::hides_or_reorders;
use crate::id::{CorralSessionId, RunId};

/// A client-supplied id for one mutating command.
///
/// Unique within the node's durable command namespace: across Sessions, Runs,
/// clients, connections, and `corrald` restarts. Not per-Session — the first
/// mutation a client makes may have no target Session yet, and a namespace
/// that reset on restart would let a new daemon re-execute a command the old
/// one already performed (ADR 0002, Q13).
///
/// A UUID is the recommended form, but correctness does not rest on UUIDs
/// never colliding: a genuine collision resolves through the fingerprint —
/// same semantic command, same receipt; different one, a conflict.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct CommandId(String);

impl CommandId {
    pub const LIMIT: usize = 128;

    pub fn new(raw: impl Into<String>) -> Result<Self, MalformedCommandId> {
        bounded_token(raw.into(), Self::LIMIT).map(Self)
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// What a command does, as the producer of that command names it.
///
/// PR2 owns the identity and idempotency mechanism, not the catalogue: the
/// phase that serves the first mutating RPC names its own commands.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct CommandKind(String);

impl CommandKind {
    pub const LIMIT: usize = 64;

    pub fn new(raw: impl Into<String>) -> Result<Self, MalformedCommandId> {
        bounded_token(raw.into(), Self::LIMIT).map(Self)
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The semantic identity of a mutating command.
///
/// Covers the command kind and every input that affects the mutation. It
/// excludes serialization formatting, transport metadata, tracing metadata,
/// and retry timestamps — idempotency binds to what the command *means*, not
/// to one encoding's incidental bytes, so a serializer change or a reordered
/// object cannot split one command into two (ADR 0002, Q12).
///
/// The canonical rendering below is stored as-is rather than hashed: it costs
/// little at this scale and it makes a conflict readable instead of a pair of
/// digests that differ for no visible reason.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct CommandFingerprint(String);

impl CommandFingerprint {
    #[must_use]
    pub fn builder(kind: CommandKind) -> CommandFingerprintBuilder {
        CommandFingerprintBuilder {
            kind,
            inputs: BTreeMap::new(),
        }
    }

    /// Read a fingerprint back out of durable state.
    #[must_use]
    pub fn from_canonical(canonical: impl Into<String>) -> Self {
        Self(canonical.into())
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Builds a fingerprint from the command's semantic inputs.
///
/// Inputs are named, so the order a producer happens to add them in cannot
/// change the result; the same command described twice fingerprints the same.
/// A name given twice is one input, last value winning: this is a description
/// of one command, not a multimap.
pub struct CommandFingerprintBuilder {
    kind: CommandKind,
    inputs: BTreeMap<String, String>,
}

impl CommandFingerprintBuilder {
    #[must_use]
    pub fn input(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.inputs.insert(name.into(), value.into());
        self
    }

    #[must_use]
    pub fn build(self) -> CommandFingerprint {
        // Length-prefixed parts, so no input value can be written to look like
        // a part boundary and impersonate a different command.
        let mut canonical = String::new();
        canonical.push('k');
        push_part(&mut canonical, self.kind.as_str());
        for (name, value) in &self.inputs {
            canonical.push('i');
            push_part(&mut canonical, name);
            push_part(&mut canonical, value);
        }
        CommandFingerprint(canonical)
    }
}

fn push_part(canonical: &mut String, part: &str) {
    canonical.push_str(&part.len().to_string());
    canonical.push(':');
    canonical.push_str(part);
}

/// One mutating command as the store sees it: who it is, and what it means.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Command {
    id: CommandId,
    fingerprint: CommandFingerprint,
}

impl Command {
    #[must_use]
    pub fn new(id: CommandId, fingerprint: CommandFingerprint) -> Self {
        Self { id, fingerprint }
    }

    #[must_use]
    pub fn id(&self) -> &CommandId {
        &self.id
    }

    #[must_use]
    pub fn fingerprint(&self) -> &CommandFingerprint {
        &self.fingerprint
    }
}

/// What a command did, recorded so that a retry can be answered without doing
/// it again.
///
/// A variant per kind of thing a command produces, because a retry has to be
/// answered with what the first attempt made. A continuation makes no Session
/// — it makes another Run of one that already existed — and answering it with
/// the Session's first Run would hand a retry the wrong episode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommandOutcome {
    SessionCreated(CorralSessionId),
    /// A new Run of a Session that already existed.
    RunStarted {
        session: CorralSessionId,
        run: RunId,
    },
}

impl CommandOutcome {
    /// The Session this command acted on or created.
    #[must_use]
    pub fn session(self) -> CorralSessionId {
        match self {
            Self::SessionCreated(session) | Self::RunStarted { session, .. } => session,
        }
    }
}

/// The durable record that a command was accepted and what it produced.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandReceipt {
    command: CommandId,
    fingerprint: CommandFingerprint,
    outcome: CommandOutcome,
    accepted_at: SystemTime,
}

impl CommandReceipt {
    #[must_use]
    pub fn new(
        command: CommandId,
        fingerprint: CommandFingerprint,
        outcome: CommandOutcome,
        accepted_at: SystemTime,
    ) -> Self {
        Self {
            command,
            fingerprint,
            outcome,
            accepted_at,
        }
    }

    #[must_use]
    pub fn command(&self) -> &CommandId {
        &self.command
    }

    #[must_use]
    pub fn fingerprint(&self) -> &CommandFingerprint {
        &self.fingerprint
    }

    #[must_use]
    pub fn outcome(&self) -> CommandOutcome {
        self.outcome
    }

    #[must_use]
    pub fn accepted_at(&self) -> SystemTime {
        self.accepted_at
    }
}

/// Neither a command id nor a command kind may carry whitespace or control
/// characters: both are compared for exact equality and appear in logs and
/// error text, where an id that can hide inside another is a way to make one
/// command look like a different one.
fn bounded_token(raw: String, limit: usize) -> Result<String, MalformedCommandId> {
    if raw.is_empty() {
        return Err(MalformedCommandId::Empty);
    }
    if raw.len() > limit {
        return Err(MalformedCommandId::TooLong {
            length: raw.len(),
            limit,
        });
    }
    if raw
        .chars()
        .any(|c| c.is_whitespace() || hides_or_reorders(c))
    {
        return Err(MalformedCommandId::UnusableCharacter);
    }
    Ok(raw)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MalformedCommandId {
    Empty,
    TooLong { length: usize, limit: usize },
    UnusableCharacter,
}

impl fmt::Display for MalformedCommandId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => f.write_str("it is empty"),
            Self::TooLong { length, limit } => {
                write!(f, "it is {length} bytes, and the limit is {limit}")
            }
            Self::UnusableCharacter => f.write_str("it contains whitespace or a control character"),
        }
    }
}

impl std::error::Error for MalformedCommandId {}

#[cfg(test)]
#[path = "command_tests.rs"]
mod tests;
