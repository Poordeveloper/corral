use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

use super::*;
use corral_core::{CorralSessionId, ExternalId};

use crate::state::DaemonState;

static COUNTER: AtomicU32 = AtomicU32::new(0);

/// A daemon holding nothing but a registry, for the decisions that read one.
struct Scratch {
    state: Arc<DaemonState>,
    directory: PathBuf,
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}

fn scratch(name: &str) -> Scratch {
    let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
    let directory = std::env::temp_dir().join(format!(
        "corrald-continuation-{}-{unique}-{name}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).expect("create the scratch directory");
    let state = DaemonState::open(
        &directory.join("registry.sqlite3"),
        &directory.join("launch"),
        &directory,
    )
    .expect("open");
    Scratch {
        state: Arc::new(state),
        directory,
    }
}

fn session() -> CorralSessionId {
    "01912345-6789-7abc-8def-0123456789ab"
        .parse()
        .expect("a session id")
}

/// The revision is a correlation handle: the same decision on the same
/// facts yields the same one, and a different fact yields a different one,
/// so a resume carrying it can be matched against what was shown.
#[test]
fn a_revision_follows_the_facts_the_decision_was_made_on() {
    let claude = KnownProvider::Claude;
    let id = ExternalId::new("abc").expect("an external id");
    let first = revision(
        session(),
        "history-live-state-unknown",
        claude,
        &id,
        1_000,
        std::path::Path::new("/tmp/w"),
    );
    let again = revision(
        session(),
        "history-live-state-unknown",
        claude,
        &id,
        1_000,
        std::path::Path::new("/tmp/w"),
    );
    assert_eq!(first, again);
    let moved = revision(
        session(),
        "history-live-state-unknown",
        claude,
        &id,
        2_000,
        std::path::Path::new("/tmp/w"),
    );
    assert_ne!(
        first, moved,
        "a newer store recency is a different decision"
    );
    let other = revision(
        session(),
        "something-else",
        claude,
        &id,
        1_000,
        std::path::Path::new("/tmp/w"),
    );
    assert_ne!(first, other);
    assert_eq!(first.len(), 16, "{first}");
}

/// The words differ by who owns the live Run, because the person does a
/// different thing with each: open the managed one, wait for the external
/// one. Only the external one, which the sweep observes, is called running.
#[test]
fn an_unverified_end_is_worded_by_whose_run_it_is() {
    let managed = refused_words(&ResumeRefused::EndUnverifiable, LiveRun::Managed);
    assert!(managed.contains("couldn't verify"), "{managed}");
    let external = refused_words(&ResumeRefused::EndUnverifiable, LiveRun::External);
    assert!(
        external.contains("Still running outside Corral"),
        "{external}"
    );
    assert!(
        !managed.contains("running"),
        "no liveness is asserted of it: {managed}"
    );
    let live = refused_words(&ResumeRefused::RunStillLive, LiveRun::Managed);
    assert!(live.contains("Open"), "{live}");
}

/// A resume that carries the revision it was shown continues; one that
/// carries none, or an older one, is turned back to ask again — and the
/// answer is a code the client branches on, not prose to parse.
#[test]
fn a_disclosed_continuation_needs_the_revision_it_was_shown() {
    assert_eq!(shown(Some("abcd"), Some("abcd")), Shown::Matching);
    assert_eq!(shown(Some("abcd"), Some("older")), Shown::Stale);
    assert_eq!(shown(Some("abcd"), None), Shown::Stale);
    assert_eq!(shown(None, None), Shown::NotNeeded);
    assert_eq!(
        shown(None, Some("abcd")),
        Shown::NotNeeded,
        "an unneeded one is not held against it"
    );
}

// ------------------------------------------- where a history row continues

/// A history row carries no location, and the provider resumes wherever it is
/// started, so the directory is the client's to state. Nothing is defaulted:
/// no daemon cwd, no decoded project path, no home (Q35).
#[test]
fn a_directory_is_never_guessed_for_a_history_row() {
    let scratch = std::env::temp_dir().join(format!("corrald-q35-{}", std::process::id()));
    std::fs::create_dir_all(&scratch).expect("a scratch directory");
    let file = scratch.join("not-a-directory");
    std::fs::write(&file, b"").expect("write");

    assert_eq!(
        usable_directory(None),
        Err(DirectoryRefusal::NotSupplied),
        "an absent directory is not an invitation to pick one"
    );
    assert_eq!(
        usable_directory(Some(std::path::Path::new(""))),
        Err(DirectoryRefusal::NotSupplied)
    );
    assert_eq!(
        usable_directory(Some(std::path::Path::new("proj"))),
        Err(DirectoryRefusal::Relative("proj".into())),
        "a relative path would resolve against whatever cwd the daemon has"
    );
    let missing = scratch.join("gone");
    assert_eq!(
        usable_directory(Some(&missing)),
        Err(DirectoryRefusal::Missing(missing))
    );
    assert_eq!(
        usable_directory(Some(&file)),
        Err(DirectoryRefusal::NotADirectory(file))
    );
    assert_eq!(usable_directory(Some(&scratch)), Ok(scratch.clone()));

    std::fs::remove_dir_all(&scratch).expect("clean up");
}

/// The person is told the exact directory another provider process will be
/// started in, and the revision is bound to it: changing the directory after
/// the preflight is a different decision, not the same one shown twice.
#[test]
fn the_disclosure_names_the_directory_and_the_revision_is_bound_to_it() {
    let claude = KnownProvider::Claude;
    let id = ExternalId::new("abc").expect("an external id");
    let here = revision(
        session(),
        HISTORY_LIVE_STATE_UNKNOWN,
        claude,
        &id,
        1_000,
        std::path::Path::new("/tmp/here"),
    );
    let there = revision(
        session(),
        HISTORY_LIVE_STATE_UNKNOWN,
        claude,
        &id,
        1_000,
        std::path::Path::new("/tmp/there"),
    );
    assert_ne!(here, there, "the directory is one of the decision's facts");

    let text = disclosure_text(claude, std::path::Path::new("/tmp/here"));
    assert!(text.contains("still running somewhere else"), "{text}");
    assert!(
        text.contains("another Claude Code process"),
        "the provider is named: {text}"
    );
    assert!(text.contains("/tmp/here"), "the directory is named: {text}");
}

/// The fourth rung, through the ladder a client actually calls: a row the
/// provider's store holds, no Run of any kind, and the directory the client
/// named. Nothing durable exists for it — that is what makes it the fourth
/// rung — so the decision is the whole of what Corral can offer here.
#[tokio::test]
async fn a_history_row_is_eligible_once_the_client_says_where() {
    let registry = scratch("history-row");
    let scratch = registry.directory.join("proj");
    std::fs::create_dir_all(&scratch).expect("a directory to continue in");
    let external_id = ExternalId::new("session-in-history").expect("an external id");
    let row = crate::history::HistoryEntry {
        provider: KnownProvider::Claude,
        external_id: external_id.clone(),
        last_active: std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_700),
        store_label: "-root-proj".to_owned(),
        path: registry.directory.join("-root-proj/x.jsonl"),
    };
    registry
        .state
        .with_runtime(|runtime| {
            runtime
                .history
                .replace(KnownProvider::Claude, vec![row], Vec::new());
        })
        .expect("the runtime is available");
    let listed = registry
        .state
        .with_runtime(|runtime| runtime.history.rows())
        .expect("the runtime is available");
    let session = listed.first().expect("one row").session;

    // Sealed here, because this test is about the directory rather than
    // about the install; `history::task` covers the sealing decision.
    let silent = decide_with(&registry.state, session, None, |_| true)
        .await
        .expect("decided");
    let Decision::Refused { reason, .. } = silent else {
        panic!("a directory Corral was not told is not one it may choose");
    };
    assert!(reason.contains("which directory"), "{reason}");

    let decided = decide_with(&registry.state, session, Some(&scratch), |_| true)
        .await
        .expect("decided");
    let Decision::EligibleWithDisclosure {
        disclosure,
        revision,
        plan,
    } = decided
    else {
        panic!("a history row with a directory is eligible, with the unknown said");
    };
    assert_eq!(disclosure.code, HISTORY_LIVE_STATE_UNKNOWN);
    assert!(
        disclosure.text.contains(&scratch.display().to_string()),
        "{}",
        disclosure.text
    );
    assert_eq!(shown(Some(&revision), Some(&revision)), Shown::Matching);
    // What the continuation would run, if it were answered.
    assert_eq!(plan.provider, KnownProvider::Claude);
    assert_eq!(plan.external_id, external_id);
    assert_eq!(plan.working_directory, scratch);

    let elsewhere = registry.directory.join("other");
    std::fs::create_dir_all(&elsewhere).expect("another directory");
    let moved = decide_with(&registry.state, session, Some(&elsewhere), |_| true)
        .await
        .expect("decided");
    let Decision::EligibleWithDisclosure {
        revision: moved, ..
    } = moved
    else {
        panic!("still eligible");
    };
    assert_eq!(
        shown(Some(&moved), Some(&revision)),
        Shown::Stale,
        "a directory changed after the preflight is a different decision"
    );
}
