//! Corral's entries in a user's own Claude Code `settings.json`: how they are
//! written, how they are recognized as Corral's, and how they are removed.
//!
//! The representation policy here is measured, not chosen for symmetry with
//! Codex (ADR 0013 D3, grill Q3′). Claude Code rejects JSONC outright — a
//! comment makes the *whole* settings file invalid and silently drops every
//! setting in it, not just hooks — and Claude reserializes the entire document
//! on its own writes, normalizing indentation and key order. So this module
//! parses strict JSON, merges structurally, preserves what it does not
//! understand by value, and reserializes. It never emits a comment, and it
//! does not try to preserve byte layout that the provider itself discards.
//!
//! Ownership is structural: an entry is Corral's when its command invokes
//! Corral's relay (ADR 0013 D2). Never by position, never by a marker, never
//! by resemblance.

use serde_json::{Map, Value, json};

use super::launch::RelayInvocation;

/// The top-level key hook entries live under.
const HOOKS: &str = "hooks";

/// The key whose truth silences every hook, at any layer.
///
/// Read and never written at global scope. Measured 2026-09-02: `true` at any
/// one of the four effective layers silences all hooks, and three of those
/// layers are not Corral's to write. Writing `false` here would be Corral
/// overriding a stated user intent, which ADR 0006 permanently bans — the
/// per-launch injected file may say it, because that file is a document Corral
/// wrote entirely.
pub const DISABLE_ALL_HOOKS: &str = "disableAllHooks";

/// The shape of one hook entry's inner command.
const HOOKS_ARRAY: &str = "hooks";
const COMMAND_TYPE: &str = "type";
const COMMAND: &str = "command";
const TYPE_COMMAND: &str = "command";

/// What a Corral-owned Claude entry looks like once written.
///
/// The guard is part of the entry, not decoration (ADR 0013 D8, grill Q1′).
/// Measured: a hook command whose path does not exist prints a visible error
/// on every prompt and every turn, and Claude judges the boundary by exit
/// status alone — so a guarded command makes a stale integration silent by
/// construction, whether or not an uninstaller ever runs.
///
/// The guard is the only shell syntax Corral writes. Nothing from a payload,
/// a prompt, an event, or a user string is ever interpolated here: the command
/// is Corral's own static invocation plus Corral's own quoted words.
fn guarded_command(relay: &RelayInvocation) -> String {
    format!("{} || true", relay.shell_command())
}

/// The entry Corral installs for one event.
fn entry(relay: &RelayInvocation) -> Value {
    json!({
        HOOKS_ARRAY: [{ COMMAND_TYPE: TYPE_COMMAND, COMMAND: guarded_command(relay) }]
    })
}

/// Whether this entry is one Corral wrote.
///
/// Structural: every command in the entry is examined, and the entry is
/// Corral's only when one of them invokes Corral's relay. An entry that merely
/// ends in `|| true` is somebody else's — the guard is not ownership evidence,
/// and treating it as such would let Corral delete a third party's fail-open
/// hook (grill Q1′).
pub fn is_corrals(entry: &Value) -> bool {
    commands_of(entry).any(super::relay_grammar::invokes_corral_relay)
}

/// Every command string inside one hook entry.
fn commands_of(entry: &Value) -> impl Iterator<Item = &str> {
    entry
        .get(HOOKS_ARRAY)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|hook| hook.get(COMMAND).and_then(Value::as_str))
}

/// The integration version a Corral-owned entry declares, if it declares one.
///
/// `None` from an entry that is Corral's means an artifact older than the
/// discriminant: repairable, because this binary understands everything it
/// wrote before the flag existed.
pub fn declared_version(entry: &Value) -> Option<u32> {
    commands_of(entry).find_map(super::relay_grammar::declared_version)
}

/// The events Corral installs globally, and the order they are written in.
///
/// ADR 0004 D6's five, now at global scope (ADR 0013 D2). Ordered so that two
/// installs of the same version produce the same bytes, which is what lets a
/// caller compare an installed file against what this binary would write.
pub const EVENTS: [&str; 5] = [
    "SessionStart",
    "UserPromptSubmit",
    "Stop",
    "Notification",
    "SessionEnd",
];

/// What examining a settings document concluded about Corral's entries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Installed {
    /// No entry in this document invokes Corral's relay.
    Absent,
    /// Every event carries a Corral entry at a version this binary wrote.
    Current,
    /// Corral owns entries here, but not the set or the version this binary
    /// would write: a repair can bring them forward.
    Stale,
    /// An entry declares a version this binary does not understand. An older
    /// Corral never rewrites what a newer Corral wrote (ADR 0013 D2).
    Newer(u32),
}

/// What Corral's entries look like in this document right now.
pub fn installed(document: &Value, relay: &RelayInvocation) -> Installed {
    let mut owned = 0_usize;
    let mut current = 0_usize;
    let expected = entry(relay);
    for event in EVENTS {
        for candidate in entries_for(document, event) {
            if !is_corrals(candidate) {
                continue;
            }
            owned += 1;
            match declared_version(candidate) {
                Some(version) if version > corral_protocol::hook::INTEGRATION_VERSION => {
                    return Installed::Newer(version);
                }
                _ => {}
            }
            if candidate == &expected {
                current += 1;
            }
        }
    }
    if owned == 0 {
        return Installed::Absent;
    }
    if current == EVENTS.len() && owned == EVENTS.len() {
        return Installed::Current;
    }
    Installed::Stale
}

fn entries_for<'a>(document: &'a Value, event: &str) -> impl Iterator<Item = &'a Value> {
    document
        .get(HOOKS)
        .and_then(|hooks| hooks.get(event))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
}

/// Put Corral's entries into this document, replacing whatever Corral wrote
/// before and leaving everything else exactly as it was.
///
/// Additive by construction: a third party's entry on the same event keeps its
/// place in the array, and an event Corral does not install is not touched at
/// all.
pub fn install(document: &mut Value, relay: &RelayInvocation) {
    let entry = entry(relay);
    for event in EVENTS {
        let Some(existing) = event_array(document, event) else {
            // Only a document that is not an object reaches this, and the
            // engine refuses that shape before it calls in. Returning rather
            // than forcing one keeps a settings file Corral does not
            // understand a settings file Corral did not rewrite.
            return;
        };
        existing.retain(|candidate| !is_corrals(candidate));
        existing.push(entry.clone());
    }
}

/// Take Corral's entries out, and nothing else.
///
/// Empty containers Corral itself created are removed as it goes: an
/// uninstall that left `"hooks": {}` behind would leave the user's file
/// changed in a way they did not ask for and Corral did not need.
pub fn uninstall(document: &mut Value) {
    let Some(hooks) = document.get_mut(HOOKS).and_then(Value::as_object_mut) else {
        return;
    };
    for event in EVENTS {
        let Some(entries) = hooks.get_mut(event).and_then(Value::as_array_mut) else {
            continue;
        };
        entries.retain(|candidate| !is_corrals(candidate));
        if entries.is_empty() {
            hooks.remove(event);
        }
    }
    if hooks.is_empty()
        && let Some(object) = document.as_object_mut()
    {
        object.remove(HOOKS);
    }
}

/// The array of entries for one event, created if this document has none.
///
/// `None` only for a document that is not an object. The containers Corral
/// creates on the way are its own; a value of an unexpected type is replaced
/// nowhere here, because the engine refuses that shape as a D4 trigger before
/// any of this runs.
fn event_array<'a>(document: &'a mut Value, event: &str) -> Option<&'a mut Vec<Value>> {
    let hooks = document
        .as_object_mut()?
        .entry(HOOKS)
        .or_insert_with(|| Value::Object(Map::new()));
    let event_entry = hooks
        .as_object_mut()?
        .entry(event)
        .or_insert_with(|| Value::Array(Vec::new()));
    event_entry.as_array_mut()
}

#[cfg(test)]
#[path = "claude_integration_tests.rs"]
mod tests;
