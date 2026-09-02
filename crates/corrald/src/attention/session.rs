//! What one Session's attention is right now, and the item that hangs off it.
//!
//! The engine derives a main state from evidence at an instant; this is the
//! part that remembers the last one, so that "since when" and "is this still
//! the same item" have answers (ADR 0015 D7).

use std::time::SystemTime;

use corral_core::{AttentionItemId, AttentionReason, AttentionState, MainState};

use super::Derived;

/// A Session's current attention item: born when the main state entered
/// Needs You or Ready, alive for exactly as long as it stays there.
///
/// The identity is ephemeral by decision (grill Q19): minted here, kept
/// across an evidence-source change for the same blocker, replaced when the
/// state is left and re-entered, never rebuilt across a restart.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Item {
    id: AttentionItemId,
    reason: AttentionReason,
    since: SystemTime,
    acknowledged: bool,
}

impl Item {
    #[must_use]
    pub fn id(&self) -> AttentionItemId {
        self.id
    }

    #[must_use]
    pub fn reason(&self) -> AttentionReason {
        self.reason
    }

    #[must_use]
    pub fn since(&self) -> SystemTime {
        self.since
    }

    #[must_use]
    pub fn acknowledged(&self) -> bool {
        self.acknowledged
    }
}

/// Why an item stopped being current. Invalidation never rings; only a new
/// item does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ItemEnd {
    /// A fresher entitled claim moved the session on: the blocker cleared, a
    /// turn started.
    Resolved,
    /// The claim behind it passed its horizon; the last known fact stays as
    /// secondary text.
    Rotted,
    /// The runtime ended: "Exited before you responded".
    Exited,
}

/// What applying a derivation changed, for the journal and the notifier.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transition {
    Unchanged,
    /// The main state changed and no item was born or ended by it.
    StateChanged {
        from: MainState,
        to: MainState,
    },
    /// A new item became actionable — the one moment a notification may be
    /// emitted for it.
    ItemBorn(AttentionItemId),
    ItemEnded {
        item: AttentionItemId,
        end: ItemEnd,
    },
}

/// What an acknowledgement did (grill Q18).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Acknowledgement {
    Acknowledged,
    /// The id names an item that is no longer current. Nothing happened to
    /// the item that replaced it.
    StaleAttentionItem,
    NoCurrentItem,
}

/// The tracker for one Session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SessionAttention {
    state: AttentionState,
    item: Option<Item>,
}

impl SessionAttention {
    /// Unknown from the start: nothing has been derived, and saying so is the
    /// honest state rather than a placeholder.
    #[must_use]
    pub fn new(created_at: SystemTime) -> Self {
        Self {
            state: AttentionState::unknown(created_at, None),
            item: None,
        }
    }

    #[must_use]
    pub fn state(&self) -> AttentionState {
        self.state
    }

    #[must_use]
    pub fn item(&self) -> Option<Item> {
        self.item
    }

    /// Take a derivation made at `now` and record what it changed.
    pub fn apply(&mut self, derived: Derived, now: SystemTime) -> Transition {
        let from = self.state.main();
        let to = derived.main;
        if from == to {
            // The same state re-derived: the last known fact may have moved,
            // the item and its identity have not.
            if to == MainState::Unknown {
                self.state = AttentionState::unknown(self.state.since(), derived.last_known);
            }
            return Transition::Unchanged;
        }
        self.state = derived.into_state(now);
        let ended = self.item.take().map(|item| Transition::ItemEnded {
            item: item.id,
            end: match to {
                MainState::Exited => ItemEnd::Exited,
                MainState::Unknown => ItemEnd::Rotted,
                MainState::Working | MainState::NeedsYou | MainState::Ready => ItemEnd::Resolved,
            },
        });
        let reason = match to {
            MainState::NeedsYou => Some(AttentionReason::NeedsInput),
            MainState::Ready => Some(AttentionReason::TurnComplete),
            MainState::Working | MainState::Unknown | MainState::Exited => None,
        };
        match (ended, reason) {
            (_, Some(reason)) => {
                let item = Item {
                    id: AttentionItemId::mint(),
                    reason,
                    since: now,
                    acknowledged: false,
                };
                self.item = Some(item);
                Transition::ItemBorn(item.id)
            }
            (Some(ended), None) => ended,
            (None, None) => Transition::StateChanged { from, to },
        }
    }

    /// Acknowledge exactly this item, if it is still the current one.
    pub fn acknowledge(&mut self, id: AttentionItemId) -> Acknowledgement {
        match self.item.as_mut() {
            None => Acknowledgement::NoCurrentItem,
            Some(item) if item.id == id => {
                item.acknowledged = true;
                Acknowledgement::Acknowledged
            }
            Some(_) => Acknowledgement::StaleAttentionItem,
        }
    }

    /// Open succeeded — the data channel bound and the first snapshot served.
    ///
    /// Viewing acknowledges a Ready item and never a Needs You one
    /// (`PRODUCT.md` §7): a blocked request is cleared by resolution or by an
    /// explicit acknowledgement, not by looking at it.
    pub fn opened(&mut self) {
        if let Some(item) = self.item.as_mut()
            && item.reason == AttentionReason::TurnComplete
        {
            item.acknowledged = true;
        }
    }
}

#[cfg(test)]
#[path = "session_tests.rs"]
mod tests;
