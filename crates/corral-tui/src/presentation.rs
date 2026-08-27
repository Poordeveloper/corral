//! What a surface is entitled to say about a session, decided once.
//!
//! Both terminal surfaces render from here — the list and `corral list` —
//! because one of them contradicting the other about the same session would
//! be worse than either being wrong alone (grill Q2).
//!
//! Nothing here derives state. Attention is the daemon's to compute and PR8's
//! to carry (`AGENTS.md` §Runtime truth); this projects what the daemon
//! already said, and exists to hold one invariant while it does:
//!
//! > Execution state may establish `Exited`, or secondary runtime truth. It
//! > must never manufacture Working / Needs You / Ready.

use std::time::{Duration, SystemTime};

use corral_protocol::method::{AgentEventKind, SessionListItem, TerminalAccess};

/// The main state, spelled as `PRODUCT.md` §4 spells it.
const UNKNOWN: &str = "Status unknown";
const EXITED: &str = "Exited";

/// The runtime fact that may sit beside the main state.
const RUNNING: &str = "Running";
/// Neutral wording for a runtime Corral cannot currently vouch for. It asserts
/// neither that the process is alive nor that it ended — both would be claims
/// nothing established.
const UNVERIFIED: &str = "Runtime unverified";

/// What a screen nobody can serve is called in front of a person.
///
/// Never the internal word, and never a main status reading `Poisoned`,
/// `Broken` or `Error`: this is neither an agent status nor a claim that a
/// process died (grill Q7).
const NO_SCREEN: &str = "Screen unavailable";

/// The strongest main state Corral may claim for a session today.
///
/// Two of the five (`PRODUCT.md` §4). Working, Needs You and Ready need
/// semantic evidence nothing produces before PR8, and no execution fact
/// stands in for it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MainState {
    Unknown,
    Exited,
}

/// One session as every surface should show it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionPresentation {
    /// The main state. Never derived from anything but the daemon's own words.
    pub state: MainState,
    /// The runtime fact allowed beside it, when there is one.
    pub runtime: Option<&'static str>,
    /// The line beneath, when Corral cannot serve this session's terminal —
    /// and the whole reason Open is refused before the keystroke rather than
    /// after it.
    pub screen: Option<&'static str>,
    /// The latest still-relevant fact the agent reported about itself, in the
    /// past tense with its provenance and its age.
    ///
    /// A report, never a claim about now. It never becomes a main state, an
    /// attention item, a badge, or a notification: those need semantic
    /// evidence nothing produces before PR8, and this is the provider saying
    /// what happened, not Corral saying what is (ADR 0004 D7).
    ///
    /// Supersession is the daemon's: it sends the latest fact, so a newer one
    /// retires the older and an `awaiting_input` is not still here after a
    /// turn started.
    pub agent: Option<String>,
}

/// What a surface may say about one listed session.
pub fn present(item: &SessionListItem) -> SessionPresentation {
    present_at(item, SystemTime::now())
}

/// The same, against a stated instant.
///
/// The age of a reported fact is the one thing here that is not a pure
/// function of what the daemon said, so the clock is a parameter rather than
/// something reached for — otherwise the only way to check the wording would
/// be to wait.
pub fn present_at(item: &SessionListItem, now: SystemTime) -> SessionPresentation {
    let (state, runtime) = match item.execution_state.as_str() {
        // Reliably knowing the runtime ended is enough for Exited, and nothing
        // else about the session's status is claimed alongside it.
        "exited" => (MainState::Exited, None),
        // Running is runtime truth and stays visible; what the agent is doing
        // with it is not something execution state may answer.
        "running" => (MainState::Unknown, Some(RUNNING)),
        // `unknown`, and every spelling this build does not know: the wire
        // contract says an unrecognised value is unknown rather than guessed
        // at, so the two arrive at the same place on purpose.
        _ => (MainState::Unknown, Some(UNVERIFIED)),
    };

    SessionPresentation {
        state,
        runtime,
        screen: match item.terminal_access {
            Some(TerminalAccess::Unavailable) => Some(NO_SCREEN),
            // Available, and unknown. Unknown says nothing: a client that
            // could not read the field still offers Open and reports whatever
            // answer comes back (`AGENTS.md` §Protocol).
            Some(TerminalAccess::Available) | None => None,
        },
        agent: agent_line(item, now),
    }
}

/// What the agent reported, as a person reads it.
///
/// Absent unless every part of the sentence is known: which agent said it,
/// what it said, and how long ago. A fact this build has no word for renders
/// nothing at all — the client states no claim it cannot name
/// (`AGENTS.md` §Protocol).
fn agent_line(item: &SessionListItem, now: SystemTime) -> Option<String> {
    let event = item.agent_event.as_ref()?;
    let provider = item.provider.as_ref()?;
    let reported = reported_phrase(&event.kind)?;
    Some(format!(
        "{} reported {reported} · {} ago",
        product(&provider.name),
        age(event.at_ms, now),
    ))
}

/// Past tense, and only what the provider actually said.
///
/// `turn_started` is deliberately not "working" and `awaiting_input` is
/// deliberately not "needs you": both are main states, both need evidence this
/// phase does not have, and the wording is where that discipline is either
/// kept or quietly lost (`PRODUCT.md` §4).
fn reported_phrase(kind: &AgentEventKind) -> Option<&'static str> {
    match kind {
        AgentEventKind::SessionStarted => Some("starting"),
        AgentEventKind::TurnStarted => Some("starting a turn"),
        AgentEventKind::TurnEnded => Some("finishing a turn"),
        AgentEventKind::AwaitingInput => Some("waiting for input"),
        AgentEventKind::SessionEnded => Some("ending"),
        // A kind a newer daemon named. Rendering the raw spelling would put
        // provider vocabulary in front of a person, and guessing at it would
        // be worse.
        AgentEventKind::Unknown(_) => None,
    }
}

/// The provider as a person writes it.
///
/// The wire name is a lowercase identifier; a sentence starts with a capital.
/// Nothing else about it is changed, because Corral does not maintain a table
/// of product names it might get wrong.
fn product(name: &str) -> String {
    let mut characters = name.chars();
    match characters.next() {
        Some(first) => first.to_uppercase().collect::<String>() + characters.as_str(),
        None => String::new(),
    }
}

/// How long ago a fact was reported, at the coarseness a person reads.
///
/// A clock that puts the report in the future reads as no time at all rather
/// than as a negative age: the two clocks disagree, which says nothing about
/// the fact.
fn age(at_ms: i64, now: SystemTime) -> String {
    let reported = if at_ms < 0 {
        SystemTime::UNIX_EPOCH - Duration::from_millis(at_ms.unsigned_abs())
    } else {
        SystemTime::UNIX_EPOCH + Duration::from_millis(at_ms.unsigned_abs())
    };
    let elapsed = now.duration_since(reported).unwrap_or(Duration::ZERO);
    let seconds = elapsed.as_secs();
    match seconds {
        ..60 => format!("{seconds}s"),
        60..3_600 => format!("{}m", seconds / 60),
        3_600..172_800 => format!("{}h", seconds / 3_600),
        _ => format!("{}d", seconds / 86_400),
    }
}

impl SessionPresentation {
    /// The one-line state text: the runtime fact and the main state, in the
    /// order `PRODUCT.md` §4 fixed — "Running · Status unknown".
    pub fn state_line(&self) -> String {
        match (self.state, self.runtime) {
            // Never "Exited · Status unknown". Once the runtime is reliably
            // over, historical attention is not a current main state, and
            // there is nothing left to be unknown about.
            (MainState::Exited, _) => EXITED.to_owned(),
            (MainState::Unknown, Some(fact)) => format!("{fact} · {UNKNOWN}"),
            (MainState::Unknown, None) => UNKNOWN.to_owned(),
        }
    }

    /// The lines beneath the state, in the order every surface prints them.
    ///
    /// One list rather than two fields each surface arranges: what the CLI and
    /// the list must agree on is the words *and* their order, and two
    /// arrangements of the same facts are two surfaces contradicting each
    /// other about which one matters (grill Q2).
    pub fn beneath(&self) -> Vec<String> {
        let mut lines = Vec::new();
        lines.extend(self.screen.map(str::to_owned));
        lines.extend(self.agent.clone());
        lines
    }

    /// Why Open is refused before the person presses it, when it is.
    ///
    /// The same line the row already shows, because a refusal should repeat
    /// what a person was told rather than invent a second vocabulary for it.
    /// The row stays in the list either way, and its execution state is
    /// untouched: what is refused is Corral's ability to show the screen.
    pub fn refuses_open(&self) -> Option<&'static str> {
        self.screen
    }
}

#[cfg(test)]
#[path = "presentation_tests.rs"]
mod tests;
