//! Attach tokens: the one-time capability that binds a terminal data channel
//! to a terminal.
//!
//! A token is a local bearer capability resting on the endpoint's same-user
//! filesystem boundary — not an authentication scheme, and not a seam claimed
//! on behalf of some future remote design.
//!
//! It binds to a Session **and** to the concrete Run, never to a Session
//! alone: a Session outlives its Runs, so a token issued before a resume must
//! never open the terminal of the process that replaced it.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use corral_core::{CorralSessionId, RunId};

/// How long an issued token stays redeemable.
///
/// A client redeems over a local socket in milliseconds; thirty seconds is
/// slack for a paused debugger, not a window anyone waits in.
pub const ATTACH_TOKEN_TTL: Duration = Duration::from_secs(30);

/// A one-time capability to open a terminal data channel.
///
/// 128 bits from the OS CSPRNG. Compared in constant time is unnecessary here —
/// the value is an index into a map of live grants, not a secret compared
/// against a stored one — but it is never logged, because a log is a place a
/// capability outlives its thirty seconds.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct AttachToken([u8; 16]);

/// What a token opens.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AttachGrant {
    pub session: CorralSessionId,
    pub run: RunId,
}

/// The OS could not supply randomness, so no token was minted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NoRandomness;

/// Why a token did not open a channel.
///
/// One variant on purpose. Distinguishing "expired" from "never existed" would
/// tell a caller which of its guesses was closer, and neither answer changes
/// what it must do: ask for another token.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AttachRefused;

struct Issued {
    grant: AttachGrant,
    expires_at: Instant,
}

/// The live grants a daemon has issued and not yet seen redeemed.
#[derive(Default)]
pub struct AttachTokens {
    issued: HashMap<AttachToken, Issued>,
}

impl AttachTokens {
    pub fn new() -> Self {
        Self::default()
    }

    /// Issue a token, or refuse when the OS cannot supply randomness.
    ///
    /// A daemon that cannot mint an unguessable token does not mint a
    /// guessable one, and does not die either: the attach fails and everything
    /// already running keeps running.
    pub fn issue(&mut self, grant: AttachGrant) -> Result<AttachToken, NoRandomness> {
        let now = Instant::now();
        // Swept here rather than on a timer: issuing is the only moment the
        // map grows, so it is the only moment it can grow without bound. A
        // client that asks for a token and never opens a channel — a crashed
        // CLI, a retry loop — would otherwise leave an entry for the daemon's
        // whole life.
        self.discard_expired_at(now);
        self.issue_at(grant, now)
    }

    /// Redeem a token, or refuse it.
    ///
    /// Redemption is one step: validating and consuming cannot be separated,
    /// or two clients could both validate the same token before either
    /// consumed it. Consumption is final — if the caller then fails to send a
    /// snapshot, the token stays spent and the client asks for another. A
    /// half-consumed token would be a branch where a capability comes back.
    pub fn redeem(&mut self, token: &AttachToken) -> Result<AttachGrant, AttachRefused> {
        self.redeem_at(token, Instant::now())
    }

    pub fn outstanding(&self) -> usize {
        self.issued.len()
    }

    fn issue_at(&mut self, grant: AttachGrant, now: Instant) -> Result<AttachToken, NoRandomness> {
        let mut bytes = [0_u8; 16];
        getrandom::fill(&mut bytes).map_err(|_| NoRandomness)?;
        let token = AttachToken(bytes);
        self.issued.insert(
            token,
            Issued {
                grant,
                expires_at: now + ATTACH_TOKEN_TTL,
            },
        );
        Ok(token)
    }

    fn redeem_at(
        &mut self,
        token: &AttachToken,
        now: Instant,
    ) -> Result<AttachGrant, AttachRefused> {
        let issued = self.issued.remove(token).ok_or(AttachRefused)?;
        if now >= issued.expires_at {
            return Err(AttachRefused);
        }
        Ok(issued.grant)
    }

    fn discard_expired_at(&mut self, now: Instant) {
        self.issued.retain(|_, issued| now < issued.expires_at);
    }
}

impl AttachToken {
    /// The wire form: lowercase hex, so the value survives a JSON string
    /// without an encoding question.
    pub fn to_wire(self) -> String {
        self.0.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    pub fn from_wire(raw: &str) -> Option<Self> {
        if raw.len() != 32 {
            return None;
        }
        let mut bytes = [0_u8; 16];
        for (index, byte) in bytes.iter_mut().enumerate() {
            *byte = u8::from_str_radix(raw.get(index * 2..index * 2 + 2)?, 16).ok()?;
        }
        Some(Self(bytes))
    }
}

/// Never prints the value: a capability in a log outlives its thirty seconds.
impl std::fmt::Debug for AttachToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AttachToken(redacted)")
    }
}

impl std::fmt::Display for AttachRefused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("the attach token is not redeemable")
    }
}

impl std::error::Error for AttachRefused {}

impl std::fmt::Display for NoRandomness {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("the operating system could not supply randomness")
    }
}

impl std::error::Error for NoRandomness {}

#[cfg(test)]
#[path = "attach_tests.rs"]
mod tests;
