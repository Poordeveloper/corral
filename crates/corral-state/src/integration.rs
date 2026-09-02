//! The Corral-owned integration tables: what the user chose, and how much
//! self-repair authority that choice still carries.
//!
//! These rows are not a projection. No event derives them, a rebuild leaves
//! them alone, and losing them would forget a user decision rather than a
//! summary that can be recomputed — which is exactly the durable-state law's
//! second kind (AGENTS.md §Durable state).

use std::time::SystemTime;

use corral_core::{
    IntegrationIntent, ProviderId, RepairAuthority, RepairBudget, RepairFingerprint,
};
use rusqlite::{Connection, OptionalExtension, Transaction};

use crate::encoding;
use crate::error::StateError;

/// What the store holds about one provider's integration.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RecordedIntent {
    intent: IntegrationIntent,
    changed_at: SystemTime,
}

impl RecordedIntent {
    #[must_use]
    pub fn intent(&self) -> IntegrationIntent {
        self.intent
    }

    /// When the user last decided. Carried so a surface can say how old the
    /// choice is without inferring it from whatever the file looks like now.
    #[must_use]
    pub fn changed_at(&self) -> SystemTime {
        self.changed_at
    }
}

pub(crate) fn intent(
    connection: &Connection,
    provider: &ProviderId,
) -> Result<Option<RecordedIntent>, StateError> {
    let row = connection
        .query_row(
            "SELECT intent, changed_at_ms FROM integration_intent WHERE provider = ?1",
            [provider.as_str()],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
        )
        .optional()?;
    let Some((token, changed_at_ms)) = row else {
        return Ok(None);
    };
    Ok(Some(RecordedIntent {
        intent: encoding::integration_intent_from_token(&token)?,
        changed_at: encoding::from_millis(changed_at_ms),
    }))
}

pub(crate) fn set_intent(
    transaction: &Transaction<'_>,
    provider: &ProviderId,
    intent: IntegrationIntent,
    at: SystemTime,
) -> Result<(), StateError> {
    transaction.execute(
        "INSERT INTO integration_intent (provider, intent, changed_at_ms)
         VALUES (?1, ?2, ?3)
         ON CONFLICT (provider) DO UPDATE SET intent = ?2, changed_at_ms = ?3",
        rusqlite::params![
            provider.as_str(),
            encoding::integration_intent_token(intent),
            encoding::millis(at)?
        ],
    )?;
    Ok(())
}

/// Decide whether an automatic repair of this fingerprint may proceed, opening
/// the breaker when the budget is already spent.
///
/// The check writes, which is the point: the breaker opens at the moment the
/// next repair would have happened, so the decision survives the restart that
/// would otherwise re-arm it (grill Q4′). Repairs older than the window are
/// pruned here rather than on a timer — a fingerprint nobody asks about costs
/// nothing to keep.
pub(crate) fn authorize_repair(
    transaction: &Transaction<'_>,
    fingerprint: &RepairFingerprint,
    now: SystemTime,
    budget: RepairBudget,
) -> Result<RepairAuthority, StateError> {
    if let Some(since) = breaker_opened_at(transaction, fingerprint)? {
        return Ok(RepairAuthority::Withdrawn { since });
    }
    prune(transaction, fingerprint, budget.window_starts(now))?;
    let spent = repairs_in_window(transaction, fingerprint, budget.window_starts(now))?;
    if spent >= budget.repairs() {
        open_breaker(transaction, fingerprint, now)?;
        return Ok(RepairAuthority::Withdrawn { since: now });
    }
    Ok(RepairAuthority::Available {
        remaining: budget.repairs() - spent,
    })
}

/// Record that a repair happened. Called after the write succeeded, so a
/// refused merge never spends budget.
pub(crate) fn record_repair(
    transaction: &Transaction<'_>,
    fingerprint: &RepairFingerprint,
    at: SystemTime,
) -> Result<(), StateError> {
    transaction.execute(
        "INSERT INTO integration_repairs (provider, config_target, drift_class, repaired_at_ms)
         VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params![
            fingerprint.provider().as_str(),
            encoding::config_target_token(fingerprint.target()),
            encoding::repairable_drift_token(fingerprint.drift()),
            encoding::millis(at)?,
        ],
    )?;
    Ok(())
}

/// Clear the breaker and the history behind it, after an explicit user
/// reconciliation re-established ownership.
///
/// The history goes with the breaker: leaving the old repairs would re-open it
/// on the next drift, which would make the user's action mean nothing.
pub(crate) fn restore_authority(
    transaction: &Transaction<'_>,
    fingerprint: &RepairFingerprint,
) -> Result<(), StateError> {
    let provider = fingerprint.provider().as_str();
    let target = encoding::config_target_token(fingerprint.target());
    let drift = encoding::repairable_drift_token(fingerprint.drift());
    transaction.execute(
        "DELETE FROM integration_breakers
         WHERE provider = ?1 AND config_target = ?2 AND drift_class = ?3",
        rusqlite::params![provider, target, drift],
    )?;
    transaction.execute(
        "DELETE FROM integration_repairs
         WHERE provider = ?1 AND config_target = ?2 AND drift_class = ?3",
        rusqlite::params![provider, target, drift],
    )?;
    Ok(())
}

fn breaker_opened_at(
    connection: &Connection,
    fingerprint: &RepairFingerprint,
) -> Result<Option<SystemTime>, StateError> {
    let opened = connection
        .query_row(
            "SELECT opened_at_ms FROM integration_breakers
             WHERE provider = ?1 AND config_target = ?2 AND drift_class = ?3",
            rusqlite::params![
                fingerprint.provider().as_str(),
                encoding::config_target_token(fingerprint.target()),
                encoding::repairable_drift_token(fingerprint.drift()),
            ],
            |row| row.get::<_, i64>(0),
        )
        .optional()?;
    Ok(opened.map(encoding::from_millis))
}

fn open_breaker(
    transaction: &Transaction<'_>,
    fingerprint: &RepairFingerprint,
    at: SystemTime,
) -> Result<(), StateError> {
    transaction.execute(
        "INSERT INTO integration_breakers (provider, config_target, drift_class, opened_at_ms)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT (provider, config_target, drift_class) DO NOTHING",
        rusqlite::params![
            fingerprint.provider().as_str(),
            encoding::config_target_token(fingerprint.target()),
            encoding::repairable_drift_token(fingerprint.drift()),
            encoding::millis(at)?,
        ],
    )?;
    Ok(())
}

fn repairs_in_window(
    connection: &Connection,
    fingerprint: &RepairFingerprint,
    since: SystemTime,
) -> Result<u32, StateError> {
    let counted: i64 = connection.query_row(
        "SELECT COUNT(*) FROM integration_repairs
         WHERE provider = ?1 AND config_target = ?2 AND drift_class = ?3
           AND repaired_at_ms >= ?4",
        rusqlite::params![
            fingerprint.provider().as_str(),
            encoding::config_target_token(fingerprint.target()),
            encoding::repairable_drift_token(fingerprint.drift()),
            encoding::millis(since)?,
        ],
        |row| row.get(0),
    )?;
    Ok(u32::try_from(counted).unwrap_or(u32::MAX))
}

fn prune(
    transaction: &Transaction<'_>,
    fingerprint: &RepairFingerprint,
    before: SystemTime,
) -> Result<(), StateError> {
    transaction.execute(
        "DELETE FROM integration_repairs
         WHERE provider = ?1 AND config_target = ?2 AND drift_class = ?3
           AND repaired_at_ms < ?4",
        rusqlite::params![
            fingerprint.provider().as_str(),
            encoding::config_target_token(fingerprint.target()),
            encoding::repairable_drift_token(fingerprint.drift()),
            encoding::millis(before)?,
        ],
    )?;
    Ok(())
}
