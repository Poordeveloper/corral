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

use corral_protocol::method::{SessionListItem, TerminalAccess};

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
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SessionPresentation {
    /// The main state. Never derived from anything but the daemon's own words.
    pub state: MainState,
    /// The runtime fact allowed beside it, when there is one.
    pub runtime: Option<&'static str>,
    /// The line beneath, when Corral cannot serve this session's terminal —
    /// and the whole reason Open is refused before the keystroke rather than
    /// after it.
    pub screen: Option<&'static str>,
}

/// What a surface may say about one listed session.
pub fn present(item: &SessionListItem) -> SessionPresentation {
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

    /// Whether Open is refused before the person presses it.
    ///
    /// The row stays in the list either way, and its execution state is
    /// untouched: what is refused is Corral's ability to show the screen.
    pub fn open_is_refused(&self) -> bool {
        self.screen.is_some()
    }
}

#[cfg(test)]
#[path = "presentation_tests.rs"]
mod tests;
