use super::*;

use std::path::PathBuf;

use corral_client::sessions::Listing;
use corral_client::{ClientActivationPolicy, EndpointSelection};
use corral_protocol::method::{AttentionCount, AttentionSummaryResult, SessionListResult};
use gpui::TestAppContext;

use crate::bridge::Capabilities;

/// A bridge with nobody behind it: every poll comes back unanswered, which is
/// the state before the daemon has answered at all.
fn silent_bridge() -> Rc<Bridge> {
    Rc::new(Bridge::start(
        ClientActivationPolicy::default(),
        EndpointSelection::Explicit(PathBuf::from("/nonexistent/corral/run/corrald.sock")),
    ))
}

fn watch(presence: TrayPresence, cx: &mut TestAppContext) -> Entity<Watch> {
    cx.update(|cx| Watch::install(silent_bridge(), presence, cx))
}

/// A current answer with nothing Corral started.
fn nothing_continuing() -> Result<Polled, Unanswered> {
    Ok(Polled {
        listing: Listing::of(SessionListResult { sessions: vec![] }),
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

fn ensure_main_window(watch: &Entity<Watch>, cx: &mut TestAppContext) -> AnyWindowHandle {
    cx.update(|cx| Watch::ensure_main_window(watch, cx))
        .expect("a main window")
}

fn quitting(watch: &Entity<Watch>, cx: &TestAppContext) -> bool {
    watch.read_with(cx, |watch, _| watch.quitting)
}

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
    let watch = watch(TrayPresence::Unsupported, cx);

    let first = ensure_main_window(&watch, cx);
    let again = ensure_main_window(&watch, cx);

    assert!(first == again);
    assert!(cx.windows() == vec![first]);
}

#[gpui::test]
fn closing_the_main_window_without_a_status_item_quits(cx: &mut TestAppContext) {
    let watch = watch(TrayPresence::Unavailable("no item".to_owned()), cx);
    let window = ensure_main_window(&watch, cx);
    assert!(!quitting(&watch, cx));

    cx.update(|cx| {
        window
            .update(cx, |_, window, _| window.remove_window())
            .expect("the window is open");
    });

    assert!(cx.windows().is_empty());
    assert!(quitting(&watch, cx));
}

#[gpui::test]
fn quit_with_nothing_continuing_asks_nothing(cx: &mut TestAppContext) {
    let watch = watch(TrayPresence::Unsupported, cx);

    watch.update(cx, |watch, cx| {
        watch.finish_poll(nothing_continuing(), cx);
        watch.request_quit(cx);
    });

    assert!(quitting(&watch, cx));
    assert!(!cx.has_pending_prompt());
}

/// Before the daemon has answered nothing is known, so Quit asks — once per
/// attempt, in a main window opened for it — and Cancel keeps watching.
#[gpui::test]
fn quit_before_the_daemon_answers_asks_once_and_cancel_keeps_watching(cx: &mut TestAppContext) {
    let watch = watch(TrayPresence::Unsupported, cx);
    let Gate::Warn(expected) = quit::gate(&SessionList::default()) else {
        panic!("an unanswered list warns");
    };

    watch.update(cx, |watch, cx| {
        watch.request_quit(cx);
        watch.request_quit(cx);
    });

    assert_eq!(cx.windows().len(), 1);
    let (message, detail) = cx.pending_prompt().expect("one confirmation");
    assert_eq!(message, expected.message);
    assert_eq!(detail, expected.detail);

    cx.simulate_prompt_answer("Cancel");
    cx.run_until_parked();
    assert!(!quitting(&watch, cx));
    assert!(!cx.has_pending_prompt());
    assert!(!watch.read_with(cx, |watch, _| watch.quit_pending));

    watch.update(cx, |watch, cx| watch.request_quit(cx));
    assert!(cx.has_pending_prompt());
    cx.simulate_prompt_answer("Quit");
    cx.run_until_parked();
    assert!(quitting(&watch, cx));
}
