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
use std::time::{Duration, SystemTime};

use corral_client::{ClientActivationPolicy, Connection};
use corral_protocol::method::{SessionListItem, SessionListResult};

use crate::ANSWER;
use crate::attach::{Geometry, LocalKeys, OpenFailed, RawMode};
use crate::daemon::{Daemon, Unanswered};
use crate::keys::{Key, Keyboard};
use crate::launch::requested;
use crate::presentation::{SessionPresentation, present_at};
use crate::screen::{Emphasis, Frame, FullScreen};

/// How often the list asks the daemon what it holds.
///
/// A client refresh policy and not a wire contract. PR4 adds no session
/// subscription, no generic event subscription and no server push, because one
/// live list does not justify defining the semantic event stream (grill Q4).
const POLL: Duration = Duration::from_secs(1);

/// The whole key map, which is also the footer: a person can see everything
/// this surface does without being told about it anywhere else.
const FOOTER: &str = "↑/↓ move · enter open · n new · c continue · q quit";

/// How long an Escape waits to find out whether it was a key or the start of
/// a sequence.
///
/// Long enough that the rest of a cursor key split across two reads has
/// arrived, short enough that pressing Escape feels like pressing Escape
/// (`Keyboard::undecided`).
const ESCAPE_GRACE: Duration = Duration::from_millis(30);

/// Run the session list until the person leaves it.
pub async fn run(policy: &ClientActivationPolicy, connection: Connection) -> std::io::Result<()> {
    // Before anything reads the terminal, and held for the whole surface
    // rather than taken and given back around each takeover. A reader parked
    // on a terminal still in line discipline echoes what the person types and
    // holds it until Enter; and the detach byte is `Ctrl-\`, which a terminal
    // that is not raw turns into SIGQUIT for whoever is in the foreground. The
    // attachment enters raw mode of its own and restores what it found, which
    // is this.
    let Some(_raw) = RawMode::enter()? else {
        return Err(std::io::Error::other(
            "the session list needs a terminal on standard input",
        ));
    };
    // The other end of the same question. Raw mode is about the terminal keys
    // come from; every frame goes to standard output, and `corral tui >file`
    // has a terminal on one and a file on the other — which would put this
    // terminal in raw mode, consume the person's keystrokes, and write the
    // list into the file where nobody can see it. Refused before the screen is
    // taken, so nothing has to be given back.
    if !std::io::IsTerminal::is_terminal(&std::io::stdout()) {
        return Err(std::io::Error::other(
            "the session list needs a terminal on standard output",
        ));
    }
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
            Chosen::New(requested) => {
                screen.hand_over()?;
                list.notice = start(&mut daemon, requested, &mut keys).await;
                returned(&mut screen, &mut list)?;
            }
            Chosen::Continue(session) => {
                screen.hand_over()?;
                list.notice = continue_into(&mut daemon, &session, &mut keys).await;
                returned(&mut screen, &mut list)?;
            }
            // Nothing leaves the list for this one: the daemon answers, the
            // notice says what it said, and the next poll shows the badge.
            Chosen::Acknowledge { session, item } => {
                list.notice = Some(match daemon.acknowledge(&session, &item).await {
                    Ok(()) => "Acknowledged.".to_owned(),
                    Err(unanswered) => unanswered.line(),
                });
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
    New(crate::launch::Requested),
    /// Continue a Session as a new Run, and go straight into it.
    Continue(String),
    /// Acknowledge the selected row's current item, by the id this surface
    /// saw — never "whatever is current" (grill Q18).
    Acknowledge {
        session: String,
        item: String,
    },
}

/// One row: a session the daemon reported, and what this surface may say.
struct Row {
    session_id: String,
    presentation: SessionPresentation,
    /// What this row says, in order: the session, its state, and the
    /// capability line when there is one.
    ///
    /// This surface's layout of it, built once per answer rather than per
    /// frame. The CLI lays the same session out in columns instead — what the
    /// two must agree on is `presentation`, which is where every word of both
    /// comes from (grill Q2).
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
    /// The daemon's counts, when its last answer carried them. Never
    /// computed here: a header that counted rows would disagree with the
    /// badge the daemon serves elsewhere (grill Q23).
    summary: Option<corral_protocol::method::AttentionSummaryResult>,
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
    // Bytes the takeover handed back were typed before this pass began, and
    // are ready the moment the loop starts. Acted on first, so the pass begins
    // showing what they did rather than what the list looked like before them.
    if let Some(waiting) = keys.pending() {
        match arriving(Some(waiting), &mut keyboard, keys, list) {
            Typed::Chose(chosen) => return Ok(chosen),
            Typed::Closed => return Ok(Chosen::Quit),
            Typed::Handled => draw(screen, list)?,
        }
    }

    let mut poll = tokio::time::interval(POLL);
    // The first tick fires immediately, and a slow answer delays the next
    // question rather than queueing one behind it — polls do not overlap,
    // because there is only ever one in flight on this task (grill Q4).
    poll.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    loop {
        // Waiting for the next poll to come due, which is most of the time.
        loop {
            tokio::select! {
                _ = poll.tick() => break,
                typed = keys.next() => match arriving(typed, &mut keyboard, keys, list) {
                    Typed::Chose(chosen) => return Ok(chosen),
                    Typed::Closed => return Ok(Chosen::Quit),
                    Typed::Handled => draw(screen, list)?,
                },
                () = tokio::time::sleep(ESCAPE_GRACE), if keyboard.undecided() => {
                    settle(&mut keyboard, list);
                    draw(screen, list)?;
                }
            }
        }

        // A question, which a keystroke does not cancel. Abandoning one
        // leaves the socket where its next read is the answer to a question
        // nobody is holding, and the connection has to be thrown away with it
        // — so the keys are answered while it is out, and it is waited for
        // even by someone on their way to a session. A macro rather than a
        // helper because it returns from `show` on the person's behalf.
        macro_rules! answered_while_typing {
            ($question:expr) => {{
                let mut question = std::pin::pin!($question);
                loop {
                    tokio::select! {
                        answered = &mut question => break answered,
                        typed = keys.next() => match arriving(typed, &mut keyboard, keys, list) {
                            Typed::Chose(chosen) => {
                                // Waited out only when what comes next needs
                                // this connection. Somebody leaving the surface
                                // does not, and making them wait for an answer
                                // nobody will read is the opposite of the point.
                                if !matches!(chosen, Chosen::Quit) {
                                    let _ = question.await;
                                }
                                return Ok(chosen);
                            }
                            Typed::Closed => return Ok(Chosen::Quit),
                            Typed::Handled => draw(screen, list)?,
                        },
                        () = tokio::time::sleep(ESCAPE_GRACE), if keyboard.undecided() => {
                            settle(&mut keyboard, list);
                            draw(screen, list)?;
                        }
                    }
                }
            }};
        }

        let answered = answered_while_typing!(daemon.sessions());
        list.take(answered.map(decode));
        draw(screen, list)?;
        // The counts, asked only of a daemon that just answered: one that did
        // not will not answer this either, and a person must not wait out a
        // second silence to be told about the first. Its own failure costs the
        // counts and nothing else — the rows are already current.
        if list.unanswered.is_none() {
            let summary = answered_while_typing!(daemon.summary()).ok();
            list.take_summary(summary);
            draw(screen, list)?;
        }
    }
}

/// What one burst of typing did.
enum Typed {
    /// Nothing that leaves the list.
    Handled,
    /// The person chose something, and whatever they typed after it has been
    /// handed back for what they chose.
    Chose(Chosen),
    /// Their terminal closed. Nothing is left to read and nothing is left to
    /// draw on.
    Closed,
}

fn arriving(
    typed: Option<Vec<u8>>,
    keyboard: &mut Keyboard,
    keys: &mut LocalKeys,
    list: &mut SessionList,
) -> Typed {
    let Some(bytes) = typed else {
        return Typed::Closed;
    };

    keyboard.add(&bytes);
    while let Some(key) = keyboard.next() {
        if let Some(chosen) = list.act(key) {
            // The rest of the burst was typed for whatever this opens, not for
            // the list that is leaving the screen.
            keys.put_back(keyboard.unread());
            return Typed::Chose(chosen);
        }
    }

    Typed::Handled
}

/// Nothing followed, so what was held is as much as it will be.
fn settle(keyboard: &mut Keyboard, list: &mut SessionList) {
    if let Some(key) = keyboard.settle() {
        // Escape cancels a prompt and Unknown does nothing: neither is a key
        // that leaves the list, which is what makes settling a redraw rather
        // than a choice.
        let leaving = list.act(key);
        debug_assert!(leaving.is_none(), "{key:?} left the list from a settle");
    }
}

/// Take the terminal back, and draw at once.
///
/// The poll below redraws, but a daemon that is slow or gone is exactly when
/// it will not do so soon — and until it does, the person is looking at
/// whatever the takeover left on the screen, with no list and no sign of
/// anything that just failed. What is drawn is the last answer, which is what
/// this list shows between any two ticks; the tick that follows fires at once.
fn returned(screen: &mut FullScreen, list: &mut SessionList) -> std::io::Result<()> {
    screen.take_back()?;
    draw(screen, list)
}

/// Take over the terminal for one session, and report anything that stopped
/// it.
async fn open(daemon: &mut Daemon<'_>, session_id: &str, keys: &mut LocalKeys) -> Option<String> {
    let outcome = {
        let connection = match daemon.connection() {
            Ok(connection) => connection,
            Err(reason) => return Some(reason),
        };
        crate::attach::open(connection, session_id, keys).await
    };

    match outcome {
        Ok(()) => None,
        Err(error) => {
            // Only the grant was asked on this connection. The channel is a
            // second socket to the same rendezvous, so its failures say
            // nothing about whether this one can be asked again.
            match &error {
                OpenFailed::Refused(refused) => daemon.forget_if_unusable(refused),
                // Nobody is holding the answer that is still coming.
                OpenFailed::GrantUnanswered => daemon.forget(),
                OpenFailed::Unopened(_)
                | OpenFailed::ChannelUnanswered
                | OpenFailed::Channel(_) => {}
            }
            Some(error.to_string())
        }
    }
}

/// Start a session and go straight into it, the way `corral new` does.
async fn start(
    daemon: &mut Daemon<'_>,
    requested: crate::launch::Requested,
    keys: &mut LocalKeys,
) -> Option<String> {
    let started = {
        let connection = match daemon.connection() {
            Ok(connection) => connection,
            Err(reason) => return Some(reason),
        };
        // Bounded here rather than in `launch`, because it is this surface
        // that has handed the terminal over. The CLI waits as long as starting
        // a session takes; here a person is looking at a screen nobody is
        // drawing. A session the daemon goes on to create is not lost by
        // this — it arrives in the next answer, which is what a list is for.
        match tokio::time::timeout(ANSWER, crate::launch::start_session(connection, requested))
            .await
        {
            Ok(started) => started,
            Err(_) => {
                daemon.forget();
                return Some(format!(
                    "corrald did not answer within {} seconds",
                    ANSWER.as_secs()
                ));
            }
        }
    };

    match started {
        Ok(started) => open(daemon, &started.session_id, keys).await,
        Err(error) => {
            daemon.forget_if_unusable(&error);
            Some(error.to_string())
        }
    }
}

/// Continue a session and go straight into it, the way `new` does.
///
/// Every precondition is the daemon's: whether the identity is one Corral
/// still stands behind, whether the previous run's exit is established, and
/// where it ran. A surface that pre-judged any of them would be a second owner
/// of a rule that fails closed, and its answer would be the one a person sees.
async fn continue_into(
    daemon: &mut Daemon<'_>,
    session: &str,
    keys: &mut LocalKeys,
) -> Option<String> {
    let continued = {
        let connection = match daemon.connection() {
            Ok(connection) => connection,
            Err(reason) => return Some(reason),
        };
        match tokio::time::timeout(
            ANSWER,
            crate::launch::continue_session(connection, session, crate::launch::Shown::NotYet),
        )
        .await
        {
            Ok(continued) => continued,
            Err(_) => {
                daemon.forget();
                return Some(format!(
                    "corrald did not answer within {} seconds",
                    ANSWER.as_secs()
                ));
            }
        }
    };

    match continued {
        Ok(crate::launch::Continued::Started { started, .. }) => {
            open(daemon, &started.session_id, keys).await
        }
        // The list has no prompt of its own yet; the words are the daemon's,
        // and the answer is given where one can be typed.
        Ok(crate::launch::Continued::NeedsDisclosure { text, .. }) => Some(format!(
            "{text} To continue anyway: corral continue --yes {session}"
        )),
        Err(error) => {
            daemon.forget_if_unusable(&error);
            Some(error.to_string())
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

fn decode(listed: SessionListResult) -> Listed {
    let mut rows = Vec::with_capacity(listed.sessions.len());
    let mut unrenderable = 0;

    // One instant for the whole answer. A row that read the clock for itself
    // would let two facts observed at the same moment print two different
    // ages, which is a listing disagreeing with itself — and the command line
    // renders the same answer from the same projection (grill Q2).
    let now = SystemTime::now();
    for session in listed.sessions {
        match serde_json::from_value::<SessionListItem>(session) {
            Ok(item) => rows.push(Row::of(&item, now)),
            Err(_) => unrenderable += 1,
        }
    }

    Listed { rows, unrenderable }
}

impl SessionList {
    /// Accept the daemon's counts, or their absence.
    fn take_summary(&mut self, summary: Option<corral_protocol::method::AttentionSummaryResult>) {
        self.summary = summary;
    }

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
            Key::Typed('c') => self.continue_selected(),
            Key::Typed('a') => self.acknowledge_selected(),
            Key::Typed('n') => {
                self.notice = None;
                self.typing = Some(String::new());
                None
            }
            Key::Typed('q') | Key::Interrupt => Some(Chosen::Quit),
            // Clipboard contents are not commands here. A paste that reached
            // this arm carries `q`, `n`, or a newline as ordinary characters,
            // and there is nothing on this screen for them to mean: the list
            // has no text field of its own.
            Key::Pasted(_) => None,
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

    /// Ask for the selected session to run again.
    ///
    /// One refusal is answered here, for the same reason Open's is: choosing
    /// hands the terminal over and takes it back, and doing that for an answer
    /// the row is already displaying is a teardown and a rebuild a person
    /// watches for nothing. Every other reason rests on facts only the daemon
    /// holds, and it states them in words this surface shows unchanged.
    fn continue_selected(&mut self) -> Option<Chosen> {
        let row = self.rows.get(self.selected)?;

        if let Some(refusal) = row.presentation.refuses_continue() {
            self.notice = Some(format!("{refusal}: this session cannot be continued."));
            return None;
        }

        self.notice = None;
        Some(Chosen::Continue(row.session_id.clone()))
    }

    /// Acknowledge the selected row's current item.
    ///
    /// Only an unacknowledged current item is something to acknowledge; a row
    /// without one is told so here rather than sent to the daemon to be told
    /// the same thing.
    fn acknowledge_selected(&mut self) -> Option<Chosen> {
        let row = self.rows.get(self.selected)?;
        let Some(item) = row.presentation.acknowledgeable() else {
            self.notice = Some("Nothing to acknowledge.".to_owned());
            return None;
        };
        self.notice = None;
        Some(Chosen::Acknowledge {
            session: row.session_id.clone(),
            item: item.to_owned(),
        })
    }

    fn type_command(&mut self, key: Key) -> Option<Chosen> {
        let typed = self.typing.as_mut()?;

        match key {
            Key::Typed(character) => {
                typed.push(character);
                None
            }
            // Pasted text is inserted and never submitted: a clipboard entry
            // ending in a newline is the ordinary shape, and running it the
            // instant it arrives would start a session the person never
            // confirmed. A newline inside a command line is what it is between
            // two words — a separator — and the line is split on whitespace
            // below anyway.
            Key::Pasted(character) => {
                typed.push(if character.is_control() {
                    ' '
                } else {
                    character
                });
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
                let words: Vec<String> = typed.split_whitespace().map(str::to_owned).collect();
                self.typing = None;
                requested(&words).map(Chosen::New)
            }
            _ => None,
        }
    }

    /// The rows that fit, with the selected one among them.
    fn window(&mut self, budget: u16) -> std::ops::Range<usize> {
        if self.rows.is_empty() {
            return 0..0;
        }

        let budget = u32::from(budget);
        self.selected = self.selected.min(self.rows.len() - 1);
        self.first = self.first.min(self.selected);
        // Scrolled by as little as it takes to bring the selection into view,
        // so a list does not jump under someone moving one row at a time. The
        // total is carried rather than re-summed, because this runs on every
        // frame and every keystroke.
        let mut used = self.lines(self.first..=self.selected);
        while self.first < self.selected && used > budget {
            used -= u32::from(self.rows[self.first].height());
            self.first += 1;
        }

        let mut end = self.selected + 1;
        while end < self.rows.len() && used + u32::from(self.rows[end].height()) <= budget {
            used += u32::from(self.rows[end].height());
            end += 1;
        }

        // Room left over above. A window that only ever moved down would leave
        // the rows before it unreachable on a screen with space for them —
        // after the terminal grew, or after the rows below the cursor went
        // away.
        while self.first > 0 && used + u32::from(self.rows[self.first - 1].height()) <= budget {
            self.first -= 1;
            used += u32::from(self.rows[self.first].height());
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
    fn of(item: &SessionListItem, now: SystemTime) -> Self {
        let presentation = present_at(item, now);
        let mut lines = vec![
            format!("{}  {}", short_id(&item.session_id), item.title),
            presentation.state_line(),
        ];
        lines.extend(presentation.beneath());

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

    // The last line is the footer or the prompt, with a line above it for
    // whatever the surface has to say.
    let tail = 1 + u16::from(list.notice.is_some() || list.typing.is_some());
    // The count of what could not be rendered sits under the rows, so its row
    // is spoken for too, and one blank row keeps the tail off the last of
    // them. Reserved rather than subtracted: `line` then stops at the
    // reservation, so nothing drawn above can take the footer's place — or the
    // prompt's, which is the only line that shows the cursor. Reserved before
    // the heading rather than after it, because on a screen too short for both
    // the prompt is the one a person cannot do without.
    let counted = u16::from(list.unrenderable > 0);
    frame.reserve(tail + counted + 1);

    frame.line(Emphasis::Plain, &heading(list));
    frame.line(Emphasis::Plain, "");

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
            // The footer is the whole key map, and this line replaces it: a
            // person who pressed `n` by accident has no other way to find out
            // that the surface is in a mode, or how to leave it. It gets a row
            // only when there is one to spare, because the prompt below it is
            // the line that shows the cursor.
            if frame.remaining() > 1 {
                frame.line(
                    Emphasis::Secondary,
                    "An agent — claude or codex — or -- and a command. Quoting is not \
                     interpreted. esc cancels.",
                );
            }
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
    let mut heading = match list.rows.len() + list.unrenderable {
        // The body already says "No sessions.", and a heading counting them
        // again is the same frame saying it twice.
        0 => "Corral".to_owned(),
        1 => "Corral — 1 session".to_owned(),
        other => format!("Corral — {other} sessions"),
    };
    // Totals, not the badge: a header that said two over three Needs You rows
    // because one was acknowledged would be the surface contradicting itself
    // (grill Q23). Nothing needing anyone says nothing.
    if let Some(summary) = &list.summary {
        if summary.needs_you.total > 0 {
            heading.push_str(&format!(" · Needs You {}", summary.needs_you.total));
        }
        if summary.ready.total > 0 {
            heading.push_str(&format!(" · Ready {}", summary.ready.total));
        }
    }
    heading
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

#[cfg(test)]
#[path = "list_tests.rs"]
mod tests;
