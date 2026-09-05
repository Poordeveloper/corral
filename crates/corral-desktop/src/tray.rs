//! The tray's projection of daemon truth: a pure value the status item and
//! its menu are rebuilt from, and only when it changes.
//!
//! Nothing here derives state or ranks. The rows are the daemon's rows in
//! the daemon's order and the counts are the daemon's counts; what this file
//! adds is the presentation policy the grill froze (grill Q6, Q7, Q10, Q13
//! in `docs/decisions/2026-09-05-tray-grill.md`): the ambient surface shows
//! Needs You and Ready alone, up to ten rows each; the badge is the
//! unacknowledged total of both classes; an unacknowledged row carries the
//! marker, never the acknowledged one; and a row's age is coarse enough that
//! a second of clock never changes the value, so the 1 s poll does not
//! become a per-second rebuild of native menu objects.

use std::time::{Duration, SystemTime};

use corral_client::presentation::MainState;
use futures::channel::mpsc::UnboundedReceiver;

use crate::sessions::{ASKING, SessionList};

#[cfg(target_os = "macos")]
#[path = "tray_macos.rs"]
pub mod macos;

/// Rows shown per group before the rest is counted (grill Q6).
pub const ROWS_PER_GROUP: usize = 10;

/// The unread marker: this attention item is still unacknowledged. It never
/// means resolved, which is why the acknowledged row is the plain one
/// (grill Q13).
const UNACKNOWLEDGED: &str = "• ";

/// What the status item and its menu show. Two projections compare equal
/// exactly when the native objects built from them would read the same.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TrayProjection {
    /// The daemon has not answered, or its last answer is no longer current:
    /// no count on the item, and the menu says why. A stale count is never
    /// shown as current (`AGENTS.md` §Runtime truth).
    Unreachable {
        line: String,
    },
    Current(Current),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Current {
    pub badge: Badge,
    pub needs_you: Group,
    pub ready: Group,
}

/// Unacknowledged attention items across both classes: the daemon's
/// `needs_you.unacknowledged + ready.unacknowledged`, never a count of rows
/// (grill Q7).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Badge(pub u32);

impl Badge {
    /// The status-item title: nothing at zero, exact to 99, then `99+` — a
    /// bound on menu-bar width, not on the daemon's number.
    #[must_use]
    pub fn text(self) -> Option<String> {
        match self.0 {
            0 => None,
            1..=99 => Some(self.0.to_string()),
            _ => Some("99+".to_owned()),
        }
    }
}

/// One attention class: the daemon's total for the header, the rows this
/// build could read in the daemon's order, and how many rows were left out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Group {
    pub label: &'static str,
    pub total: u32,
    pub rows: Vec<TrayRow>,
    pub overflow: usize,
}

impl Group {
    fn of(label: &'static str, total: u32, mut rows: Vec<TrayRow>) -> Self {
        let overflow = rows.len().saturating_sub(ROWS_PER_GROUP);
        rows.truncate(ROWS_PER_GROUP);
        Self {
            label,
            total,
            rows,
            overflow,
        }
    }

    /// "… k more in Corral": the route to what the menu does not list.
    #[must_use]
    pub fn overflow_line(&self) -> Option<String> {
        match self.overflow {
            0 => None,
            k => Some(format!("… {k} more in Corral")),
        }
    }
}

/// One session in a group. The identity is what a click carries; the rest is
/// what the row reads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrayRow {
    pub session_id: String,
    /// The current item while it is unacknowledged: what an acknowledgement
    /// would name. `None` is an acknowledged row.
    pub unacknowledged_item: Option<String>,
    pub title: String,
    pub state: MainState,
    /// How long the session has been in this state, bucketed.
    pub age: Option<String>,
}

impl TrayRow {
    #[must_use]
    pub fn acknowledged(&self) -> bool {
        self.unacknowledged_item.is_none()
    }

    /// The menu row: the marker on an unacknowledged row, the title, the
    /// state in the words every surface uses, the age.
    #[must_use]
    pub fn text(&self) -> String {
        let marker = if self.acknowledged() {
            ""
        } else {
            UNACKNOWLEDGED
        };
        let state = match self.state {
            MainState::NeedsYou => "Needs You",
            MainState::Ready => "Ready",
            // A row in neither class is not built (see `of`); should one be,
            // it reads its state rather than a wrong one.
            MainState::Working => "Working",
            MainState::Unknown => "Status unknown",
            MainState::Exited => "Exited",
        };
        match &self.age {
            Some(age) => format!("{marker}{} · {state} · {age}", self.title),
            None => format!("{marker}{} · {state}", self.title),
        }
    }
}

impl TrayProjection {
    /// Project the list as it stands at `now`.
    #[must_use]
    pub fn of(list: &SessionList, now: SystemTime) -> Self {
        let unreachable = |line: String| Self::Unreachable { line };
        if !list.is_current() {
            return unreachable(
                list.unanswered()
                    .map_or_else(|| ASKING.to_owned(), |unanswered| unanswered.line()),
            );
        }
        let Some(summary) = list.summary() else {
            return unreachable(ASKING.to_owned());
        };
        let mut needs_you = Vec::new();
        let mut ready = Vec::new();
        for row in list.rows() {
            let group = match row.presentation.state {
                MainState::NeedsYou => &mut needs_you,
                MainState::Ready => &mut ready,
                MainState::Working | MainState::Unknown | MainState::Exited => continue,
            };
            group.push(TrayRow {
                session_id: row.session_id.clone(),
                unacknowledged_item: row.presentation.acknowledgeable().map(str::to_owned),
                title: row.title.clone(),
                state: row.presentation.state,
                age: row
                    .attention_since_unix_ms
                    .map(|since| age_bucket(elapsed_since(since, now))),
            });
        }
        Self::Current(Current {
            badge: Badge(summary.needs_you.unacknowledged + summary.ready.unacknowledged),
            needs_you: Group::of("Needs You", summary.needs_you.total, needs_you),
            ready: Group::of("Ready", summary.ready.total, ready),
        })
    }

    /// The disabled header line.
    #[must_use]
    pub fn header(&self) -> String {
        match self {
            Self::Unreachable { line } => line.clone(),
            Self::Current(current) => format!(
                "Needs You {} · Ready {}",
                current.needs_you.total, current.ready.total
            ),
        }
    }

    /// The status-item title beside the icon.
    #[must_use]
    pub fn badge_text(&self) -> Option<String> {
        match self {
            Self::Unreachable { .. } => None,
            Self::Current(current) => current.badge.text(),
        }
    }

    /// The menu, top to bottom: the header, the groups that have rows, then
    /// the ways into Corral. Decided here, as words, so the native menu is
    /// built mechanically and what it says is under test. While the daemon
    /// is unreachable nothing that would ask it for a runtime is offered,
    /// as in the window; Open Corral is the route to the reason.
    #[must_use]
    pub fn menu(&self) -> Vec<MenuLine> {
        let mut lines = vec![MenuLine::Note(self.header())];
        let mut current = false;
        if let Self::Current(projection) = self {
            current = true;
            for group in [&projection.needs_you, &projection.ready] {
                if group.rows.is_empty() {
                    continue;
                }
                lines.push(MenuLine::Separator);
                lines.push(MenuLine::Note(group.label.to_owned()));
                for row in &group.rows {
                    lines.push(MenuLine::Item {
                        action: TrayAction::OpenSession(row.session_id.clone()),
                        text: row.text(),
                    });
                }
                if let Some(text) = group.overflow_line() {
                    lines.push(MenuLine::Item {
                        action: TrayAction::More,
                        text,
                    });
                }
            }
        }
        lines.push(MenuLine::Separator);
        lines.push(MenuLine::Item {
            action: TrayAction::OpenCorral,
            text: "Open Corral".to_owned(),
        });
        if current {
            lines.push(MenuLine::Item {
                action: TrayAction::NewSession,
                text: "New Session…".to_owned(),
            });
        }
        lines.push(MenuLine::Separator);
        lines.push(MenuLine::Item {
            action: TrayAction::Quit,
            text: "Quit Corral".to_owned(),
        });
        lines
    }
}

/// One line of the menu. A `Note` is words that do nothing — the header, a
/// group's label; an `Item` carries its action in the id the platform hands
/// back on a click.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MenuLine {
    Note(String),
    Separator,
    Item { action: TrayAction, text: String },
}

/// The native status item, behind the one thing the Watch asks of it. The
/// Watch decides when — only when the projection changed (grill Q10) — and
/// the platform decides how; a test item merely remembers what it was shown.
pub trait StatusItem {
    /// Show this projection as one generation: menu and badge together,
    /// never one and then the other.
    fn show(&mut self, projection: &TrayProjection) -> Result<(), String>;
}

/// The menu ids the platform's handler forwarded, in click order and as it
/// spelled them. Read on gpui's foreground by `TrayAction::from_menu_id`:
/// the handler itself touches nothing of gpui (grill Q3).
pub type Clicks = UnboundedReceiver<String>;

fn elapsed_since(unix_ms: i64, now: SystemTime) -> Duration {
    let at = if unix_ms < 0 {
        SystemTime::UNIX_EPOCH - Duration::from_millis(unix_ms.unsigned_abs())
    } else {
        SystemTime::UNIX_EPOCH + Duration::from_millis(unix_ms.unsigned_abs())
    };
    now.duration_since(at).unwrap_or(Duration::ZERO)
}

/// Whole minutes to the hour, whole hours to two days, then days. Never
/// seconds: the value must survive a poll (grill Q10).
#[must_use]
pub fn age_bucket(elapsed: Duration) -> String {
    let seconds = elapsed.as_secs();
    match seconds {
        ..60 => "<1m".to_owned(),
        60..3_600 => format!("{}m", seconds / 60),
        3_600..172_800 => format!("{}h", seconds / 3_600),
        _ => format!("{}d", seconds / 86_400),
    }
}

/// What a menu item does, carried by the item's own id so a click resolves
/// an identity and never a position (grill Q10).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TrayAction {
    OpenCorral,
    NewSession,
    Quit,
    /// The overflow line: the Desktop, where the rest is.
    More,
    OpenSession(String),
}

const OPEN: &str = "open";
const NEW: &str = "new";
const QUIT: &str = "quit";
const MORE: &str = "more";
const SESSION: &str = "session:";

impl TrayAction {
    #[must_use]
    pub fn menu_id(&self) -> String {
        match self {
            Self::OpenCorral => OPEN.to_owned(),
            Self::NewSession => NEW.to_owned(),
            Self::Quit => QUIT.to_owned(),
            Self::More => MORE.to_owned(),
            Self::OpenSession(session_id) => format!("{SESSION}{session_id}"),
        }
    }

    /// `None` for an id that names no action: the header, a group label, or
    /// a word a newer build put there.
    #[must_use]
    pub fn from_menu_id(id: &str) -> Option<Self> {
        match id {
            OPEN => Some(Self::OpenCorral),
            NEW => Some(Self::NewSession),
            QUIT => Some(Self::Quit),
            MORE => Some(Self::More),
            other => other
                .strip_prefix(SESSION)
                .filter(|session_id| !session_id.is_empty())
                .map(|session_id| Self::OpenSession(session_id.to_owned())),
        }
    }
}

#[cfg(test)]
#[path = "tray_tests.rs"]
mod tests;
