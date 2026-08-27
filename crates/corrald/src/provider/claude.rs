//! Claude Code: the one place this build knows anything about it.
//!
//! Four named boundaries, which are the four things a provider integration
//! turns out to be: composing a launch, composing a resume, reading hook
//! ingress, and validating what a payload claims. A trait extracted from two
//! implementations will find them here rather than have to invent them
//! (grill Q5).
//!
//! Behaviour verified first-party against 2.1.247; the evidence and its limits
//! are `docs/references/2026-08-27-pr5-claude-code-hook-matrix.md`. A version
//! outside that record is not gated — launch proceeds, evidence is
//! best-effort, and unknown event names assert nothing.

use std::ffi::OsString;
use std::path::Path;

use corral_core::ExternalId;
use serde_json::{Value, json};

use super::{AgentFactKind, ProviderReport, SessionOrigin, Uninterpretable};

/// How Corral names this provider, on the wire and in bindings.
///
/// Never the reserved `corral` namespace, which records who minted an identity
/// rather than which agent is running (ADR 0008 D3).
pub const PROVIDER: &str = "claude";

/// The executable a managed launch runs.
///
/// Resolved through `PATH` by the spawn, exactly as a person's own shell would
/// resolve it: Corral integrates the Claude Code the user installed, and a
/// hardcoded location would integrate a different one.
pub const PROGRAM: &str = "claude";

/// The flag that loads one additional settings file for this launch alone.
///
/// First-party verified present on 2.1.247. The user's global and project
/// settings still load, their own hooks still run, and no strict flag is
/// passed: the read-only law is honored by never touching provider-owned
/// files, not by editing them carefully (ADR 0004 D6).
const SETTINGS_FLAG: &str = "--settings";

/// The flag that continues a named conversation.
const RESUME_FLAG: &str = "--resume";

/// The hook events Corral injects, and what each one means in Corral's
/// vocabulary.
///
/// `PreToolUse` / `PostToolUse` are deliberately absent: high-frequency, and
/// nothing in this phase consumes them. Adding an event later is additive
/// (ADR 0004 D6).
const INJECTED: [(&str, AgentFactKind); 5] = [
    ("SessionStart", AgentFactKind::SessionStarted),
    ("UserPromptSubmit", AgentFactKind::TurnStarted),
    ("Stop", AgentFactKind::TurnEnded),
    ("Notification", AgentFactKind::AwaitingInput),
    ("SessionEnd", AgentFactKind::SessionEnded),
];

/// The argv of a fresh managed Claude session.
///
/// The injected settings file goes **first**, before anything the caller
/// passed, and the two halves of that choice are both first-party evidence.
/// Placed last it survives nothing: a caller's `--` turns it into prompt text,
/// and a caller's trailing value-taking flag eats it as a value — both
/// launching a session that looks managed and can never report (matrix
/// scenario 10). Placed first, neither can reach it: a `--` after it is
/// harmless (scenario 12), and a caller's flag with a missing value fails the
/// launch loudly instead of degrading it silently.
///
/// The one thing position cannot answer is a caller repeating the flag, since
/// the last `--settings` is the one loaded (scenario 8) — which is what
/// `refuse_arguments` is for.
pub fn launch_argv(settings: &Path, args: &[String]) -> Vec<OsString> {
    let mut argv = vec![
        OsString::from(SETTINGS_FLAG),
        OsString::from(settings.as_os_str()),
    ];
    argv.extend(args.iter().map(OsString::from));
    argv
}

/// Why a caller's provider arguments cannot be passed through.
///
/// One reason, because there is one: an argument that would compete with the
/// hook injection. Everything else a person may want to pass is theirs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArgumentRefused {
    pub argument: String,
}

/// Refuse the one provider argument Corral cannot honour.
///
/// One, because position answers everything else: a caller repeating
/// `--settings` would displace Corral's own, since the last one is the one
/// loaded. Everything else a person may want to pass to their own agent is
/// theirs, including the separator.
///
/// Refused rather than dropped. Silently discarding a settings file a person
/// asked for would be Corral deciding their configuration, and silently
/// honouring it would be a session Corral believes it is watching and is not.
pub fn refuse_arguments(args: &[String]) -> Result<(), ArgumentRefused> {
    let equals = format!("{SETTINGS_FLAG}=");
    let competing =
        |argument: &&String| *argument == SETTINGS_FLAG || argument.starts_with(&equals);
    match args.iter().find(competing) {
        Some(argument) => Err(ArgumentRefused {
            argument: argument.clone(),
        }),
        None => Ok(()),
    }
}

/// The argv that continues the provider's own session as a new Run.
///
/// Verified first-party: `--settings` composes with `--resume`, and the
/// resumed run keeps the same `session_id` and transcript. Carries no
/// caller-supplied arguments — a resume is Corral continuing what it already
/// recorded, and arguments would make one Session's Runs differ in ways
/// nothing recorded.
pub fn resume_argv(external_id: &ExternalId, settings: &Path) -> Vec<OsString> {
    vec![
        OsString::from(RESUME_FLAG),
        OsString::from(external_id.as_str()),
        OsString::from(SETTINGS_FLAG),
        OsString::from(settings.as_os_str()),
    ]
}

/// The Corral-owned settings file one launch is given.
///
/// Additive by construction: it declares hooks and nothing else, so the user's
/// own configuration — model, permissions, their own hooks — is untouched and
/// still applies.
///
/// Every event runs the same command. The relay never parses the payload and
/// never learns which event it carried, so one command line is the whole
/// integration (ADR 0004 D1).
pub fn settings_document(relay_command: &str) -> String {
    let mut hooks = serde_json::Map::new();
    for (event, _) in INJECTED {
        hooks.insert(
            event.to_owned(),
            json!([{ "hooks": [{ "type": "command", "command": relay_command }] }]),
        );
    }
    // Pretty-printed: this file lands in a user's own state directory, and a
    // person who opens it to find out what Corral did should be able to read
    // it.
    let document = json!({ "hooks": Value::Object(hooks) });
    format!("{document:#}\n")
}

/// Read one Claude hook payload as Corral facts.
///
/// Untrusted input. A payload that is not what this expects degrades to
/// diagnostics: nothing here panics, and nothing invents a fact a payload did
/// not carry (`ARCHITECTURE.md` §5).
pub fn interpret(payload: &str) -> Result<ProviderReport, Uninterpretable> {
    let document: Value = serde_json::from_str(payload).map_err(|_| Uninterpretable::Malformed)?;
    let event = document
        .get("hook_event_name")
        .and_then(Value::as_str)
        .ok_or(Uninterpretable::Malformed)?;
    let fact = INJECTED
        .iter()
        .find(|(name, _)| *name == event)
        .map(|(_, fact)| *fact)
        .ok_or(Uninterpretable::UnknownEvent)?;

    // A known event with no usable id is a fact without an identity, not a
    // malformed payload: the fact is still true of the launch the token names,
    // and refusing the whole event would lose it over a field Corral does not
    // need to know what happened.
    let identity = document
        .get("session_id")
        .and_then(Value::as_str)
        .and_then(|id| ExternalId::new(id).ok());

    Ok(ProviderReport {
        identity,
        fact: Some(fact),
        origin: document
            .get("source")
            .and_then(Value::as_str)
            .map(session_origin),
    })
}

/// Claude Code's `SessionStart.source`, normalized.
///
/// Verified first-party: `startup` on a fresh launch, `resume` on `--resume`,
/// `--continue`, and the in-session picker, and `clear` when a person clears
/// the conversation in place (matrix scenarios 2, 5, 9). `fork` is documented
/// and not exercised here; `compact` is the other in-place replacement.
fn session_origin(source: &str) -> SessionOrigin {
    match source {
        "startup" => SessionOrigin::Startup,
        "resume" => SessionOrigin::Resumed,
        "fork" => SessionOrigin::Forked,
        "clear" | "compact" => SessionOrigin::Replaced,
        _ => SessionOrigin::Unrecognized,
    }
}

impl std::fmt::Display for ArgumentRefused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} is Corral's to pass: it is how Corral watches the session it starts for you",
            self.argument,
        )
    }
}

impl std::error::Error for ArgumentRefused {}

#[cfg(test)]
#[path = "claude_tests.rs"]
mod tests;
