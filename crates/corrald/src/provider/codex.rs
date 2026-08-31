//! Codex: the one place this build knows anything about it.
//!
//! The same four boundaries `super::claude` names — composing a launch,
//! composing a resume, reading ingress, validating what a payload claims —
//! answered by a provider that shares almost none of Claude's mechanics. There
//! is no hooks system and no settings file: one `notify` program, invoked with
//! the notification JSON as its final argument, configured by a value that
//! replaces rather than merges (ADR 0009).
//!
//! Managed Codex is the interactive TUI under a Corral-owned PTY, and that is
//! the whole supported surface (ADR 0009 D1, grill Q7).
//!
//! Behaviour verified first-party against codex-cli 0.145.0; the evidence and
//! its limits are `docs/references/2026-08-31-pr6-codex-notify-matrix.md`. A
//! version outside that record is not gated — launch proceeds, evidence is
//! best-effort, and unknown notify types assert nothing.

use std::ffi::OsString;
use std::fmt::Write as _;

use corral_core::ExternalId;
use serde_json::Value;
use tracing::warn;

use super::launch::RelayInvocation;
use super::{
    AgentFactKind, ArgumentRefused, LaunchIntent, ProviderLaunch, ProviderReport, Uninterpretable,
};

/// How Corral names this provider, on the wire and in bindings.
pub const PROVIDER: &str = "codex";

/// The executable a managed launch runs.
///
/// Resolved through `PATH` by the spawn, exactly as a person's own shell would
/// resolve it: Corral integrates the Codex the user installed.
pub const PROGRAM: &str = "codex";

/// The flag that overrides one configuration value for this launch alone.
///
/// Its value is `key=value` with the value parsed as TOML, and it sits above
/// the user, profile, and project layers — measured, not read off
/// documentation (spike scenario 3).
const CONFIG_FLAG: &str = "-c";
const CONFIG_FLAG_LONG: &str = "--config";

/// The configuration key that names the program Codex runs when a turn
/// completes.
///
/// One value, not a merged list: overriding it substitutes Corral's notifier
/// for the user's own, for this process only. That is a managed-launch
/// capability substitution, disclosed as such, and never a claim that the
/// original notifier was preserved (ADR 0009 D4).
const NOTIFY_KEY: &str = "notify";

/// The subcommand that continues a previous interactive session.
///
/// The verb the provider itself prints on exit: `To continue this session, run
/// codex resume <thread-id>` (spike scenario 2).
const RESUME_VERB: &str = "resume";

/// The notify type this build has a word for, and what it means in Corral's
/// vocabulary.
///
/// One, because one is what Codex fires today and one is what the spike
/// captured. A later release that adds types is additive: each new one is
/// mapped deliberately or asserts nothing (ADR 0004 D3).
const NOTIFIED: [(&str, AgentFactKind); 1] = [("agent-turn-complete", AgentFactKind::TurnEnded)];

/// The payload field carrying the provider session identity.
const THREAD_ID: &str = "thread-id";

/// Compose this provider's launch: the notify override, then the rest.
///
/// Infallible, and that is the difference the second provider makes visible.
/// Claude's injection is a file that can fail to be written; Codex's is a word
/// in its own command line, so there is nothing to publish, nothing to clean
/// up, and no artifact for the launch-file lifecycle to own (ADR 0009 D1).
pub fn compose_launch(intent: &LaunchIntent, relay: &RelayInvocation) -> ProviderLaunch {
    ProviderLaunch {
        argv: match intent {
            LaunchIntent::Fresh { args } => launch_argv(relay, args),
            LaunchIntent::Continue { external_id } => resume_argv(relay, external_id),
        },
        artifact: None,
    }
}

/// The argv of a fresh managed Codex session.
///
/// The override goes **first**, before anything the caller passed, for the
/// reason PR5 established against Claude and the spike confirmed here: nothing
/// Corral needs may sit where caller input can reach it. Placed last it can be
/// turned into prompt text by a caller's `--`, or eaten as the value of a
/// caller's trailing value-taking flag — both launching a session that looks
/// managed and can never report.
///
/// What position cannot answer is a caller repeating the flag, since the last
/// `-c notify` is the one that takes effect (spike scenario 5) — which is what
/// `refuse_arguments` is for.
fn launch_argv(relay: &RelayInvocation, args: &[String]) -> Vec<OsString> {
    let mut argv = vec![OsString::from(CONFIG_FLAG), notify_override(relay)];
    argv.extend(args.iter().map(OsString::from));
    argv
}

/// The argv that continues the provider's own session as a new Run.
///
/// A fresh token in a fresh override, because a continuation is a new Run and
/// evidence is attributed per launch (ADR 0004 D5). Carries no caller
/// arguments.
///
/// The override goes first here for the sharper version of the same reason:
/// the word after `resume` is a **provider** string. `ExternalId` bounds its
/// length and refuses characters that hide or reorder text, and that is all —
/// a payload naming a flag would, with the override placed after it, either
/// displace Corral's own or take it as a value.
fn resume_argv(relay: &RelayInvocation, external_id: &ExternalId) -> Vec<OsString> {
    vec![
        OsString::from(CONFIG_FLAG),
        notify_override(relay),
        OsString::from(RESUME_VERB),
        OsString::from(external_id.as_str()),
    ]
}

/// The `notify=[…]` assignment one launch is given.
///
/// A TOML array literal, because that is what the value half of `-c key=value`
/// is parsed as. Corral owns the quoting: the relay path is a filesystem path,
/// which may hold quotes, backslashes, and anything else a filesystem accepts,
/// and a value that does not round-trip is a launch that looks managed and can
/// never report.
fn notify_override(relay: &RelayInvocation) -> OsString {
    let words: Vec<String> = relay.words().map(toml_string).collect();
    OsString::from(format!("{NOTIFY_KEY}=[{}]", words.join(",")))
}

/// One word as a TOML basic string.
///
/// Escapes what TOML requires escaped and nothing that would change the value:
/// the quote and the backslash, the five named control characters, and every
/// other control character as `\uXXXX`. A path Corral cannot represent here
/// does not exist — every scalar has a spelling.
fn toml_string(raw: &str) -> String {
    let mut quoted = String::with_capacity(raw.len() + 2);
    quoted.push('"');
    for character in raw.chars() {
        match character {
            '"' => quoted.push_str("\\\""),
            '\\' => quoted.push_str("\\\\"),
            '\u{8}' => quoted.push_str("\\b"),
            '\t' => quoted.push_str("\\t"),
            '\n' => quoted.push_str("\\n"),
            '\u{c}' => quoted.push_str("\\f"),
            '\r' => quoted.push_str("\\r"),
            control if control.is_control() => {
                // Infallible into a String; the write is how a formatted
                // escape is appended, not something that can fail.
                let _ = write!(quoted, "\\u{:04X}", control as u32);
            }
            other => quoted.push(other),
        }
    }
    quoted.push('"');
    quoted
}

/// Refuse the provider arguments Corral cannot honour.
///
/// One reason, in every spelling this CLI accepts for it: a caller's own
/// `notify` override displaces Corral's, because the last one on the
/// invocation is the one that takes effect (spike scenario 5). That leaves a
/// launch Corral believes it is watching and is not.
///
/// The value half is never inspected. `-c notify=[]` disables the notifier,
/// `-c notify=["…"]` replaces it, and a spelling this build has not thought of
/// does one of the two: the key is what makes the argument Corral's, and
/// judging the value would be this list guessing at intent.
///
/// Version-sensitive by nature, exactly as Claude's is: this is a claim about
/// one provider's command line, held against the version the matrix records. A
/// flag a later release adds is a flag this list does not know, which is why a
/// managed launch also has to survive learning nothing — it does, as an
/// identity that never binds rather than a false one.
pub fn refuse_arguments(args: &[String]) -> Result<(), ArgumentRefused> {
    let mut args = args.iter().peekable();
    while let Some(argument) = args.next() {
        // Separated from its value, which is the ordinary spelling: the flag
        // says nothing on its own, so the argument named in the refusal is the
        // assignment the person actually wrote.
        if argument == CONFIG_FLAG || argument == CONFIG_FLAG_LONG {
            if let Some(assignment) = args.peek().filter(|next| names_notify(next)) {
                return Err(ArgumentRefused {
                    argument: (*assignment).clone(),
                });
            }
            continue;
        }
        // Joined to it, in each of the forms this CLI's parser accepts:
        // `-cnotify=…`, `-c=notify=…`, `--config=notify=…`.
        let joined = argument
            .strip_prefix(CONFIG_FLAG_LONG)
            .or_else(|| argument.strip_prefix(CONFIG_FLAG))
            .map(|rest| rest.strip_prefix('=').unwrap_or(rest));
        if joined.is_some_and(names_notify) {
            return Err(ArgumentRefused {
                argument: argument.clone(),
            });
        }
    }
    Ok(())
}

/// Whether one `key=value` override names the notify program.
///
/// The dotted form too: `-c` takes a dotted path into nested values, so a key
/// under `notify` is an override of the same setting whatever this build knows
/// about its shape.
fn names_notify(assignment: &str) -> bool {
    let Some((key, _)) = assignment.split_once('=') else {
        return false;
    };
    let key = key.trim();
    key == NOTIFY_KEY || key.starts_with(&format!("{NOTIFY_KEY}."))
}

/// Read one Codex notify payload as Corral facts.
///
/// Untrusted input. A payload that is not what this expects degrades to
/// diagnostics: nothing here panics, and nothing invents a fact a payload did
/// not carry (`ARCHITECTURE.md` §5).
pub fn interpret(payload: &str) -> Result<ProviderReport, Uninterpretable> {
    let document: Value = serde_json::from_str(payload).map_err(|_| Uninterpretable::Malformed)?;
    let notified = document
        .get("type")
        .and_then(Value::as_str)
        .ok_or(Uninterpretable::Malformed)?;
    let fact = NOTIFIED
        .iter()
        .find(|(name, _)| *name == notified)
        .map(|(_, fact)| *fact)
        .ok_or(Uninterpretable::UnknownEvent)?;

    // A known notification with no usable id is a fact without an identity,
    // not a malformed payload: the fact is still true of the launch the token
    // names. Refused separately from absent, and said out loud — a payload
    // carrying an id Corral will not hold is a provider doing something this
    // build did not expect, and treating it exactly like a payload that
    // carried none would leave a session that never binds and never explains
    // why.
    let identity = match document.get(THREAD_ID).and_then(Value::as_str) {
        None => None,
        Some(raw) => match ExternalId::new(raw) {
            Ok(id) => Some(id),
            Err(refusal) => {
                warn!(%refusal, "a provider reported an identity Corral cannot hold");
                None
            }
        },
    };

    Ok(ProviderReport {
        identity,
        fact,
        // Never an origin, and never `Unrecognized`. Codex reports no start at
        // all, so there is no spelling to normalize and nothing to fail to
        // recognize: unreported and unrecognizable are different facts
        // (ADR 0009 D3).
        origin: None,
    })
}

#[cfg(test)]
#[path = "codex_tests.rs"]
mod tests;
