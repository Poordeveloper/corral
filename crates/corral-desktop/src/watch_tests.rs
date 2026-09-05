use super::*;

use corral_client::sessions::Listing;
use corral_protocol::method::{AttentionCount, AttentionSummaryResult, SessionListResult};
use gpui::{TestAppContext, VisualTestContext};
use serde_json::json;
use tokio::sync::mpsc::UnboundedReceiver;

use crate::bridge::{Capabilities, Request};

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

#[test]
fn only_an_established_status_item_keeps_the_process() {
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
