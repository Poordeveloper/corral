//! One answer to `session.list`, read the way every surface reads it.
//!
//! A newer daemon may describe a session in a shape this build has no words
//! for. Such a row is counted rather than dropped or guessed at, so a person
//! is told there is more than they can see and the sessions that did decode
//! keep the daemon's order (`AGENTS.md` §Protocol). Nothing here derives
//! state: what a row may *say* is `presentation`'s.

use corral_protocol::method::{SessionListItem, SessionListResult};

/// What one answer held, as far as this build can read it.
#[derive(Clone, Debug, Default)]
pub struct Listing {
    /// The sessions this build could read, in the daemon's order.
    pub items: Vec<SessionListItem>,
    /// Sessions the daemon described in a shape this build cannot read.
    pub unreadable: usize,
}

impl Listing {
    /// Read the daemon's answer. A field this build does not know is skipped
    /// inside a row; a row this build cannot read at all is counted.
    #[must_use]
    pub fn of(listed: SessionListResult) -> Self {
        let mut items = Vec::with_capacity(listed.sessions.len());
        let mut unreadable = 0;
        for session in listed.sessions {
            match serde_json::from_value::<SessionListItem>(session) {
                Ok(item) => items.push(item),
                Err(_) => unreadable += 1,
            }
        }
        Self { items, unreadable }
    }
}

/// Enough of an id to read, with the whole thing still the identity.
///
/// The same rule on every surface, so they name a session the same way and
/// `corral attach` takes what any of them showed.
#[must_use]
pub fn short_id(session_id: &str) -> &str {
    session_id
        .split_once('-')
        .map_or(session_id, |(head, _)| head)
}

#[cfg(test)]
#[path = "sessions_tests.rs"]
mod tests;
