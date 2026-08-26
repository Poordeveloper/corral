//! The session list: the first surface of Corral a person uses daily.
//!
//! See every session, know what needs you, take control. The third of those is
//! Open, and Open is a **takeover**: the list leaves the screen, the
//! attachment this crate already implements runs full-screen, and the list is
//! rebuilt and refreshed the moment the person detaches. Switching between
//! sessions is therefore navigate-then-Open and needs no mechanism of its own
//! (grill Q1).
//!
//! Nothing here derives state. The daemon says what a session is; this decides
//! only what order the sessions appear in — it does not, `session.list` does —
//! and what a row is allowed to say, which is `presentation`'s.

use std::ops::RangeInclusive;
use std::time::Duration;

use corral_client::{ClientActivationPolicy, Connection, RequestError, activate};
use corral_protocol::method::{SessionListItem, SessionListResult, SessionNewParams};

use crate::attach::{Geometry, LocalKeys, OpenFailed, RawMode};
use crate::keys::{self, Key};
use crate::presentation::{SessionPresentation, present};
use crate::screen::{Emphasis, Frame, FullScreen};

/// How often the list asks the daemon what it holds.
///
/// A client refresh policy and not a wire contract. PR4 adds no session
/// subscription, no generic event subscription and no server push, because one
/// live list does not justify defining the semantic event stream (grill Q4).
const POLL: Duration = Duration::from_secs(1);

/// How long one answer may take before the daemon counts as unreadable.
///
/// A client's own patience, not a wire contract. `session.list` is answered
/// from published state on the daemon's reactor thread, so an answer this late
/// means something is wrong rather than busy. The connection it would have
/// arrived on is dropped rather than reused: its next read would be the answer
/// to a question nobody is holding any more.
const ANSWER: Duration = Duration::from_secs(5);

/// The whole key map, which is also the footer: a person can see everything
/// this surface does without being told about it anywhere else.
const FOOTER: &str = "↑/↓ move · enter open · n new · q quit";

/// Run the session list until the person leaves it.
pub async fn run(policy: &ClientActivationPolicy, connection: Connection) -> std::io::Result<()> {
    if Geometry::of(&std::io::stdin()).is_none() {
        return Err(std::io::Error::other(
            "the session list needs a terminal on standard input",
        ));
    }
    // Before anything reads the terminal, and held for the whole surface
    // rather than taken and given back around each takeover. A reader parked
    // on a terminal still in line discipline echoes what the person types and
    // holds it until Enter; and the detach byte is `Ctrl-\`, which a terminal
    // that is not raw turns into SIGQUIT for whoever is in the foreground. The
    // attachment enters raw mode of its own and restores what it found, which
    // is this.
    let _raw = RawMode::enter()?;
    let Some(mut keys) = LocalKeys::start() else {
        return Err(std::io::Error::other(
            "something is already reading this terminal",
        ));
    };

    let mut daemon = Daemon {
        policy,
        connection: Some(connection),
    };
    let mut list = SessionList::default();
    // Taken once and held across every takeover, because the takeover happens
    // on this screen: an Open that gave the terminal back first would put the
    // session's snapshot clear, and everything it drew after it, on top of the
    // person's own screen.
    let mut screen = FullScreen::take()?;
    // Something on screen before the first answer. Every later frame is drawn
    // by an answer or a keystroke, and a person who starts this against a slow
    // daemon would otherwise be looking at nothing at all.
    draw(&mut screen, &mut list)?;

    loop {
        match show(&mut screen, &mut daemon, &mut list, &mut keys).await? {
            Chosen::Quit => return Ok(()),
            Chosen::Open(session) => {
                screen.hand_over()?;
                list.notice = open(&mut daemon, &session, &mut keys).await;
                screen.take_back()?;
            }
            Chosen::New(argv) => {
                screen.hand_over()?;
                list.notice = start(&mut daemon, argv, &mut keys).await;
                screen.take_back()?;
            }
        }
        // Returning here re-enters `show`, whose first poll fires immediately:
        // the list a person comes back to is current, not a second stale
        // (grill Q4).
    }
}

/// What the person chose to do, which is always something that leaves the
/// list.
enum Chosen {
    Quit,
    Open(String),
    New(Vec<String>),
}

/// One row: a session the daemon reported, and what this surface may say.
struct Row {
    session_id: String,
    title: String,
    presentation: SessionPresentation,
}

/// Everything the list holds between redraws.
#[derive(Default)]
struct SessionList {
    rows: Vec<Row>,
    selected: usize,
    /// The first row on screen, kept so the list scrolls rather than jumping
    /// the selection to an edge every time it moves.
    first: usize,
    /// Why the last poll produced no list at all. While this is set the rows
    /// are empty on purpose: an old snapshot shown as current is the one thing
    /// a disconnected list must not do (grill Q4).
    unanswered: Option<Unanswered>,
    /// Sessions the last answer described in a shape this build cannot read.
    ///
    /// Held apart from `notice` rather than written into it: this is a fact
    /// about the list, replaced by every answer, and that is a reply to a
    /// keystroke, which is the person's until they act again. One overwriting
    /// the other every second is how an answer to a person disappears before
    /// they have read it.
    unrenderable: usize,
    /// What the last action produced, shown until the next one.
    notice: Option<String>,
    /// The command being typed, when the person is starting a session.
    typing: Option<String>,
    /// Whether the daemon has answered at all yet, so an empty list before the
    /// first answer does not claim there are no sessions.
    answered: bool,
}

/// One pass of the list, ending in whatever the person chose.
async fn show(
    screen: &mut FullScreen,
    daemon: &mut Daemon<'_>,
    list: &mut SessionList,
    keys: &mut LocalKeys,
) -> std::io::Result<Chosen> {
    let mut poll = tokio::time::interval(POLL);
    // The first tick fires immediately, and a slow answer delays the next
    // question rather than queueing one behind it — polls do not overlap,
    // because there is only ever one in flight on this task (grill Q4).
    poll.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            answered = next_answer(&mut poll, daemon) => {
                list.take(answered);
                draw(screen, list)?;
            }
            typed = keys.next() => {
                // The person's terminal closed. Nothing is left to read and
                // nothing is left to draw on.
                let Some(bytes) = typed else { return Ok(Chosen::Quit) };
                for key in keys::decode(&bytes) {
                    if let Some(chosen) = list.act(key) {
                        return Ok(chosen);
                    }
                }
                draw(screen, list)?;
            }
        }
    }
}

/// The next answer, once the next poll is due.
///
/// One future over both the wait and the question, so exactly one question is
/// ever in flight and a slow answer delays the next rather than queueing one
/// behind it (grill Q4). It is also droppable whole, which is what lets the
/// surface keep answering the keyboard while a daemon answers nothing — and
/// with raw mode holding `Ctrl-C`, a surface that stops reading keys is one a
/// person cannot leave.
async fn next_answer(
    poll: &mut tokio::time::Interval,
    daemon: &mut Daemon<'_>,
) -> Result<Listed, Unanswered> {
    poll.tick().await;
    daemon.sessions().await
}

/// Take over the terminal for one session, and report anything that stopped
/// it.
async fn open(daemon: &mut Daemon<'_>, session_id: &str, keys: &mut LocalKeys) -> Option<String> {
    let outcome = {
        let connection = match daemon.connection().await {
            Ok(connection) => connection,
            Err(reason) => return Some(reason),
        };
        crate::attach::open(connection, session_id, keys).await
    };

    match outcome {
        Ok(()) => None,
        Err(error) => {
            if let OpenFailed::Refused(refused) = &error {
                daemon.forget_if_lost(refused);
            }
            Some(error.to_string())
        }
    }
}

/// Start a session and go straight into it, the way `corral new` does.
async fn start(daemon: &mut Daemon<'_>, argv: Vec<String>, keys: &mut LocalKeys) -> Option<String> {
    let geometry = Geometry::of(&std::io::stdin());
    let cwd = std::env::current_dir()
        .ok()
        .map(|path| path.to_string_lossy().into_owned());
    // Minted per invocation, and the same id is what a retry would carry: it
    // is what stops a lost response from starting a second agent. This surface
    // does not retry, so nothing here re-sends it — the id is the daemon's
    // protection against a client that does (ADR 0002, Q13).
    let command_id = uuid::Uuid::new_v4().as_hyphenated().to_string();

    let started = {
        let connection = match daemon.connection().await {
            Ok(connection) => connection,
            Err(reason) => return Some(reason),
        };
        connection
            .session_new(SessionNewParams {
                command_id,
                argv,
                cwd,
                rows: geometry.map(|geometry| geometry.rows),
                cols: geometry.map(|geometry| geometry.cols),
            })
            .await
    };

    match started {
        Ok(started) => open(daemon, &started.session_id, keys).await,
        Err(error) => {
            daemon.forget_if_lost(&error);
            Some(error.to_string())
        }
    }
}

/// The connection to `corrald`, and the ability to get another one.
///
/// A lost daemon is not a dead list: the list says it cannot be read, keeps
/// asking, and picks up again when one answers — a person who restarted
/// `corrald` should not have to restart this too. Activation is the client
/// library's, exactly as the CLI does it, so this surface can never start a
/// daemon on terms of its own (ADR 0001).
struct Daemon<'a> {
    policy: &'a ClientActivationPolicy,
    connection: Option<Connection>,
}

/// Why there is no list to show.
///
/// Two different claims about the daemon, and the surface must not make the
/// wrong one: a daemon that refused answered, and is on the other end of a
/// connection that is fine; one that could not be read may not be there at all
/// (`AGENTS.md` §Runtime truth).
enum Unanswered {
    Refused(String),
    Unreadable(String),
}

impl Unanswered {
    fn line(&self) -> String {
        match self {
            Self::Refused(detail) => format!("corrald would not list its sessions: {detail}"),
            Self::Unreadable(detail) => format!("corrald could not be read: {detail}"),
        }
    }
}

/// What one answer to `session.list` produced.
struct Listed {
    rows: Vec<Row>,
    /// Sessions a newer daemon described in a shape this build cannot read.
    /// Counted rather than dropped silently or guessed at.
    unrenderable: usize,
}

impl Daemon<'_> {
    async fn connection(&mut self) -> Result<&mut Connection, String> {
        match self.connection {
            Some(ref mut connection) => Ok(connection),
            None => {
                let fresh = activate(self.policy)
                    .await
                    .map_err(|error| error.to_string())?;
                Ok(self.connection.insert(fresh))
            }
        }
    }

    /// The connection, taken out of this daemon for exactly one question.
    ///
    /// Taken rather than borrowed, because waiting for the answer is
    /// interruptible: a question this surface stopped waiting for — a
    /// keystroke arrived, or `ANSWER` ran out — leaves a socket whose next
    /// read is an answer nobody is holding any more. Putting it back only once
    /// one arrived means an abandoned question costs a reconnect and never a
    /// mismatched answer.
    async fn borrow_for_one_question(&mut self) -> Result<Connection, Unanswered> {
        match self.connection.take() {
            Some(connection) => Ok(connection),
            None => activate(self.policy)
                .await
                .map_err(|error| Unanswered::Unreadable(error.to_string())),
        }
    }

    /// What the daemon holds, or why it did not say.
    async fn sessions(&mut self) -> Result<Listed, Unanswered> {
        let mut connection = self.borrow_for_one_question().await?;

        let Ok(answered) = tokio::time::timeout(ANSWER, connection.session_list()).await else {
            return Err(Unanswered::Unreadable(format!(
                "no answer in {} seconds",
                ANSWER.as_secs()
            )));
        };

        match answered {
            Ok(listed) => {
                self.connection = Some(connection);
                Ok(decode(listed))
            }
            // A refusal is a daemon that answered. The connection it answered
            // on is fine and the next question may well be answered too, so
            // this says what it refused rather than claiming it is gone.
            Err(refusal @ RequestError::Refused(_)) => {
                self.connection = Some(connection);
                Err(Unanswered::Refused(refusal.to_string()))
            }
            // A lost daemon and one that broke the protocol are both sockets
            // this surface is done with: nobody is on the first, and the
            // second cannot be trusted to be at a known place in the stream.
            Err(error) => Err(Unanswered::Unreadable(error.to_string())),
        }
    }

    /// Drop a connection the daemon is no longer on the other end of, so the
    /// next question activates instead of asking a socket nobody holds.
    ///
    /// A refusal is not that: the daemon answered, and the connection it
    /// answered on is fine.
    fn forget_if_lost(&mut self, error: &RequestError) {
        if matches!(error, RequestError::DaemonConnectionLost { .. }) {
            self.connection = None;
        }
    }
}

fn decode(listed: SessionListResult) -> Listed {
    let mut rows = Vec::with_capacity(listed.sessions.len());
    let mut unrenderable = 0;

    for session in listed.sessions {
        match serde_json::from_value::<SessionListItem>(session) {
            Ok(item) => rows.push(Row {
                presentation: present(&item),
                session_id: item.session_id,
                title: item.title,
            }),
            Err(_) => unrenderable += 1,
        }
    }

    Listed { rows, unrenderable }
}

impl SessionList {
    /// Accept what the last poll produced.
    fn take(&mut self, answered: Result<Listed, Unanswered>) {
        match answered {
            Ok(listed) => {
                // The cursor follows the session it was on, not the position
                // it was at: the daemon orders by start time, so a new session
                // appearing would otherwise move the selection under a
                // person's fingers.
                let was_on = self
                    .rows
                    .get(self.selected)
                    .map(|row| row.session_id.clone());
                self.rows = listed.rows;
                self.unrenderable = listed.unrenderable;
                self.selected = was_on
                    .and_then(|id| self.rows.iter().position(|row| row.session_id == id))
                    .unwrap_or(0);
                self.unanswered = None;
                self.answered = true;
            }
            Err(unanswered) => {
                // Cleared, not kept: a list that keeps drawing its last
                // snapshot while the daemon is gone is presenting a memory as
                // current truth.
                self.rows.clear();
                self.unrenderable = 0;
                self.selected = 0;
                self.first = 0;
                self.unanswered = Some(unanswered);
            }
        }
    }

    fn act(&mut self, key: Key) -> Option<Chosen> {
        if self.typing.is_some() {
            return self.type_command(key);
        }

        match key {
            // Moving clears whatever the last action said: the notice was
            // about the row the person was on, and this is them leaving it.
            Key::Up => {
                self.selected = self.selected.saturating_sub(1);
                self.notice = None;
                None
            }
            Key::Down => {
                self.selected = (self.selected + 1).min(self.rows.len().saturating_sub(1));
                self.notice = None;
                None
            }
            Key::Enter => self.open_selected(),
            Key::Typed('n') => {
                self.notice = None;
                self.typing = Some(String::new());
                None
            }
            Key::Typed('q') | Key::Interrupt => Some(Chosen::Quit),
            _ => None,
        }
    }

    fn open_selected(&mut self) -> Option<Chosen> {
        // Nothing selected because there is nothing to select: an empty list
        // has no action, and inventing a message for it would be noise on the
        // one screen that already says "No sessions."
        let row = self.rows.get(self.selected)?;

        // Refused before the keystroke rather than after it. The row stays in
        // the list, its execution state is untouched, and the reason has been
        // on screen since the poll that reported it (grill Q7).
        if let Some(refusal) = row.presentation.screen {
            self.notice = Some(format!("{refusal}: this session cannot be opened."));
            return None;
        }

        Some(Chosen::Open(row.session_id.clone()))
    }

    fn type_command(&mut self, key: Key) -> Option<Chosen> {
        let typed = self.typing.as_mut()?;

        match key {
            Key::Typed(character) => {
                typed.push(character);
                None
            }
            Key::Backspace => {
                typed.pop();
                None
            }
            Key::Escape | Key::Interrupt => {
                self.typing = None;
                None
            }
            Key::Enter => {
                // Words, and no shell: the daemon is given a program and its
                // arguments, never a line for something to interpret. The
                // prompt says so, because a person who typed quotes would
                // otherwise watch them arrive as part of an argument.
                let argv: Vec<String> = typed.split_whitespace().map(str::to_owned).collect();
                self.typing = None;
                if argv.is_empty() {
                    return None;
                }
                Some(Chosen::New(argv))
            }
            _ => None,
        }
    }

    /// The rows that fit, with the selected one among them.
    fn window(&mut self, budget: u16) -> std::ops::Range<usize> {
        if self.rows.is_empty() {
            return 0..0;
        }

        self.selected = self.selected.min(self.rows.len() - 1);
        self.first = self.first.min(self.selected);
        // Scrolled by as little as it takes to bring the selection into view,
        // so a list does not jump under someone moving one row at a time.
        while self.first < self.selected
            && self.lines(self.first..=self.selected) > u32::from(budget)
        {
            self.first += 1;
        }

        let mut used = self.lines(self.first..=self.selected);
        let mut end = self.selected + 1;
        while end < self.rows.len()
            && used + u32::from(self.rows[end].height()) <= u32::from(budget)
        {
            used += u32::from(self.rows[end].height());
            end += 1;
        }

        self.first..end
    }

    fn lines(&self, rows: RangeInclusive<usize>) -> u32 {
        self.rows[rows]
            .iter()
            .map(|row| u32::from(row.height()))
            .sum()
    }
}

impl Row {
    /// How many lines this row occupies: the session, its state, and the
    /// capability line when there is one to show.
    fn height(&self) -> u16 {
        2 + u16::from(self.presentation.screen.is_some())
    }
}

fn draw(screen: &mut FullScreen, list: &mut SessionList) -> std::io::Result<()> {
    let Some(geometry) = screen.geometry() else {
        return Err(std::io::Error::other(
            "the terminal stopped reporting its size",
        ));
    };
    let mut frame = Frame::new(geometry);

    frame.line(Emphasis::Plain, &heading(list));
    frame.line(Emphasis::Plain, "");

    // The last line is the footer or the prompt, with a line above it for
    // whatever the surface has to say.
    let tail = 1 + u16::from(list.notice.is_some() || list.typing.is_some());
    // The count of what could not be rendered sits under the rows, so the
    // window has to be told a line is coming after it.
    let counted = u16::from(list.unrenderable > 0);
    let budget = frame.remaining().saturating_sub(tail + counted + 1);

    if let Some(unanswered) = &list.unanswered {
        frame.line(Emphasis::Plain, &unanswered.line());
        frame.line(Emphasis::Secondary, "Asking again every second.");
    } else if list.rows.is_empty() && list.unrenderable == 0 {
        frame.line(
            Emphasis::Plain,
            if list.answered {
                "No sessions."
            } else {
                "Asking corrald…"
            },
        );
    } else {
        for index in list.window(budget) {
            draw_row(
                &mut frame,
                &list.rows[index],
                index == list.selected,
                geometry,
            );
        }
    }

    if list.unrenderable > 0 {
        frame.line(
            Emphasis::Secondary,
            &format!("{} more this build cannot render yet.", list.unrenderable),
        );
    }

    while frame.remaining() > tail {
        frame.line(Emphasis::Plain, "");
    }

    match &list.typing {
        Some(typed) => {
            frame.line(
                Emphasis::Secondary,
                "The program and its arguments. Quoting is not interpreted.",
            );
            // "new session", not "run": Run is an internal noun, and the one
            // domain noun this surface exposes is Session (`PRODUCT.md` §8).
            frame.prompt(&format!("new session: {typed}"));
        }
        None => {
            if let Some(notice) = &list.notice {
                frame.line(Emphasis::Secondary, notice);
            }
            frame.line(Emphasis::Secondary, FOOTER);
        }
    }

    screen.show(frame)
}

fn heading(list: &SessionList) -> String {
    // A count is a claim. There is nothing to count before the daemon has
    // answered, or once it can no longer be read — and a heading saying zero
    // over a body saying the daemon is gone is the surface contradicting
    // itself.
    if list.unanswered.is_some() || !list.answered {
        return "Corral".to_owned();
    }

    // What the daemon reported, which is not what this build could draw: a
    // heading counting only the rows would disagree with the line under them
    // saying there are more.
    match list.rows.len() + list.unrenderable {
        1 => "Corral — 1 session".to_owned(),
        other => format!("Corral — {other} sessions"),
    }
}

fn draw_row(frame: &mut Frame, row: &Row, selected: bool, geometry: Geometry) {
    let marker = if selected { "> " } else { "  " };
    let heading = format!("{marker}{}  {}", short_id(&row.session_id), row.title);
    if selected {
        // Padded so the highlight reads as a row rather than as a word: an
        // inverse run that stops at the title looks like emphasis on the
        // title.
        frame.line(
            Emphasis::Selected,
            &format!("{heading:width$}", width = usize::from(geometry.cols)),
        );
    } else {
        frame.line(Emphasis::Plain, &heading);
    }

    frame.line(
        Emphasis::Secondary,
        &format!("    {}", row.presentation.state_line()),
    );
    if let Some(screen) = row.presentation.screen {
        frame.line(Emphasis::Secondary, &format!("    {screen}"));
    }
}

/// Enough of an id to read, with the whole thing still the identity.
///
/// The same rule the CLI prints under, so the two surfaces name a session the
/// same way and `corral attach` takes what either of them showed.
pub fn short_id(session_id: &str) -> &str {
    session_id
        .split_once('-')
        .map_or(session_id, |(head, _)| head)
}

/// The lines a row shows, without the styling that puts them on a screen.
///
/// Exists so the CLI's own list can be held to saying exactly what this one
/// says about the same session (grill Q2).
pub fn row_text(item: &SessionListItem) -> Vec<String> {
    let presented = present(item);
    let mut lines = vec![
        format!("{}  {}", short_id(&item.session_id), item.title),
        presented.state_line(),
    ];
    lines.extend(presented.screen.map(str::to_owned));
    lines
}

#[cfg(test)]
#[path = "list_tests.rs"]
mod tests;
