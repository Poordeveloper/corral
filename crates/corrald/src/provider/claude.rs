//! Claude Code: the one place this build knows anything about it.
//!
//! Four named boundaries, which are the four things a provider integration
//! turns out to be: composing a launch, composing a resume, reading hook
//! ingress, and validating what a payload claims. The second implementation
//! (`super::codex`) found the same four (grill Q5).
//!
//! Behaviour verified first-party against 2.1.247; the evidence and its limits
//! are `docs/references/2026-08-27-pr5-claude-code-hook-matrix.md`. A version
//! outside that record is not gated — launch proceeds, evidence is
//! best-effort, and unknown event names assert nothing.

use std::ffi::OsString;
use std::path::Path;

use corral_core::{ExternalId, RunId};
use serde_json::{Value, json};
use tracing::warn;

use super::launch::{InjectedSettings, InjectionFailed, RelayInvocation};
use super::{
    AgentFactKind, ArgumentRefused, LaunchIntent, ProviderLaunch, ProviderReport, SessionOrigin,
    Uninterpretable,
};

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

/// The flag that starts the agent with every customization off.
///
/// First-party measured on 2.1.251: with it, a launch carrying Corral's
/// `--settings` runs normally, exits 0, and fires no hook at all — and
/// `disableAllHooks: false` in the injected file does not bring them back,
/// because this is not a settings key. Only admin policy survives it.
const SAFE_MODE_FLAG: &str = "--safe-mode";

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
fn launch_argv(settings: &Path, args: &[String]) -> Vec<OsString> {
    let mut argv = vec![
        OsString::from(SETTINGS_FLAG),
        OsString::from(settings.as_os_str()),
    ];
    argv.extend(args.iter().map(OsString::from));
    argv
}

/// Compose this provider's launch: its Corral-owned settings file, then the
/// argv that names it.
///
/// The file is written before the argv is built and not afterwards, because
/// the argv's whole content is where the file went — and a launch given a path
/// nothing wrote is the session that looks managed and can never report.
pub fn compose_launch(
    intent: &LaunchIntent,
    relay: &RelayInvocation,
    launch_dir: &Path,
    run: RunId,
) -> Result<ProviderLaunch, InjectionFailed> {
    let settings =
        InjectedSettings::write(launch_dir, run, &settings_document(&relay.shell_command()))?;
    let argv = match intent {
        LaunchIntent::Fresh { args } => launch_argv(settings.path(), args),
        LaunchIntent::Continue { external_id } => resume_argv(external_id, settings.path()),
    };
    Ok(ProviderLaunch {
        argv,
        artifact: Some(settings),
    })
}

/// Refuse the provider arguments Corral cannot honour.
///
/// Two, and the same reason twice: each one leaves a launch Corral believes it
/// is watching and is not. A caller repeating `--settings` displaces Corral's
/// own, since the last one is the one loaded; `--safe-mode` starts the agent
/// with hooks off, so the injected file is loaded and ignored. Everything else
/// a person may want to pass to their own agent is theirs, including the
/// separator.
///
/// Version-sensitive by nature: this is a claim about one provider's command
/// line, held against the version the matrix records. A flag a later release
/// adds is a flag this list does not know, which is why a managed launch also
/// has to survive learning nothing — it does, as an identity that never binds
/// rather than as a false one.
pub fn refuse_arguments(args: &[String]) -> Result<(), ArgumentRefused> {
    let equals = format!("{SETTINGS_FLAG}=");
    let competing = |argument: &&String| {
        *argument == SETTINGS_FLAG || argument.starts_with(&equals) || *argument == SAFE_MODE_FLAG
    };
    match args.iter().find(competing) {
        Some(argument) => Err(ArgumentRefused::CompetesWithInjection(argument.clone())),
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
/// The injection goes first here for the same reason it does in `launch_argv`,
/// and the reason is sharper: the word after `--resume` is a **provider**
/// string. `ExternalId` bounds its length and refuses characters that hide or
/// reorder text, and that is all — `--`, `-p` and `--settings` are all valid
/// external ids. A payload naming one of those would, with the injection
/// placed after it, either swallow Corral's flag as prompt text (matrix
/// scenario 10) or take it as its own value. Nothing Corral needs may sit
/// where a provider payload can reach it.
fn resume_argv(external_id: &ExternalId, settings: &Path) -> Vec<OsString> {
    vec![
        OsString::from(SETTINGS_FLAG),
        OsString::from(settings.as_os_str()),
        OsString::from(RESUME_FLAG),
        OsString::from(external_id.as_str()),
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
fn settings_document(relay_command: &str) -> String {
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
    // Stated, not assumed. Settings layers merge by key, so a `disableAllHooks`
    // already set in the user's or the project's own file survives an injection
    // that does not mention it — and then Corral's hooks are configured,
    // accepted, and never run. Measured on 2.1.251: a project
    // `disableAllHooks: true` silences the injected hooks entirely, and this
    // key restores them. Writing `false` here overrides nothing a person will
    // miss: it says only that the file Corral is loading wants its own hooks.
    let document = json!({ "disableAllHooks": false, "hooks": Value::Object(hooks) });
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
    // Refused separately from absent, and said out loud. A payload carrying an
    // id Corral will not hold is a provider doing something this build did not
    // expect; treating it exactly like a payload that carried none would leave
    // a session that never binds and never explains why.
    let identity = match document.get("session_id").and_then(Value::as_str) {
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
        // Only a start has an origin. The field is read off a start and
        // nowhere else, because `SessionOrigin` means "how this provider
        // session came to be" — Claude Code's `SessionEnd` already carries a
        // sibling `reason` with overlapping values, and a later release adding
        // `source` to a mid-session event would otherwise have a contest
        // report an origin describing nothing that started.
        origin: (fact == AgentFactKind::SessionStarted)
            .then(|| {
                document
                    .get("source")
                    .and_then(Value::as_str)
                    .map(session_origin)
            })
            .flatten(),
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

#[cfg(test)]
#[path = "claude_tests.rs"]
mod tests;
