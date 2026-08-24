use std::ffi::OsString;
use std::time::{Duration, Instant};

use super::*;
use crate::runtime::launch::LaunchRequest;
use crate::runtime::spawn::{PtyGeometry, spawn};

const GEOMETRY: PtyGeometry = PtyGeometry { rows: 24, cols: 80 };

fn request(script: &str) -> LaunchRequest {
    LaunchRequest::new(
        "/bin/sh",
        ["-c", script].iter().map(OsString::from),
        std::env::temp_dir(),
    )
    .expect("a valid launch request")
}

/// What a finished pump leaves behind.
///
/// The terminal itself cannot cross the thread boundary — it is not `Send` —
/// so a test asks its questions on the pump's own thread and carries back the
/// answers. Every consumer of a session's screen will do the same thing for
/// the same reason.
struct Pumped {
    title: Option<Vec<u8>>,
    retained_rows: usize,
}

/// Run the pump on its own thread and wait for it, so a test that would
/// otherwise block forever fails instead.
fn pump_to_completion(script: &str) -> Pumped {
    let mut runtime = spawn(&request(script), GEOMETRY).expect("the program starts");
    let (sender, receiver) = std::sync::mpsc::channel();

    std::thread::spawn(move || {
        let mut terminal = AuthoritativeTerminal::new(GEOMETRY);
        let end = pump(&runtime, &mut terminal);
        let _ = runtime.wait();
        let _ = sender.send(end.map(|_| Pumped {
            title: terminal.title().map(<[u8]>::to_vec),
            retained_rows: terminal.terminal().screens.active().pages.total_rows(),
        }));
    });

    receiver
        .recv_timeout(Duration::from_secs(10))
        .expect("the pump finished")
        .expect("the pump did not fail")
}

/// The contract that makes an unattached session usable: a child that queries
/// its terminal is answered by the daemon, with no client anywhere.
#[test]
fn an_unattached_child_gets_its_device_query_answered() {
    let started = Instant::now();

    // The child blocks until the reply arrives; without one this script hangs
    // and the harness times out rather than passing. Raw mode because a device
    // reply carries no newline, and a line-buffered read would wait for one
    // that never comes — the same trap an unattached agent would hit.
    let pumped = pump_to_completion(
        "stty raw -echo; printf '\\033[c'; dd bs=1 count=1 >/dev/null 2>&1; \
         stty sane; printf '\\033]2;answered\\007'",
    );

    assert!(
        started.elapsed() < Duration::from_secs(10),
        "the child waited for a reply that never came"
    );
    assert_eq!(
        pumped.title.as_deref(),
        Some(b"answered".as_slice()),
        "the child never got past its own device query"
    );
}

/// The pump ends when the child's side of the terminal closes. That is an end,
/// not a failure: on Unix the master read fails with EIO once the child is
/// gone, and the backend maps it to EOF.
#[test]
fn the_pump_ends_when_the_child_closes_the_terminal() {
    let mut runtime = spawn(&request("printf 'done\\r\\n'"), GEOMETRY).expect("the program starts");
    let mut terminal = AuthoritativeTerminal::new(GEOMETRY);

    let end = pump(&runtime, &mut terminal).expect("the pump did not fail");

    assert!(matches!(end, PumpEnd::Closed), "{end:?}");
    let _ = runtime.wait();
}

/// Output reaches the authoritative screen, which is what every surface will
/// render from.
#[test]
fn child_output_lands_in_the_authoritative_terminal() {
    let pumped = pump_to_completion("printf 'hello from the child\\r\\n'");

    assert!(pumped.retained_rows >= usize::from(GEOMETRY.rows));
}

/// A title the child sets during the session is held by the daemon, ready for
/// the snapshot that has to re-emit it (ADR 0003 D3).
#[test]
fn a_title_set_by_the_child_survives_in_the_daemons_state() {
    let pumped = pump_to_completion("printf '\\033]2;building\\007'");

    assert_eq!(pumped.title.as_deref(), Some(b"building".as_slice()));
}
