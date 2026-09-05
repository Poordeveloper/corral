use super::*;

use std::cell::RefCell;

use corral_client::launch::{LaunchSite, Requested, Shown};
use corral_client::sessions::Listing;
use corral_protocol::method::{AttentionCount, AttentionSummaryResult, SessionListResult};
use futures::channel::mpsc::unbounded;
use gpui::{TestAppContext, VisualTestContext};
use serde_json::json;
use tokio::sync::mpsc::UnboundedReceiver;

use crate::bridge::{Capabilities, Request};
use crate::sessions::ASKING;

/// A status item that remembers what it was shown, in order.
#[derive(Clone, Default)]
struct Remembered(Rc<RefCell<Vec<TrayProjection>>>);

impl Remembered {
    fn count(&self) -> usize {
        self.0.borrow().len()
    }

    fn last(&self) -> TrayProjection {
        self.0.borrow().last().expect("shown something").clone()
    }
}

impl StatusItem for Remembered {
    fn show(&mut self, projection: &TrayProjection) -> Result<(), String> {
        self.0.borrow_mut().push(projection.clone());
        Ok(())
    }
}

fn established() -> (TrayPresence, Remembered) {
    let shown = Remembered::default();
    (TrayPresence::Established(Box::new(shown.clone())), shown)
}

/// The bridge's questions, to answer as the test decides.
type Questions = UnboundedReceiver<Request>;

fn watch(presence: TrayPresence, cx: &mut TestAppContext) -> (Entity<Watch>, Questions) {
    let (bridge, questions) = Bridge::scripted();
    let watch = cx.update(|cx| Watch::install(Rc::new(bridge), presence, cx));
    (watch, questions)
}

fn generation(sessions: Vec<serde_json::Value>) -> Result<Polled, Unanswered> {
    Ok(Polled {
        listing: Listing::of(SessionListResult { sessions }),
        summary: AttentionSummaryResult {
            needs_you: AttentionCount {
                total: 0,
                unacknowledged: 0,
            },
            ready: AttentionCount {
                total: 0,
                unacknowledged: 0,
            },
        },
        capabilities: Capabilities::default(),
    })
}

/// A current answer with nothing Corral started.
fn nothing_continuing() -> Result<Polled, Unanswered> {
    generation(vec![])
}

/// A current answer with one session Corral started, still running.
fn one_running() -> Result<Polled, Unanswered> {
    generation(vec![json!({
        "session_id": "managed-1",
        "title": "sh",
        "origin": "managed",
        "execution_state": "running",
    })])
}

/// A current answer with one session needing you, unacknowledged: a row the
/// tray lists.
fn one_needs_you() -> Result<Polled, Unanswered> {
    Ok(Polled {
        listing: Listing::of(SessionListResult {
            sessions: vec![json!({
                "session_id": "needs-1",
                "title": "fix the test",
                "execution_state": "running",
                "attention": {
                    "state": "needs_you",
                    "since_unix_ms": 0,
                    "items": [{
                        "attention_item_id": "item-1",
                        "reason": "needs_input",
                        "since_unix_ms": 0,
                        "acknowledged": false,
                    }],
                },
            })],
        }),
        summary: AttentionSummaryResult {
            needs_you: AttentionCount {
                total: 1,
                unacknowledged: 1,
            },
            ready: AttentionCount {
                total: 0,
                unacknowledged: 0,
            },
        },
        capabilities: Capabilities::default(),
    })
}

fn unanswered() -> Result<Polled, Unanswered> {
    Err(Unanswered::Silent("nobody there".to_owned()))
}

/// Let the Watch ask, then answer every question out — the list's own poll
/// and a Quit's — with the given generation.
fn answer_polls(
    questions: &mut Questions,
    polled: fn() -> Result<Polled, Unanswered>,
    cx: &mut TestAppContext,
) {
    cx.run_until_parked();
    let mut answered = 0;
    while let Ok(request) = questions.try_recv() {
        let Request::Poll(reply) = request else {
            panic!("the Watch asks only polls");
        };
        let _ = reply.send(polled());
        answered += 1;
    }
    assert!(answered > 0, "a question was out");
    cx.run_until_parked();
}

fn ensure_main_window(watch: &Entity<Watch>, cx: &mut TestAppContext) -> AnyWindowHandle {
    cx.update(|cx| Watch::ensure_main_window(watch, cx))
        .expect("a main window")
}

fn quitting(watch: &Entity<Watch>, cx: &TestAppContext) -> bool {
    watch.read_with(cx, |watch, _| watch.quitting)
}

fn request_quit(watch: &Entity<Watch>, cx: &mut TestAppContext) {
    watch.update(cx, |watch, cx| watch.request_quit(cx));
}

const ONE_RUNNING: &str = "1 session will continue running.";

fn start(watch: &Entity<Watch>, cx: &TestAppContext) -> bool {
    watch
        .read_with(cx, |watch, _| {
            watch.start_session(
                Requested::Command(vec!["sh".to_owned()]),
                LaunchSite {
                    working_directory: None,
                    rows: None,
                    cols: None,
                },
            )
        })
        .is_some()
}

fn continue_session(watch: &Entity<Watch>, cx: &TestAppContext) -> bool {
    watch
        .read_with(cx, |watch, _| {
            watch.continue_session("managed-1".to_owned(), Shown::NotYet, None)
        })
        .is_some()
}

#[test]
fn only_an_established_status_item_keeps_the_process() {
    assert!(established().0.keeps_process());
    assert!(!TrayPresence::Unavailable("no item".to_owned()).keeps_process());
    assert!(!TrayPresence::Unsupported.keeps_process());
}

#[test]
fn the_banner_names_a_failure_and_never_the_platform_gap() {
    assert_eq!(
        TrayPresence::Unavailable("no item".to_owned()).banner(),
        Some(TRAY_UNAVAILABLE)
    );
    assert_eq!(TrayPresence::Unsupported.banner(), None);
    assert_eq!(established().0.banner(), None);
}

/// With a status item the main window is presentation: it closes without a
/// question, the process stays, and the next way in opens a new one.
#[gpui::test]
fn with_a_status_item_the_main_window_closes_and_the_process_stays(cx: &mut TestAppContext) {
    let (presence, _shown) = established();
    let (watch, _questions) = watch(presence, cx);
    let window = ensure_main_window(&watch, cx);
    let mut visual = VisualTestContext::from_window(window, cx);

    assert!(visual.simulate_close());
    assert!(!visual.has_pending_prompt());
    let _ = window.update(&mut *visual, |_, window, _| window.remove_window());
    visual.run_until_parked();

    assert!(visual.windows().is_empty());
    assert!(!quitting(&watch, &visual));
    let again = ensure_main_window(&watch, &mut visual);
    assert!(again != window);
    assert!(visual.windows() == vec![again]);
}

/// The item says "asking" from the first moment, then each generation as it
/// arrives — and not again for the same one: the poll is not a rebuild.
#[gpui::test]
fn the_status_item_is_shown_a_generation_only_when_it_differs(cx: &mut TestAppContext) {
    let (presence, shown) = established();
    let (_watch, mut questions) = watch(presence, cx);
    assert_eq!(shown.count(), 1);
    assert_eq!(
        shown.last(),
        TrayProjection::Unreachable {
            line: ASKING.to_owned()
        }
    );

    answer_polls(&mut questions, nothing_continuing, cx);
    assert_eq!(shown.count(), 2);
    assert_eq!(shown.last().header(), "Needs You 0 · Ready 0");

    cx.executor().advance_clock(POLL);
    answer_polls(&mut questions, nothing_continuing, cx);
    assert_eq!(shown.count(), 2);

    cx.executor().advance_clock(POLL);
    answer_polls(&mut questions, one_needs_you, cx);
    assert_eq!(shown.count(), 3);
    assert_eq!(shown.last().badge_text(), Some("1".to_owned()));

    cx.executor().advance_clock(POLL);
    answer_polls(&mut questions, unanswered, cx);
    assert_eq!(shown.count(), 4);
    assert_eq!(shown.last().badge_text(), None);
    assert_eq!(
        shown.last().header(),
        "corrald did not answer: nobody there"
    );
}

/// A row click names a session: listed, it is selected and opened through
/// the window's own path; gone since the menu was built, the window opens on
/// the list as it is now and nothing else happens.
#[gpui::test]
fn a_row_click_opens_the_session_it_named_and_a_stale_one_converges(cx: &mut TestAppContext) {
    let (presence, _shown) = established();
    let (watch, mut questions) = watch(presence, cx);
    answer_polls(&mut questions, one_needs_you, cx);
    assert!(watch.read_with(cx, |watch, _| watch.list().selected().is_none()));

    cx.update(|cx| Watch::act(&watch, TrayAction::OpenSession("needs-1".to_owned()), cx));
    cx.run_until_parked();
    assert_eq!(cx.windows().len(), 1);
    assert_eq!(
        watch.read_with(cx, |watch, _| watch
            .list()
            .selected()
            .map(|row| row.session_id.clone())),
        Some("needs-1".to_owned())
    );
    // Held, not dropped: a dropped request is a refused attach, which the
    // window answers with a refresh of its own.
    let attach = questions.try_recv().expect("the window asked to attach");
    assert!(matches!(&attach, Request::Attach { session_id, .. } if session_id == "needs-1"));

    cx.update(|cx| Watch::act(&watch, TrayAction::OpenSession("gone".to_owned()), cx));
    cx.run_until_parked();
    assert_eq!(cx.windows().len(), 1);
    assert_eq!(
        watch.read_with(cx, |watch, _| watch
            .list()
            .selected()
            .map(|row| row.session_id.clone())),
        Some("needs-1".to_owned())
    );
    assert!(
        questions.try_recv().is_err(),
        "nothing was asked for a session not listed"
    );
    drop(attach);
}

/// What the platform's handler forwarded reaches the Watch on the
/// foreground as an action; an id this build cannot read is dropped; the
/// tray's Quit runs the one gate.
#[gpui::test]
fn tray_clicks_reach_the_watch_on_the_foreground(cx: &mut TestAppContext) {
    let (presence, _shown) = established();
    let (watch, mut questions) = watch(presence, cx);
    let (clicks, receiver) = unbounded();
    cx.update(|cx| Watch::bind_tray(&watch, receiver, cx));

    clicks.unbounded_send("open".to_owned()).expect("bound");
    cx.run_until_parked();
    assert_eq!(cx.windows().len(), 1);

    clicks.unbounded_send("session:".to_owned()).expect("bound");
    clicks.unbounded_send("new".to_owned()).expect("bound");
    cx.run_until_parked();
    assert_eq!(cx.windows().len(), 1);
    assert!(!quitting(&watch, cx));

    clicks.unbounded_send("quit".to_owned()).expect("bound");
    answer_polls(&mut questions, nothing_continuing, cx);
    assert!(quitting(&watch, cx));
}

#[gpui::test]
fn ensure_main_window_reuses_the_open_window(cx: &mut TestAppContext) {
    let (watch, _questions) = watch(TrayPresence::Unsupported, cx);

    let first = ensure_main_window(&watch, cx);
    let again = ensure_main_window(&watch, cx);

    assert!(first == again);
    assert!(cx.windows() == vec![first]);
}

/// The last generation said nothing continues; a newer question is what the
/// gate waits for, and it may say otherwise — a session Corral just started
/// reaches the list only with an answer asked for after it.
#[gpui::test]
fn quit_decides_from_a_generation_asked_for_after_the_request(cx: &mut TestAppContext) {
    let (watch, mut questions) = watch(TrayPresence::Unsupported, cx);
    answer_polls(&mut questions, nothing_continuing, cx);
    assert!(watch.read_with(cx, |watch, _| watch.list().is_current()));

    request_quit(&watch, cx);
    cx.run_until_parked();
    assert!(!quitting(&watch, cx));
    assert!(!cx.has_pending_prompt());

    answer_polls(&mut questions, one_running, cx);
    assert!(!quitting(&watch, cx));
    let (message, _) = cx.pending_prompt().expect("the fresh answer warns");
    assert_eq!(message, ONE_RUNNING);
}

#[gpui::test]
fn quit_with_nothing_continuing_asks_nothing(cx: &mut TestAppContext) {
    let (watch, mut questions) = watch(TrayPresence::Unsupported, cx);
    answer_polls(&mut questions, one_running, cx);

    request_quit(&watch, cx);
    answer_polls(&mut questions, nothing_continuing, cx);

    assert!(quitting(&watch, cx));
    assert!(!cx.has_pending_prompt());
}

/// A daemon that does not answer leaves nothing known, so Quit asks — once
/// per attempt, in a main window opened for it — and Cancel keeps watching.
#[gpui::test]
fn quit_asks_once_when_the_daemon_does_not_answer_and_cancel_keeps_watching(
    cx: &mut TestAppContext,
) {
    let (watch, mut questions) = watch(TrayPresence::Unsupported, cx);
    let Gate::Warn(expected) = quit::gate(&SessionList::default()) else {
        panic!("an unanswered list warns");
    };

    request_quit(&watch, cx);
    request_quit(&watch, cx);
    answer_polls(&mut questions, unanswered, cx);

    assert_eq!(cx.windows().len(), 1);
    let (message, detail) = cx.pending_prompt().expect("one confirmation");
    assert_eq!(message, expected.message);
    assert_eq!(detail, expected.detail);

    cx.simulate_prompt_answer("Cancel");
    cx.run_until_parked();
    assert!(!quitting(&watch, cx));
    assert!(!cx.has_pending_prompt());
    assert!(!watch.read_with(cx, |watch, _| watch.quit_pending));

    request_quit(&watch, cx);
    answer_polls(&mut questions, unanswered, cx);
    assert!(cx.has_pending_prompt());
    cx.simulate_prompt_answer("Quit");
    cx.run_until_parked();
    assert!(quitting(&watch, cx));
}

/// Without a status item, closing the main window is quitting, so the close
/// runs the same gate: the window stays while the question is out, warns
/// when a session Corral started continues, and closes with the process
/// once the gate says quit.
#[gpui::test]
fn closing_the_main_window_without_a_status_item_runs_the_gate(cx: &mut TestAppContext) {
    let (watch, mut questions) = watch(TrayPresence::Unavailable("no item".to_owned()), cx);
    let window = ensure_main_window(&watch, cx);
    let mut visual = VisualTestContext::from_window(window, cx);

    assert!(!visual.simulate_close());
    assert!(visual.windows() == vec![window]);
    answer_polls(&mut questions, one_running, &mut visual);
    let (message, _) = visual.pending_prompt().expect("the close warns");
    assert_eq!(message, ONE_RUNNING);

    visual.simulate_prompt_answer("Cancel");
    visual.run_until_parked();
    assert!(visual.windows() == vec![window]);
    assert!(!quitting(&watch, &visual));

    assert!(!visual.simulate_close());
    answer_polls(&mut questions, nothing_continuing, &mut visual);
    assert!(quitting(&watch, &visual));
    assert!(!visual.has_pending_prompt());
}

/// While the gate's question is out, nothing that could start a runtime is
/// sent: the bridge answers in order, so a start queued behind the question
/// would begin a session its answer does not carry, and Quit would commit
/// over it.
#[gpui::test]
fn a_pending_quit_refuses_what_could_start_a_runtime(cx: &mut TestAppContext) {
    let (watch, mut questions) = watch(TrayPresence::Unsupported, cx);
    answer_polls(&mut questions, nothing_continuing, cx);

    request_quit(&watch, cx);
    cx.run_until_parked();
    let Ok(Request::Poll(question)) = questions.try_recv() else {
        panic!("the gate's question is out");
    };
    assert!(!start(&watch, cx));
    assert!(!continue_session(&watch, cx));
    assert!(
        questions.try_recv().is_err(),
        "nothing queued behind the question"
    );

    let _ = question.send(nothing_continuing());
    cx.run_until_parked();
    assert!(quitting(&watch, cx));
    assert!(!start(&watch, cx));
}

/// Cancel lifts the refusal: the next start is sent.
#[gpui::test]
fn cancel_lifts_the_refusal(cx: &mut TestAppContext) {
    let (watch, mut questions) = watch(TrayPresence::Unsupported, cx);
    request_quit(&watch, cx);
    answer_polls(&mut questions, one_running, cx);
    assert!(cx.has_pending_prompt());
    assert!(!start(&watch, cx));

    cx.simulate_prompt_answer("Cancel");
    cx.run_until_parked();

    assert!(start(&watch, cx));
    assert!(matches!(questions.try_recv(), Ok(Request::Start { .. })));
    assert!(continue_session(&watch, cx));
    assert!(matches!(questions.try_recv(), Ok(Request::Continue { .. })));
}
