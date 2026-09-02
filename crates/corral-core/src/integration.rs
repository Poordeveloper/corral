//! What the user chose about provider integration, and how much self-repair
//! that choice authorizes.
//!
//! Intent is a Corral-owned fact with no external source of truth: the
//! provider's own configuration file cannot carry it, because a Corral-owned
//! entry missing from that file is indistinguishable from a provider rewrite
//! that dropped it and from the user deleting it by hand (ADR 0013 D5/D6). The
//! file is evidence of what is installed; it is never the record of what the
//! user chose.

use std::time::{Duration, SystemTime};

use crate::external_name::ProviderId;

/// Whether the user wants Corral integrated with a provider on this node.
///
/// Two states and no third: "installed" is not an intent, it is an observation
/// about a file, and a `Broken`/`Degraded` intent would let a failed write
/// silently revoke a choice the user never revisited.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum IntegrationIntent {
    /// Corral may maintain what Corral owns in this provider's configuration.
    Enabled,
    /// The user turned integration off. Token-less deliveries for this
    /// provider are dropped, and nothing is written or repaired.
    Disabled,
}

/// The provider configuration a Corral-owned entry lives in.
///
/// Named files rather than a provider-derived path, because the trigger list,
/// the parser, and the repair budget are all per file: a provider that grows a
/// second integration surface must add a variant and answer those questions
/// again rather than inherit the first file's answers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ConfigTarget {
    /// Claude Code's user-scope `settings.json`.
    ClaudeUserSettings,
    /// Codex's user-scope `config.toml`.
    CodexUserConfig,
}

/// Drift that automatic repair may correct.
///
/// Ownership conflict — Corral's slot holding content Corral cannot prove is
/// its own — is deliberately absent: it is never auto-repaired, so it can
/// never consume repair budget or open a breaker (grill Q4′). Making that a
/// variant here would let a caller record a repair that policy forbids.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RepairableDrift {
    /// The Corral-owned entry is gone. Expected after a provider rewrites its
    /// own file, so this is the ordinary path, not evidence of a competing
    /// authority.
    Missing,
    /// The entry is present in a representation an older Corral wrote.
    OldRepresentation,
}

/// What a repeated repair is counted against.
///
/// Two drifts of different classes in one file are different recurrences: a
/// provider dropping the entry and a stale representation surviving an upgrade
/// deserve different answers, and one budget shared between them would let the
/// benign case exhaust the authority the suspicious case needs.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RepairFingerprint {
    provider: ProviderId,
    target: ConfigTarget,
    drift: RepairableDrift,
}

impl RepairFingerprint {
    #[must_use]
    pub fn new(provider: ProviderId, target: ConfigTarget, drift: RepairableDrift) -> Self {
        Self {
            provider,
            target,
            drift,
        }
    }

    #[must_use]
    pub fn provider(&self) -> &ProviderId {
        &self.provider
    }

    #[must_use]
    pub fn target(&self) -> ConfigTarget {
        self.target
    }

    #[must_use]
    pub fn drift(&self) -> RepairableDrift {
        self.drift
    }
}

/// How many automatic repairs of one fingerprint are allowed, and over what
/// span.
///
/// A policy default the caller supplies, never a constant read here: the
/// numbers are dogfood-tunable and belong with the rest of the daemon's
/// policy, while what a budget *means* belongs to the domain (grill Q4′).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RepairBudget {
    repairs: u32,
    window: Duration,
}

impl RepairBudget {
    #[must_use]
    pub const fn new(repairs: u32, window: Duration) -> Self {
        Self { repairs, window }
    }

    #[must_use]
    pub fn repairs(self) -> u32 {
        self.repairs
    }

    #[must_use]
    pub fn window(self) -> Duration {
        self.window
    }

    /// The earliest repair still inside the window ending at `now`.
    ///
    /// A bound that cannot be represented falls back to the epoch, which
    /// counts every recorded repair rather than none: the failure mode of a
    /// clock this broken should be withdrawing authority, never a rewrite
    /// loop.
    #[must_use]
    pub fn window_starts(self, now: SystemTime) -> SystemTime {
        now.checked_sub(self.window)
            .unwrap_or(SystemTime::UNIX_EPOCH)
    }
}

/// Whether Corral may still repair this fingerprint without asking.
///
/// The withdrawn state is sticky by construction — it carries no expiry and no
/// remaining count, because the rolling window decides only when authority is
/// withdrawn, never when it returns. Only an explicit user reconciliation
/// clears it; a daemon restart or a quiet day must not (grill Q4′).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RepairAuthority {
    /// Automatic repair is authorized. `remaining` is how many more repairs
    /// the current window admits before authority is withdrawn.
    Available { remaining: u32 },
    /// The breaker is open: another authority keeps undoing Corral's
    /// integration, and Corral stops rather than joining a tug-of-war.
    Withdrawn { since: SystemTime },
}

impl RepairAuthority {
    /// Whether an automatic repair may proceed now.
    #[must_use]
    pub fn permits_repair(self) -> bool {
        match self {
            Self::Available { remaining } => remaining > 0,
            Self::Withdrawn { .. } => false,
        }
    }
}

#[cfg(test)]
#[path = "integration_tests.rs"]
mod tests;
