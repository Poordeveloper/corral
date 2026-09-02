//! The one mutator of a user's own provider configuration.
//!
//! Corral writes into files it does not own — `~/.claude/settings.json`,
//! `~/.codex/config.toml` — and this module is the only code that may
//! (ADR 0013 D1). Everything about that authority is bounded here: what
//! Corral owns inside those files (D2, delegated to the provider adapters),
//! how a merge preserves the rest (D3), the closed list of conditions that
//! refuse a write instead of guessing (D4), and how drift is repaired without
//! becoming a configuration tug-of-war (D5).
//!
//! Nothing here interprets provider semantics and nothing in the provider
//! adapters touches the filesystem. The adapters answer "what does a
//! Corral-owned entry look like in this document"; this module answers "may
//! that document be replaced, and how".

mod file;
mod trigger;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::SystemTime;

use corral_core::{
    ConfigTarget, IntegrationIntent, ProviderId, RepairAuthority, RepairFingerprint,
    RepairableDrift,
};
use corral_state::StateError;

use crate::provider::KnownProvider;
use crate::provider::launch::RelayInvocation;
use crate::state::DaemonState;

pub use trigger::Trigger;

/// One mutation of a provider's configuration at a time.
///
/// An operation is a sequence — record what the user chose, then bring the
/// file to it — and two sequences interleaved can leave the record saying one
/// thing and the file another, each reporting success. The write's own
/// identity check does not close that: two Corral writers read the same file,
/// both pass the check, and the second rename discards the first. So the
/// sequence is what is serialized, per provider, for as long as any operation
/// that may write is in flight — an explicit enable or disable, and the repair
/// a daemon start runs beside whatever its connections are doing.
#[derive(Default)]
pub struct WriteTurns {
    claude: tokio::sync::Mutex<()>,
    codex: tokio::sync::Mutex<()>,
}

impl WriteTurns {
    async fn take(&self, provider: KnownProvider) -> tokio::sync::MutexGuard<'_, ()> {
        match provider {
            KnownProvider::Claude => self.claude.lock().await,
            KnownProvider::Codex => self.codex.lock().await,
        }
    }
}

/// What one provider's integration looks like right now.
///
/// The answer `status` gives and the answer `install` and `repair` return:
/// one shape, so a surface renders the same thing whatever produced it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Standing {
    /// Corral's entries are present, current, and this binary wrote them.
    Installed,
    /// The file carries no Corral-owned entry.
    NotInstalled,
    /// Corral owns entries here, but not the ones this binary would write.
    /// Repairable in place.
    Drifted(RepairableDrift),
    /// A condition on D4's closed list. No write happened, nothing was
    /// overwritten, and the cause is named for the user to resolve.
    Refused(Trigger),
    /// Repair was authorized once and is not any more: something keeps
    /// undoing Corral's integration, and Corral stopped rather than joining a
    /// configuration tug-of-war (grill Q4′). Only an explicit user action
    /// re-arms it.
    RepairWithheld { since: SystemTime },
}

impl Standing {
    /// Whether Corral can currently expect deliveries from this provider.
    ///
    /// A refusal or drift means the integration cannot claim delivery, which
    /// is what puts a provider into Limited awareness (`PRODUCT.md` §6).
    #[must_use]
    pub fn claims_delivery(&self) -> bool {
        matches!(self, Self::Installed)
    }
}

/// The provider file one operation acts on.
///
/// Bundles the three things every operation needs — which provider, which
/// file, and what a Corral-owned entry in it looks like — so no caller can
/// pair a provider with another provider's path.
#[derive(Clone)]
pub struct Target {
    provider: KnownProvider,
    target: ConfigTarget,
    path: PathBuf,
}

impl Target {
    /// Where this provider's user-scope configuration lives.
    ///
    /// Resolved through the account database, never `$HOME` (ADR 0001 D1):
    /// the one mutator must not be pointable at another account's files by a
    /// shell variable.
    pub fn resolve(provider: KnownProvider) -> Result<Self, corral_rendezvous::RendezvousError> {
        let home = corral_rendezvous::provider_home()?;
        let (target, path) = match provider {
            KnownProvider::Claude => (
                ConfigTarget::ClaudeUserSettings,
                home.join(".claude").join("settings.json"),
            ),
            KnownProvider::Codex => (
                ConfigTarget::CodexUserConfig,
                home.join(".codex").join("config.toml"),
            ),
        };
        Ok(Self {
            provider,
            target,
            path,
        })
    }

    #[must_use]
    pub fn provider(&self) -> KnownProvider {
        self.provider
    }

    #[must_use]
    pub fn config_target(&self) -> ConfigTarget {
        self.target
    }

    #[must_use]
    pub fn path(&self) -> &std::path::Path {
        &self.path
    }
}

/// What the file holds, and what this binary would put there.
///
/// Read once per operation. Every decision below is taken against this
/// snapshot rather than against a second read, so a file that changes under
/// Corral is caught by the write's own identity re-check (D3) instead of
/// producing two different answers within one operation.
struct Examined {
    /// The file's bytes, and `None` when there is no file yet.
    original: Option<file::Read>,
    standing: Standing,
}

/// What one provider's integration looks like, without changing anything.
///
/// Reads a file, so it is the caller's job to keep it off the reactor —
/// `corrald` runs one runtime thread, and a synchronous read on it stalls
/// every connection the daemon is serving. `off_the_reactor` is how every
/// caller here does that.
pub fn status(target: &Target, relay: &RelayInvocation) -> Standing {
    examine(target, relay).standing
}

/// Run one integration operation on the blocking pool.
///
/// Every operation in this module reads a file and most of them write one
/// with an `fsync`. None of that may happen on the reactor thread (the same
/// rule `managed_launch` follows for injection), and routing them all through
/// one helper is what keeps a later operation from quietly skipping it.
pub async fn off_the_reactor<T: Send + 'static>(
    work: impl FnOnce() -> T + Send + 'static,
) -> Option<T> {
    match tokio::task::spawn_blocking(work).await {
        Ok(outcome) => Some(outcome),
        // The blocking pool is gone or the work panicked. Neither is an
        // answer about the user's configuration, and inventing one would be
        // worse than saying nothing.
        Err(error) => {
            tracing::warn!(%error, "an integration operation did not complete");
            None
        }
    }
}

/// Install Corral's entries, or report why not.
///
/// The write path is the same one `repair` uses: there is one merge engine,
/// one trigger list, and one backup rule, because a second path is how the
/// safe one comes to be skipped.
pub fn install(
    target: &Target,
    relay: &RelayInvocation,
    now: SystemTime,
    state_dir: &std::path::Path,
) -> Standing {
    let examined = examine(target, relay);
    match examined.standing {
        // A refusal names a condition in the file; withholding names a
        // decision about repeated repair, which `examine` never reaches.
        refused @ (Standing::Refused(_) | Standing::RepairWithheld { .. }) => refused,
        Standing::Installed => Standing::Installed,
        Standing::NotInstalled | Standing::Drifted(_) => {
            match write_entries(target, relay, examined.original, now, state_dir) {
                Ok(()) => Standing::Installed,
                Err(trigger) => Standing::Refused(trigger),
            }
        }
    }
}

/// Take Corral's entries out, leaving everything else exactly as it was.
///
/// A file Corral never wrote into needs no write at all, and a refused read
/// is still a refusal: uninstall never falls back to deleting a file it could
/// not parse.
pub fn uninstall(
    target: &Target,
    relay: &RelayInvocation,
    now: SystemTime,
    state_dir: &std::path::Path,
) -> Standing {
    let examined = examine(target, relay);
    if let Standing::Refused(trigger) = examined.standing {
        return Standing::Refused(trigger);
    }
    let Some(original) = examined.original else {
        return Standing::NotInstalled;
    };
    if matches!(examined.standing, Standing::NotInstalled) {
        return Standing::NotInstalled;
    }
    match file::replace(target, &original, now, state_dir, |document| {
        document.uninstall();
        Ok(())
    }) {
        Ok(()) => Standing::NotInstalled,
        Err(trigger) => Standing::Refused(trigger),
    }
}

/// Record that the user chose the integration, then install it.
///
/// Intent before the write, so a refused merge still leaves a decision the
/// user can act on: the file is evidence of what is installed and never the
/// record of what was chosen (D5/D6). Enabling is also the explicit user
/// action that re-arms repair: a breaker opened because something kept
/// undoing Corral's integration stays open until the user says so, and this
/// is them saying so (grill Q4′).
///
/// `None` is an operation that did not run at all, which is not a standing.
pub async fn enable(
    state: &Arc<DaemonState>,
    target: Target,
    relay: RelayInvocation,
    now: SystemTime,
    state_dir: PathBuf,
) -> Result<Option<Standing>, StateError> {
    let _turn = state.integration_turns().take(target.provider()).await;
    record_intent(state, target.provider(), IntegrationIntent::Enabled, now).await?;
    restore_repair_authority(state, &target).await?;
    Ok(off_the_reactor(move || install(&target, &relay, now, &state_dir)).await)
}

/// Record that the user withdrew the integration, then take it out.
pub async fn disable(
    state: &Arc<DaemonState>,
    target: Target,
    relay: RelayInvocation,
    now: SystemTime,
    state_dir: PathBuf,
) -> Result<Option<Standing>, StateError> {
    let _turn = state.integration_turns().take(target.provider()).await;
    record_intent(state, target.provider(), IntegrationIntent::Disabled, now).await?;
    Ok(off_the_reactor(move || uninstall(&target, &relay, now, &state_dir)).await)
}

/// Clear every repair breaker this file could have opened.
///
/// Both repairable drift classes, because the user's action is about the
/// integration and not about whichever recurrence happened to trip first.
/// Ownership conflict is deliberately not among them: it is never
/// auto-repaired, so it never had a breaker to clear.
async fn restore_repair_authority(
    state: &Arc<DaemonState>,
    target: &Target,
) -> Result<(), StateError> {
    let Ok(named) = ProviderId::new(target.provider().as_str()) else {
        return Ok(());
    };
    for drift in [RepairableDrift::Missing, RepairableDrift::OldRepresentation] {
        state
            .restore_repair_authority(RepairFingerprint::new(
                named.clone(),
                target.config_target(),
                drift,
            ))
            .await?;
    }
    Ok(())
}

async fn record_intent(
    state: &Arc<DaemonState>,
    provider: KnownProvider,
    intent: IntegrationIntent,
    now: SystemTime,
) -> Result<(), StateError> {
    let Ok(named) = ProviderId::new(provider.as_str()) else {
        // A name this build declares cannot fail the domain's own rule; if it
        // ever did, the honest answer is to change nothing rather than record
        // a decision under a name nothing else will match.
        tracing::warn!("a provider name this build declares is not a usable provider id");
        return Ok(());
    };
    state.set_integration_intent(named, intent, now).await
}

/// Bring drift back to what intent says should be installed, inside the
/// authority that intent grants.
///
/// Runs at daemon start and inside explicit named operations — never on a
/// timer, never mid-run, never once per hook (ADR 0013 D5). Three things have
/// to be true before a byte is written: the user chose `Enabled`, the drift
/// is one Corral can prove it owns, and the repair budget for that exact
/// drift is not spent.
pub async fn repair(
    state: &Arc<DaemonState>,
    target: Target,
    relay: RelayInvocation,
    now: SystemTime,
    state_dir: PathBuf,
) -> Result<Standing, StateError> {
    let _turn = state.integration_turns().take(target.provider()).await;
    let provider = match ProviderId::new(target.provider().as_str()) {
        Ok(provider) => provider,
        Err(error) => {
            tracing::warn!(%error, "a provider name this build declares is not a usable provider id");
            return Ok(examined(&target, &relay).await);
        }
    };
    let intent = state.integration_intent(provider.clone()).await?;
    if !matches!(
        intent.map(|recorded| recorded.intent()),
        Some(IntegrationIntent::Enabled)
    ) {
        // No decision is not a decision to install. During PR7 dogfood the
        // only thing that installs is an explicit `corral integration enable`
        // (grill Q2), so a daemon start finds nothing to maintain here.
        return Ok(examined(&target, &relay).await);
    }

    let standing = examined(&target, &relay).await;
    let drift = match standing {
        Standing::NotInstalled => RepairableDrift::Missing,
        Standing::Drifted(drift) => drift,
        // An ownership conflict is never auto-repaired, so it must never
        // spend repair budget or open a breaker (grill Q4′). It goes straight
        // to the user with its cause.
        other => return Ok(other),
    };

    let fingerprint = RepairFingerprint::new(provider, target.config_target(), drift);
    let authority = state.authorize_repair(fingerprint.clone(), now).await?;
    if !authority.permits_repair() {
        return Ok(match authority {
            RepairAuthority::Withdrawn { since } => Standing::RepairWithheld { since },
            RepairAuthority::Available { .. } => standing,
        });
    }

    let Some(repaired) = off_the_reactor(move || install(&target, &relay, now, &state_dir)).await
    else {
        return Ok(standing);
    };
    if matches!(repaired, Standing::Installed) {
        // Recorded only on success: a refused merge must not spend the budget
        // a real recurrence needs.
        state.record_repair(fingerprint, now).await?;
    }
    Ok(repaired)
}

/// Repair every provider the user enabled, once, at daemon start.
///
/// Failure is reported and never fatal: a provider file Corral cannot repair
/// costs awareness of that provider's sessions, and a daemon that refused to
/// serve over it would cost the user everything else Corral does.
pub async fn repair_at_startup(state: Arc<DaemonState>) {
    let now = SystemTime::now();
    let state_dir = state.state_dir().to_path_buf();
    for provider in KnownProvider::ALL {
        let (Ok(target), Ok(relay)) = (
            Target::resolve(provider),
            RelayInvocation::compose_global(provider),
        ) else {
            tracing::warn!(
                provider = provider.as_str(),
                "corral cannot name its own integration, so none was checked"
            );
            continue;
        };
        match repair(&state, target, relay, now, state_dir.clone()).await {
            Ok(Standing::Installed) => {}
            Ok(standing) => tracing::info!(
                provider = provider.as_str(),
                ?standing,
                "the provider integration is not currently delivering"
            ),
            Err(error) => tracing::warn!(
                %error,
                provider = provider.as_str(),
                "the integration state could not be read"
            ),
        }
    }
}

fn write_entries(
    target: &Target,
    relay: &RelayInvocation,
    original: Option<file::Read>,
    now: SystemTime,
    state_dir: &std::path::Path,
) -> Result<(), Trigger> {
    let original = original.unwrap_or_else(|| file::Read::absent(target));
    file::replace(target, &original, now, state_dir, |document| {
        document.install(relay)
    })
}

/// `status`, off the reactor, for the callers that are already async.
///
/// A read that could not be performed is reported as not installed rather
/// than as a refusal: nothing was found and nothing was examined, and naming
/// a trigger would blame the user's file for the daemon's own failure.
async fn examined(target: &Target, relay: &RelayInvocation) -> Standing {
    let (target, relay) = (target.clone(), relay.clone());
    off_the_reactor(move || status(&target, &relay))
        .await
        .unwrap_or(Standing::NotInstalled)
}

fn examine(target: &Target, relay: &RelayInvocation) -> Examined {
    match file::read(target) {
        Err(trigger) => Examined {
            original: None,
            standing: Standing::Refused(trigger),
        },
        Ok(None) => Examined {
            original: None,
            standing: Standing::NotInstalled,
        },
        Ok(Some(read)) => {
            let standing = read.standing(relay);
            Examined {
                original: Some(read),
                standing,
            }
        }
    }
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
