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
use std::time::{Duration, Instant};

use corral_client::{ClientActivationPolicy, Connection, RequestError, activate};
use corral_protocol::method::{SessionListItem, SessionListResult};

use crate::attach::{Geometry, LocalKeys, OpenFailed, RawMode};
use crate::keys::{Key, Keyboard};
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
    let Some(_raw) = RawMode::enter()? else {
        // Not a condition to carry on through. Everything above says why: a
        // terminal in line discipline echoes over the frame, holds what was
        // typed until Enter, and turns the detach byte into SIGQUIT.
        return Err(std::io::Error::other(
            "the session list needs a terminal it can put in raw mode",
        ));
    };
    let Some(mut keys) = LocalKeys::start() else {
        return Err(std::io::Error::other(
            "something is already reading this terminal",
        ));
    };

    let mut daemon = Daemon {
        policy,
        connection: Some(connection),
        retry: None,
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
                returned(&mut screen, &mut list)?;
            }
            Chosen::New(argv) => {
                screen.hand_over()?;
                list.notice = start(&mut daemon, argv, &mut keys).await;
                returned(&mut screen, &mut list)?;
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
    presentation: SessionPresentation,
    /// What this row says, in order: the session, its state, and the
    /// capability line when there is one.
    ///
    /// Built once per answer rather than per frame, and the one place a row's
    /// text is decided: `draw_row` puts these on a screen and `row_text` hands
    /// them to the CLI, so neither can start saying something the other does
    /// not.
    lines: Vec<String>,
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
    // Kept across the whole pass: a read boundary can fall inside a cursor
    // key, and the half that arrived is not a key until the rest does.
    let mut keyboard = Keyboard::default();

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
                keyboard.add(&bytes);
                while let Some(key) = keyboard.next() {
                    if let Some(chosen) = list.act(key) {
                        // The rest of the burst was typed for whatever this
                        // opens, not for the list that is leaving the screen.
                        keys.put_back(keyboard.unread());
                        return Ok(chosen);
                    }
                }
                draw(screen, list)?;
            }
        }
    }
}

/// Take the terminal back, and say at once if the takeover had something to
/// report.
///
/// The poll below redraws, but a daemon that is slow or gone is exactly when
/// it will not do so soon — and until it does, the person is looking at
/// whatever the takeover left on the screen, with no sign of what just failed.
fn returned(screen: &mut FullScreen, list: &mut SessionList) -> std::io::Result<()> {
    screen.take_back()?;
    if list.notice.is_some() {
        draw(screen, list)?;
    }
    Ok(())
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
                daemon.forget_if_unusable(refused);
            }
            Some(error.to_string())
        }
    }
}

/// Start a session and go straight into it, the way `corral new` does.
async fn start(daemon: &mut Daemon<'_>, argv: Vec<String>, keys: &mut LocalKeys) -> Option<String> {
    let started = {
        let connection = match daemon.connection().await {
            Ok(connection) => connection,
            Err(reason) => return Some(reason),
        };
        crate::launch::start_session(connection, argv).await
    };

    match started {
        Ok(started) => open(daemon, &started.session_id, keys).await,
        Err(error) => {
            daemon.forget_if_unusable(&error);
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
    /// When activation may be attempted again, and how many have failed in a
    /// row.
    ///
    /// Activation may start a daemon. A `corrald` that dies on startup leaves
    /// no owner behind, so a poll that activated every second would start one
    /// every second forever — a retry loop that is indistinguishable, from the
    /// outside, from a fork bomb with a one-second fuse. The poll keeps its
    /// cadence; only starting one backs off.
    retry: Option<Backoff>,
}

/// How long to wait before trying to activate again.
struct Backoff {
    failures: u32,
    until: Instant,
}

impl Backoff {
    /// Doubling, to a ceiling: long enough that a daemon which cannot start is
    /// not started repeatedly, short enough that a person who fixes whatever
    /// stopped it does not wait long to see the list come back.
    const CEILING: Duration = Duration::from_secs(30);

    fn after(failures: u32) -> Self {
        let seconds = 1_u64 << failures.min(5);
        Self {
            failures: failures.saturating_add(1),
            until: Instant::now() + Duration::from_secs(seconds).min(Self::CEILING),
        }
    }

    fn waiting(&self) -> Option<Duration> {
        self.until.checked_duration_since(Instant::now())
    }
}

/// Why there is no list to show.
///
/// Two different claims about the daemon, and the surface must not make the
/// wrong one: a daemon that refused answered, and is on the other end of a
/// connection that is fine; one that could not be read may not be there at all
/// (`AGENTS.md` §Runtime truth).
enum Unanswered {
    /// It answered, and the answer was a refusal.
    Refused(String),
    /// It answered, and the answer was not one this build can read.
    Unreadable(String),
    /// Nothing answered.
    Silent(String),
}

impl Unanswered {
    fn line(&self) -> String {
        match self {
            Self::Refused(detail) => format!("corrald would not list its sessions: {detail}"),
            Self::Unreadable(detail) => {
                format!("corrald answered with something this build cannot read: {detail}")
            }
            Self::Silent(detail) => format!("corrald did not answer: {detail}"),
        }
    }
}

/// What one failed request says about the daemon behind it.
///
/// The claim and the disposal are two decisions, and only one of them is the
/// same for both: a protocol fault and a lost daemon both leave a connection
/// this client cannot ask again, but only one of them means nothing is there
/// (`AGENTS.md` §Runtime truth).
fn about(error: &RequestError) -> Unanswered {
    match error {
        RequestError::Refused(_) => Unanswered::Refused(error.to_string()),
        RequestError::Protocol { .. } => Unanswered::Unreadable(error.to_string()),
        RequestError::DaemonConnectionLost { .. } => Unanswered::Silent(error.to_string()),
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
                let fresh = self.activated().await.map_err(|why| why.line())?;
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
            None => self.activated().await,
        }
    }

    /// A connection to a running daemon, starting one if there is none and the
    /// last attempt is far enough behind.
    ///
    /// The one place this surface activates. Both the poll and the person's
    /// own actions come through here, so a daemon that cannot start is not
    /// started once per second by one and once per keystroke by the other.
    async fn activated(&mut self) -> Result<Connection, Unanswered> {
        if let Some(waiting) = self.retry.as_ref().and_then(Backoff::waiting) {
            return Err(Unanswered::Silent(format!(
                "no corrald is running; trying again in {} seconds",
                waiting.as_secs().max(1)
            )));
        }

        let failures = self.retry.as_ref().map_or(0, |backoff| backoff.failures);
        match activate(self.policy).await {
            Ok(connection) => {
                self.retry = None;
                Ok(connection)
            }
            Err(error) => {
                self.retry = Some(Backoff::after(failures));
                Err(Unanswered::Silent(error.to_string()))
            }
        }
    }

    /// What the daemon holds, or why it did not say.
    async fn sessions(&mut self) -> Result<Listed, Unanswered> {
        let mut connection = self.borrow_for_one_question().await?;

        let Ok(answered) = tokio::time::timeout(ANSWER, connection.session_list()).await else {
            return Err(Unanswered::Silent(format!(
                "nothing within {} seconds",
                ANSWER.as_secs()
            )));
        };

        match answered {
            Ok(listed) => {
                self.connection = Some(connection);
                Ok(decode(listed))
            }
            Err(error) => {
                // Put back only what can be asked again. A refusal came on a
                // connection that is fine; the others left one with nobody on
                // it, or at a place in the stream this client cannot find.
                if matches!(error, RequestError::Refused(_)) {
                    self.connection = Some(connection);
                }
                Err(about(&error))
            }
        }
    }

    /// Drop a connection that cannot be asked another question.
    ///
    /// A daemon that went away has nobody on the other end, and one that broke
    /// the protocol may have left the stream at a place this client cannot
    /// find — asking again on either would answer the wrong question. A
    /// refusal is neither: the daemon answered, on a connection that is fine.
    ///
    /// The same rule `sessions` applies, because it is the same connection.
    fn forget_if_unusable(&mut self, error: &RequestError) {
        if !matches!(error, RequestError::Refused(_)) {
            self.connection = None;
        }
    }
}

fn decode(listed: SessionListResult) -> Listed {
    let mut rows = Vec::with_capacity(listed.sessions.len());
    let mut unrenderable = 0;

    for session in listed.sessions {
        match serde_json::from_value::<SessionListItem>(session) {
            Ok(item) => rows.push(Row::of(&item)),
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
                let was_at = self.selected;
                self.rows = listed.rows;
                self.unrenderable = listed.unrenderable;
                // Gone, so the cursor stays where the person left it rather
                // than going to the top — which under newest-first ordering is
                // whatever started most recently, and one keystroke from being
                // opened.
                self.selected = was_on
                    .and_then(|id| self.rows.iter().position(|row| row.session_id == id))
                    .unwrap_or_else(|| was_at.min(self.rows.len().saturating_sub(1)));
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
        if let Some(refusal) = row.presentation.refuses_open() {
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
    fn of(item: &SessionListItem) -> Self {
        let presentation = present(item);
        let mut lines = vec![
            format!("{}  {}", short_id(&item.session_id), item.title),
            presentation.state_line(),
        ];
        lines.extend(presentation.screen.map(str::to_owned));

        Self {
            session_id: item.session_id.clone(),
            presentation,
            lines,
        }
    }

    /// How many lines this row occupies, which is how many it has.
    fn height(&self) -> u16 {
        u16::try_from(self.lines.len()).unwrap_or(u16::MAX)
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
    // The count of what could not be rendered sits under the rows, so its row
    // is spoken for too, and one blank row keeps the tail off the last of
    // them. Reserved rather than subtracted: `line` then stops at the
    // reservation, so a row taller than what is left cannot take the footer's
    // place — or the prompt's, which is the only line that shows the cursor.
    let counted = u16::from(list.unrenderable > 0);
    frame.reserve(tail + counted + 1);
    let budget = frame.remaining();

    if let Some(unanswered) = &list.unanswered {
        frame.line(Emphasis::Plain, &unanswered.line());
        frame.line(Emphasis::Secondary, "Asking again.");
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

    frame.reserve(tail);
    if list.unrenderable > 0 {
        frame.line(
            Emphasis::Secondary,
            &format!("{} more this build cannot render yet.", list.unrenderable),
        );
    }

    while frame.remaining() > 0 {
        frame.line(Emphasis::Plain, "");
    }

    frame.reserve(0);
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
    let mut lines = row.lines.iter();
    let Some(heading) = lines.next() else { return };

    let marker = if selected { "> " } else { "  " };
    let heading = format!("{marker}{heading}");
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

    for beneath in lines {
        frame.line(Emphasis::Secondary, &format!("    {beneath}"));
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
    Row::of(item).lines
}

#[cfg(test)]
#[path = "list_tests.rs"]
mod tests;
