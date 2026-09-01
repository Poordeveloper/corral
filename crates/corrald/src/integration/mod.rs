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
use std::time::SystemTime;

use corral_core::{
    ConfigTarget, IntegrationIntent, RepairAuthority, RepairFingerprint, RepairableDrift,
};

use crate::provider::KnownProvider;
use crate::provider::launch::RelayInvocation;

pub use trigger::Trigger;

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
pub fn status(target: &Target, relay: &RelayInvocation) -> Standing {
    examine(target, relay).standing
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

/// Bring drift back to what intent says should be installed, inside the
/// authority that intent grants.
///
/// Runs at daemon start and inside explicit named operations — never on a
/// timer, never mid-run, never once per hook (ADR 0013 D5). Three things have
/// to be true before a byte is written: the user chose `Enabled`, the drift
/// is one Corral can prove it owns, and the repair budget for that exact
/// drift is not spent.
pub async fn repair(
    state: &std::sync::Arc<crate::state::DaemonState>,
    target: &Target,
    relay: &RelayInvocation,
    now: SystemTime,
    state_dir: &std::path::Path,
) -> Result<Standing, corral_state::StateError> {
    let provider = match corral_core::ProviderId::new(target.provider().as_str()) {
        Ok(provider) => provider,
        Err(error) => {
            tracing::warn!(%error, "a provider name this build declares is not a usable provider id");
            return Ok(status(target, relay));
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
        return Ok(status(target, relay));
    }

    let standing = status(target, relay);
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

    let repaired = install(target, relay, now, state_dir);
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
pub async fn repair_at_startup(state: std::sync::Arc<crate::state::DaemonState>) {
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
        match repair(&state, &target, &relay, now, &state_dir).await {
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
