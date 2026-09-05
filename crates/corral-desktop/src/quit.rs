//! The Quit gate: what to say before the Desktop exits, and from which facts.
//!
//! Quitting the Desktop ends watchfulness and nothing else — no runtime is
//! signalled and corrald keeps its own lifetime (`PRODUCT.md` §7). The one
//! thing the gate must get right is the claim it makes about the sessions
//! Corral started: two counts kept apart, one for the runtimes the daemon
//! says are running and one for those it says it cannot verify, so a
//! conservative warning never becomes a false Running claim (grill Q11).
//! Both come from the daemon's words on the row — origin and execution
//! state — never from what the row looks like.

use corral_protocol::method::ORIGIN_MANAGED;

use crate::sessions::{Row, SessionList};

/// Corral-owned runtimes the daemon reports as live or as unverifiable.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Continuing {
    /// `managed` and `running`: they will continue after the Desktop exits.
    pub running: u32,
    /// `managed` and neither `running` nor `exited`: the daemon cannot say.
    pub unverified: u32,
}

/// Count from the rows as the daemon described them.
#[must_use]
pub fn continuing(rows: &[Row]) -> Continuing {
    let mut counts = Continuing::default();
    for row in rows {
        if row.origin.as_deref() != Some(ORIGIN_MANAGED) {
            continue;
        }
        match row.execution_state.as_str() {
            "running" => counts.running += 1,
            "exited" => {}
            // `unknown`, and every spelling this build has no word for: the
            // wire says an unrecognised value is unknown, never guessed at.
            _ => counts.unverified += 1,
        }
    }
    counts
}

/// What Quit does next.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Gate {
    /// Nothing continues that the person should hear about.
    Quit,
    /// One confirmation, per attempt.
    Warn(Warning),
}

/// The confirmation: what continues, and what Corral stops doing. The words
/// describe the capability this build has — watching — and claim no
/// notification until delivery exists (grill Q5).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Warning {
    pub message: String,
    pub detail: &'static str,
}

const STOP_WATCHING_THEM: &str = "Corral will stop watching them for attention.";
const STOP_WATCHING: &str = "Corral will stop watching for attention.";
const UNREACHABLE: &str = "Corral can't reach its service, so it can't tell whether \
                           sessions it started are still running.";

/// The gate for the list as it stands. Only a current generation counts:
/// an answer from before the connection was lost is not the present, and
/// missing data is never zero (grill Q11).
#[must_use]
pub fn gate(list: &SessionList) -> Gate {
    if !list.is_current() {
        return Gate::Warn(Warning {
            message: UNREACHABLE.to_owned(),
            detail: STOP_WATCHING,
        });
    }
    let Continuing {
        running,
        unverified,
    } = continuing(list.rows());
    match (running, unverified) {
        (0, 0) => Gate::Quit,
        (running, 0) => Gate::Warn(Warning {
            message: format!("{running} {} will continue running.", sessions(running)),
            detail: STOP_WATCHING_THEM,
        }),
        (0, unverified) => Gate::Warn(Warning {
            message: format!(
                "Corral couldn't verify whether {unverified} {} it started {} ended.",
                sessions(unverified),
                have(unverified)
            ),
            detail: STOP_WATCHING,
        }),
        (running, unverified) => Gate::Warn(Warning {
            message: format!(
                "{running} {} {} still running. Corral couldn't verify whether \
                 {unverified} other {} it started {} ended.",
                sessions(running),
                are(running),
                sessions(unverified),
                have(unverified)
            ),
            detail: STOP_WATCHING,
        }),
    }
}

fn sessions(count: u32) -> &'static str {
    if count == 1 { "session" } else { "sessions" }
}

fn are(count: u32) -> &'static str {
    if count == 1 { "is" } else { "are" }
}

fn have(count: u32) -> &'static str {
    if count == 1 { "has" } else { "have" }
}

#[cfg(test)]
#[path = "quit_tests.rs"]
mod tests;
