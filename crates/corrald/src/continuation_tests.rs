use super::*;
use corral_core::{CorralSessionId, ExternalId};

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
    let first = revision(session(), "history-live-state-unknown", claude, &id, 1_000);
    let again = revision(session(), "history-live-state-unknown", claude, &id, 1_000);
    assert_eq!(first, again);
    let moved = revision(session(), "history-live-state-unknown", claude, &id, 2_000);
    assert_ne!(
        first, moved,
        "a newer store recency is a different decision"
    );
    let other = revision(session(), "something-else", claude, &id, 1_000);
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
