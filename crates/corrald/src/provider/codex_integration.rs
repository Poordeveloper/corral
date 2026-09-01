//! Corral's `notify` value in a user's own Codex `config.toml`: how it is
//! written, recognized, and removed.
//!
//! The opposite representation policy from Claude's, and for measured reasons
//! (ADR 0013 D3, grill Q3′). TOML legally carries the user's comments, Codex
//! itself patches this file surgically and preserves everything it did not
//! write, and a malformed `config.toml` is *fatal* to the Codex CLI rather
//! than silently ignored. So this module edits in place with a
//! format-preserving parser: comments, key order, and whitespace outside
//! Corral's own value survive exactly as the user left them.
//!
//! `notify` is a single value, not a list of entries. Codex has one notifier
//! slot and Corral never takes it from somebody else: an occupied slot is
//! preserved, degraded, and explained (ADR 0013 D7, grill Q3). There is no
//! force, no takeover, and no chaining.

use toml_edit::{Array, DocumentMut, Item, Value};

use super::launch::RelayInvocation;
use super::relay_grammar;

/// The one key Corral owns in this file.
pub const NOTIFY: &str = "notify";

/// What examining a `config.toml` concluded about the notifier slot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Slot {
    /// No `notify` key. Corral may set it (ADR 0013 D7's only writable case).
    Absent,
    /// Corral's own notifier, at the version this binary writes.
    Current,
    /// Corral's own notifier, at a version this binary can bring forward.
    Stale,
    /// Corral's own notifier, declaring a version this binary does not
    /// understand. Left untouched and reported.
    Newer(u32),
    /// Somebody else's notifier. Never overwritten, never wrapped, never
    /// chained — Corral degrades and asks the user to resolve it.
    Occupied,
    /// A `notify` whose type is not the array Codex requires. Measured: Codex
    /// refuses to start on this file, so it is somebody else's problem to fix
    /// and never something Corral silently normalizes.
    Malformed,
}

/// What the notifier slot holds right now.
pub fn slot(document: &DocumentMut, relay: &RelayInvocation) -> Slot {
    let Some(item) = document.get(NOTIFY) else {
        return Slot::Absent;
    };
    let Some(words) = argv_words(item) else {
        return Slot::Malformed;
    };
    if !relay_grammar::words_invoke_corral_relay(&words) {
        return Slot::Occupied;
    }
    match relay_grammar::version_in(&words) {
        Some(version) if version > corral_protocol::hook::INTEGRATION_VERSION => {
            Slot::Newer(version)
        }
        _ if words == expected_words(relay) => Slot::Current,
        _ => Slot::Stale,
    }
}

/// Put Corral's notifier in the slot.
///
/// The caller decides whether it may: `slot` reports `Occupied` for somebody
/// else's notifier, and the engine refuses on that before reaching here.
pub fn install(document: &mut DocumentMut, relay: &RelayInvocation) {
    let mut array = Array::new();
    for word in relay.words() {
        array.push(word);
    }
    document[NOTIFY] = Item::Value(Value::Array(array));
}

/// Take Corral's notifier out, and only Corral's.
///
/// A slot holding somebody else's notifier is left exactly as it is: an
/// uninstall that cleared it would hand the user a silent notifier loss in
/// exchange for tidiness.
pub fn uninstall(document: &mut DocumentMut) {
    let is_corrals = document
        .get(NOTIFY)
        .and_then(argv_words)
        .is_some_and(|words| relay_grammar::words_invoke_corral_relay(&words));
    if is_corrals {
        document.remove(NOTIFY);
    }
}

/// The words this binary would write, for comparing against what is there.
fn expected_words(relay: &RelayInvocation) -> Vec<String> {
    relay.words().map(str::to_owned).collect()
}

/// One `notify` value as argv words, or `None` when it is not the array of
/// strings Codex requires.
fn argv_words(item: &Item) -> Option<Vec<String>> {
    let array = item.as_array()?;
    array
        .iter()
        .map(|value| value.as_str().map(str::to_owned))
        .collect()
}

#[cfg(test)]
#[path = "codex_integration_tests.rs"]
mod tests;
