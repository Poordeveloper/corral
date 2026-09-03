//! What a pass does when the store it read stops being readable evidence.

use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, Ordering};
use std::thread::ThreadId;

use super::*;
use crate::continuation;

static COUNTER: AtomicU32 = AtomicU32::new(0);

/// 2026-09-02T12:00:00Z.
fn now() -> SystemTime {
    SystemTime::UNIX_EPOCH + Duration::from_secs(1_788_350_400)
}

/// A daemon whose provider home holds one Claude session, recently touched.
fn daemon(name: &str) -> (Arc<DaemonState>, std::path::PathBuf) {
    let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
    let directory = std::env::temp_dir().join(format!(
        "corrald-history-task-{}-{unique}-{name}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).expect("scratch");
    let state = Arc::new(
        DaemonState::open(
            &directory.join("registry.sqlite3"),
            &directory.join("launch"),
            &directory,
        )
        .expect("open"),
    );
    let home = directory.join("home");
    let session =
        home.join(".claude/projects/-root-proj/0f9b6c1a-1111-4111-8111-000000000001.jsonl");
    std::fs::create_dir_all(session.parent().expect("parent")).expect("project");
    std::fs::write(&session, "{}\n").expect("session file");
    std::fs::File::options()
        .write(true)
        .open(&session)
        .expect("open")
        .set_modified(now() - Duration::from_secs(3_600))
        .expect("mtime");
    state.attach_provider_home(home);
    (state, directory)
}

/// A provider that stops being sealed — upgraded in place to a version the
/// matrix has not measured, uninstalled, or installed in a shape whose
/// version cannot be read; all three are this predicate saying no — takes its
/// rows with it. Kept, they would stay listable *and continuable* on a layout
/// claim this daemon can no longer make (ADR 0016 D1).
#[tokio::test]
async fn a_provider_that_stops_being_sealed_takes_its_rows_with_it() {
    let (state, directory) = daemon("unsealed");

    pass(&state, now(), sealed_at).await;
    let listed = state
        .with_runtime(|runtime| runtime.history.rows())
        .expect("the runtime");
    assert_eq!(
        listed.len(),
        1,
        "the sealed pass listed the store's session"
    );
    let row = listed[0].session;
    assert!(
        matches!(
            continuation::decide_with(&state, row, Some(std::path::Path::new("/tmp")), sealed_at)
                .await,
            Ok(continuation::Decision::EligibleWithDisclosure { .. })
        ),
        "a row read under a sealed layout is continuable"
    );

    pass(&state, now(), not_sealed).await;

    assert!(
        state
            .with_runtime(|runtime| runtime.history.rows())
            .expect("the runtime")
            .is_empty(),
        "the evidence the rows stood on is gone"
    );
    assert!(
        matches!(
            continuation::decide_with(&state, row, Some(std::path::Path::new("/tmp")), sealed_at)
                .await,
            Ok(continuation::Decision::Refused { .. })
        ),
        "and so is the continuation it supported"
    );
    let _ = std::fs::remove_dir_all(&directory);
}

/// A registry that cannot answer is not the same statement. Nothing is known
/// about the store this pass, so the previous pass stands rather than being
/// retracted on evidence nobody has (AGENTS.md §Runtime truth).
#[tokio::test]
async fn a_sealed_provider_keeps_its_rows_across_a_pass_that_found_nothing_new() {
    let (state, directory) = daemon("still-sealed");

    pass(&state, now(), sealed_at).await;
    pass(&state, now(), sealed_at).await;

    assert_eq!(
        state
            .with_runtime(|runtime| runtime.history.rows())
            .expect("the runtime")
            .len(),
        1
    );
    let _ = std::fs::remove_dir_all(&directory);
}

/// The version was sealed on a file, so the plan names that file.
///
/// Nothing downstream may resolve the provider's program name a second time:
/// between the sealing check and the exec, an in-place upgrade would make the
/// two names refer to different binaries, and Corral would run a version it
/// never measured while reporting that it had. `continue_history_row` hands
/// this straight to the launch, which is what makes the check bind.
#[tokio::test]
async fn the_plan_names_the_executable_the_sealing_check_read() {
    let (state, directory) = daemon("sealed-executable");
    pass(&state, now(), sealed_at).await;
    let row = state
        .with_runtime(|runtime| runtime.history.rows())
        .expect("the runtime")[0]
        .session;

    let decision =
        continuation::decide_with(&state, row, Some(std::path::Path::new("/tmp")), sealed_at).await;

    let Ok(continuation::Decision::EligibleWithDisclosure { plan, .. }) = decision else {
        panic!("a sealed install continues");
    };
    assert_eq!(
        plan.executable,
        sealed_at(KnownProvider::Claude).expect("sealed").executable,
        "the launch runs what the check read, not a name resolved again"
    );
    let _ = std::fs::remove_dir_all(&directory);
}

/// The cadence is not a licence. Between an in-place upgrade and the pass
/// that would retract the rows, a continuation would otherwise start the
/// version installed *now* on evidence read under the version installed
/// *then* — and an unmeasured version inherits nothing (ADR 0016). The
/// decision asks again, so the re-decision `session.resume` makes on its way
/// to spawning asks again too, exactly as it already does for the working
/// directory.
#[tokio::test]
async fn a_row_is_refused_the_moment_its_provider_stops_being_sealed() {
    let (state, directory) = daemon("unsealed-at-decision");
    pass(&state, now(), sealed_at).await;
    let row = state
        .with_runtime(|runtime| runtime.history.rows())
        .expect("the runtime")[0]
        .session;

    // No pass in between: the install changed under the daemon, and this is
    // the next thing the daemon is asked.
    let decision =
        continuation::decide_with(&state, row, Some(std::path::Path::new("/tmp")), not_sealed)
            .await;

    assert!(
        matches!(decision, Ok(continuation::Decision::Refused { .. })),
        "an unmeasured version inherits nothing"
    );
    assert!(
        state
            .with_runtime(|runtime| runtime.history.rows())
            .expect("the runtime")
            .is_empty(),
        "and the row it refused is not still offered"
    );
    let _ = std::fs::remove_dir_all(&directory);
}

/// Where the sealing question was answered, recorded by the predicate itself.
/// A `fn` pointer carries no captures, so the answer comes back here.
static PROBED_ON: Mutex<Option<ThreadId>> = Mutex::new(None);

fn sealed_recording_its_thread(provider: KnownProvider) -> Option<crate::history::SealedInstall> {
    *PROBED_ON.lock().expect("the recorder") = Some(std::thread::current().id());
    sealed_at(provider)
}

/// `corrald` runs one reactor thread, and resolving the installed version
/// walks `PATH`, stats what it finds, canonicalizes it, and reads package
/// metadata. Asked inline, every cadence, one slow mount on `PATH` would
/// stall client requests, hook delivery and timers along with the refresh.
///
/// Asserted structurally rather than by timing: the question must be answered
/// somewhere other than the thread the pass is running on.
#[tokio::test]
async fn a_pass_asks_about_the_install_off_the_reactor() {
    let (state, directory) = daemon("off-reactor");
    *PROBED_ON.lock().expect("the recorder") = None;

    pass(&state, now(), sealed_recording_its_thread).await;

    let probed = PROBED_ON
        .lock()
        .expect("the recorder")
        .expect("it was asked");
    assert_ne!(
        probed,
        std::thread::current().id(),
        "the reactor thread waited on the filesystem"
    );
    let _ = std::fs::remove_dir_all(&directory);
}

/// A sealed install standing in for this machine's, so a test drives the
/// transition rather than whatever happens to be on `PATH`.
fn sealed_at(provider: KnownProvider) -> Option<crate::history::SealedInstall> {
    let _ = provider;
    Some(crate::history::SealedInstall {
        executable: std::path::PathBuf::from("/opt/measured/claude"),
        version: "2.1.259".to_owned(),
    })
}

fn not_sealed(_: KnownProvider) -> Option<crate::history::SealedInstall> {
    None
}
