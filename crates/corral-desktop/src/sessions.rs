//! The session list: what the daemon last said, and which row is chosen.
//!
//! Nothing here derives state. The daemon says what a session is; this keeps
//! its last answer, in the words `corral_client::presentation` allows, and
//! decides only which row is selected. Its counts are the daemon's: a header
//! that counted rows would disagree with the badge the daemon serves elsewhere
//! (PR4 grill Q23).

use std::time::{Duration, SystemTime};

use corral_client::presentation::{SessionPresentation, present_at};
use corral_protocol::method::AttentionSummaryResult;

use crate::bridge::{Capabilities, Polled, Unanswered};

/// One row: a session the daemon reported, and what this surface may say.
#[derive(Clone, Debug)]
pub struct Row {
    pub session_id: String,
    pub title: String,
    pub presentation: SessionPresentation,
}

/// Everything the list holds between polls.
#[derive(Debug, Default)]
pub struct SessionList {
    rows: Vec<Row>,
    /// Sessions the last answer described in a shape this build cannot read.
    /// Counted rather than dropped silently or guessed at.
    unreadable: usize,
    summary: Option<AttentionSummaryResult>,
    capabilities: Capabilities,
    /// The selected session, by identity: a poll that reorders rows keeps
    /// the person on the session they chose.
    selected: Option<String>,
    /// Why the last poll produced no answer. While this is set the rows are
    /// the daemon's last answer, shown as history with its age, never as a
    /// current claim.
    unanswered: Option<Unanswered>,
    /// When the rows were last current.
    answered_at: Option<SystemTime>,
}

impl SessionList {
    /// Accept one poll generation, or its absence.
    pub fn take(&mut self, polled: Result<Polled, Unanswered>, now: SystemTime) {
        match polled {
            Ok(polled) => {
                // One instant for the whole answer, so two facts observed at
                // the same moment cannot print two different ages.
                let rows: Vec<Row> = polled
                    .listing
                    .items
                    .iter()
                    .map(|item| Row {
                        session_id: item.session_id.clone(),
                        title: item.title.clone(),
                        presentation: present_at(item, now),
                    })
                    .collect();
                let previous_index = self.selected_index();
                self.rows = rows;
                self.unreadable = polled.listing.unreadable;
                self.summary = Some(polled.summary);
                self.capabilities = polled.capabilities;
                self.unanswered = None;
                self.answered_at = Some(now);
                self.follow_selection(previous_index);
            }
            Err(unanswered) => {
                // Kept, not cleared: the last answer stays as historical
                // context with its age, and nothing about it is offered as
                // current — actions are absent while this is set.
                self.unanswered = Some(unanswered);
            }
        }
    }

    /// Keep the selection on the session it was on; if that session is gone,
    /// on the row that took its place, so the cursor does not vanish.
    fn follow_selection(&mut self, previous_index: Option<usize>) {
        let still_there = self
            .selected
            .as_ref()
            .is_some_and(|id| self.rows.iter().any(|row| &row.session_id == id));
        if still_there {
            return;
        }
        self.selected = previous_index
            .and_then(|index| self.rows.get(index.min(self.rows.len().saturating_sub(1))))
            .map(|row| row.session_id.clone());
    }

    fn selected_index(&self) -> Option<usize> {
        let selected = self.selected.as_ref()?;
        self.rows.iter().position(|row| &row.session_id == selected)
    }

    pub fn rows(&self) -> &[Row] {
        &self.rows
    }

    #[must_use]
    pub fn unreadable(&self) -> usize {
        self.unreadable
    }

    #[must_use]
    pub fn capabilities(&self) -> Capabilities {
        self.capabilities
    }

    /// Whether the rows are the daemon's current answer. Nothing is offered
    /// on a row that is not.
    #[must_use]
    pub fn is_current(&self) -> bool {
        self.unanswered.is_none() && self.answered_at.is_some()
    }

    pub fn selected(&self) -> Option<&Row> {
        let index = self.selected_index()?;
        self.rows.get(index)
    }

    pub fn select(&mut self, session_id: &str) {
        if self.rows.iter().any(|row| row.session_id == session_id) {
            self.selected = Some(session_id.to_owned());
        }
    }

    /// Move the selection by rows, stopping at the ends. With nothing selected,
    /// moving selects the first row.
    pub fn move_selection(&mut self, by: isize) {
        if self.rows.is_empty() {
            return;
        }
        let last = self.rows.len() - 1;
        let index = match self.selected_index() {
            Some(index) => index.saturating_add_signed(by).min(last),
            None => 0,
        };
        self.selected = Some(self.rows[index].session_id.clone());
    }

    /// The heading: a count only once the daemon has answered, the daemon's
    /// own totals beside it.
    #[must_use]
    pub fn heading(&self) -> String {
        if self.answered_at.is_none() {
            return "Corral".to_owned();
        }
        let mut heading = match self.rows.len() + self.unreadable {
            0 => "Corral".to_owned(),
            1 => "Corral — 1 session".to_owned(),
            other => format!("Corral — {other} sessions"),
        };
        if let Some(summary) = &self.summary {
            if summary.needs_you.total > 0 {
                heading.push_str(&format!(" · Needs You {}", summary.needs_you.total));
            }
            if summary.ready.total > 0 {
                heading.push_str(&format!(" · Ready {}", summary.ready.total));
            }
        }
        heading
    }

    /// The line above the rows when they are not the daemon's current answer:
    /// what went wrong, and how old what is shown is.
    #[must_use]
    pub fn banner(&self, now: SystemTime) -> Option<String> {
        let unanswered = self.unanswered.as_ref()?;
        let line = unanswered.line();
        match self.answered_at {
            Some(answered_at) => {
                let age = now.duration_since(answered_at).unwrap_or(Duration::ZERO);
                Some(format!(
                    "{line} Showing what it last said, {} ago.",
                    coarse_age(age)
                ))
            }
            None => Some(line),
        }
    }

    /// What the body says when there are no rows to show.
    #[must_use]
    pub fn empty_line(&self) -> Option<&'static str> {
        if !self.rows.is_empty() || self.unreadable > 0 {
            return None;
        }
        Some(if self.answered_at.is_some() {
            "No sessions."
        } else {
            "Asking corrald…"
        })
    }
}

fn coarse_age(elapsed: Duration) -> String {
    let seconds = elapsed.as_secs();
    match seconds {
        ..60 => format!("{seconds}s"),
        60..3_600 => format!("{}m", seconds / 60),
        3_600..172_800 => format!("{}h", seconds / 3_600),
        _ => format!("{}d", seconds / 86_400),
    }
}

#[cfg(test)]
#[path = "sessions_tests.rs"]
mod tests;
