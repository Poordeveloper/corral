use std::time::{Duration, SystemTime};

use super::*;
use crate::provider::KnownProvider;

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("corral-history-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch");
    dir
}

fn touch(path: &std::path::Path, modified: SystemTime) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("parent");
    }
    std::fs::write(path, "{}\n").expect("write");
    let file = std::fs::File::options()
        .write(true)
        .open(path)
        .expect("open");
    file.set_modified(modified).expect("set mtime");
}

fn now() -> SystemTime {
    SystemTime::UNIX_EPOCH + Duration::from_secs(1_788_350_400)
}

const A: &str = "0f9b6c1a-1111-4111-8111-000000000001";
const B: &str = "0f9b6c1a-2222-4222-8222-000000000002";
const C: &str = "0f9b6c1a-3333-4333-8333-000000000003";

/// Claude files one `<uuid>.jsonl` per session under the encoded working
/// directory; directories beside them, and `memory/`, are not sessions
/// (ADR 0016 D1, matrix "Session stores").
#[test]
fn claude_sessions_are_the_top_level_jsonl_files_only() {
    let home = scratch("claude-layout");
    let project = home.join(".claude/projects/-root-proj");
    touch(
        &project.join(format!("{A}.jsonl")),
        now() - Duration::from_secs(3_600),
    );
    touch(
        &project.join(format!("{B}.jsonl")),
        now() - Duration::from_secs(60),
    );
    std::fs::create_dir_all(project.join(A)).expect("session dir");
    std::fs::create_dir_all(project.join("memory")).expect("memory dir");
    touch(&project.join("notes.txt"), now());

    let entries = enumerate(
        KnownProvider::Claude,
        &store_root(KnownProvider::Claude, &home),
        now(),
        &Recent::default(),
    );
    let ids: Vec<&str> = entries
        .iter()
        .map(|entry| entry.external_id.as_str())
        .collect();
    assert_eq!(ids, [B, A], "newest first, files only");
    assert_eq!(entries[0].store_label, "-root-proj");
    assert_eq!(entries[0].last_active, now() - Duration::from_secs(60));
}

/// Codex names each rollout with the session's own thread id, under a
/// date tree; the id is read from the name and nothing else.
#[test]
fn codex_sessions_are_named_by_their_thread_id() {
    let home = scratch("codex-layout");
    let day = home.join(".codex/sessions/2026/09/02");
    touch(
        &day.join(format!("rollout-2026-09-02T12-49-39-{A}.jsonl")),
        now() - Duration::from_secs(120),
    );
    touch(
        &day.join(format!("rollout-2026-09-02T13-00-00-{B}.jsonl")),
        now() - Duration::from_secs(30),
    );
    touch(&day.join("not-a-rollout.jsonl"), now());

    let entries = enumerate(
        KnownProvider::Codex,
        &store_root(KnownProvider::Codex, &home),
        now(),
        &Recent::default(),
    );
    let ids: Vec<&str> = entries
        .iter()
        .map(|entry| entry.external_id.as_str())
        .collect();
    assert_eq!(ids, [B, A]);
    assert_eq!(entries[0].store_label, "2026/09/02");
}

/// The window and the cap are query defaults (grill Q25): older files fall
/// outside, the newest thirty stay, and one id present twice counts once
/// with its newest time.
#[test]
fn the_window_cap_and_dedupe_apply() {
    let home = scratch("window-cap");
    let project = home.join(".claude/projects/-w");
    touch(
        &project.join(format!("{A}.jsonl")),
        now() - Duration::from_secs(15 * 24 * 3_600),
    );
    for i in 1..=35_u32 {
        let id = format!("0f9b6c1a-4444-4444-8444-{i:012}");
        touch(
            &project.join(format!("{id}.jsonl")),
            now() - Duration::from_secs(u64::from(i) * 60),
        );
    }
    let other = home.join(".claude/projects/-x");
    touch(
        &other.join(format!("{C}.jsonl")),
        now() - Duration::from_secs(10),
    );
    touch(
        &project.join(format!("{C}.jsonl")),
        now() - Duration::from_secs(600),
    );

    let entries = enumerate(
        KnownProvider::Claude,
        &store_root(KnownProvider::Claude, &home),
        now(),
        &Recent::default(),
    );
    assert_eq!(entries.len(), 30);
    assert!(
        !entries.iter().any(|entry| entry.external_id.as_str() == A),
        "outside the window"
    );
    assert_eq!(
        entries[0].external_id.as_str(),
        C,
        "the newest copy of a duplicated id wins"
    );
    assert_eq!(
        entries
            .iter()
            .filter(|entry| entry.external_id.as_str() == C)
            .count(),
        1
    );
}

#[test]
fn a_missing_store_is_no_sessions_not_an_error() {
    let home = scratch("missing");
    assert!(
        enumerate(
            KnownProvider::Claude,
            &store_root(KnownProvider::Claude, &home),
            now(),
            &Recent::default()
        )
        .is_empty()
    );
}

/// A store layout is sealed for the versions the matrix actually measured
/// and for no others. The founder's own macOS installs were not exercised
/// (grill Q28), so they inherit nothing: a session that only a sealed
/// version's store could describe is not listed from an unmeasured one.
#[test]
fn a_layout_is_sealed_only_for_the_versions_that_were_measured() {
    assert!(layout_sealed(KnownProvider::Claude, "2.1.258"));
    assert!(layout_sealed(KnownProvider::Claude, "2.1.259"));
    assert!(layout_sealed(KnownProvider::Codex, "0.152.0"));

    assert!(!layout_sealed(KnownProvider::Claude, "2.1.252"));
    assert!(!layout_sealed(KnownProvider::Codex, "0.145.0"));
    assert!(
        !layout_sealed(KnownProvider::Claude, "2.1.260"),
        "a newer version is unmeasured, not assumed"
    );
    assert!(
        !layout_sealed(KnownProvider::Claude, "2.1.257"),
        "a version between two measured ones is not covered by them"
    );
    assert!(!layout_sealed(KnownProvider::Claude, ""));
    assert!(
        !layout_sealed(KnownProvider::Codex, "0.152.0-rc.1"),
        "sealing is an exact match, not a prefix"
    );
}

/// A name in the store that points somewhere else is not a session the
/// provider holds. Following it would enumerate whatever the filesystem can
/// reach from the store and hand it the assurance a history record carries —
/// the sealed layouts describe what a provider writes *under* its store, and
/// that is the whole of what was measured (ADR 0016 D1).
#[test]
fn a_linked_project_or_session_is_outside_the_store_and_is_not_enumerated() {
    let home = scratch("claude-symlink");
    let outside = home.join("elsewhere/-root-other");
    touch(&outside.join(format!("{A}.jsonl")), now());
    touch(&outside.join(format!("{B}.jsonl")), now());

    let projects = home.join(".claude/projects");
    // A real session, so the assertion is about what is refused rather than
    // about an empty store.
    touch(
        &projects.join("-root-proj").join(format!("{C}.jsonl")),
        now(),
    );
    std::os::unix::fs::symlink(&outside, projects.join("-root-linked")).expect("linked project");
    std::os::unix::fs::symlink(
        outside.join(format!("{A}.jsonl")),
        projects.join("-root-proj").join(format!("{B}.jsonl")),
    )
    .expect("linked session");

    let entries = enumerate(
        KnownProvider::Claude,
        &store_root(KnownProvider::Claude, &home),
        now(),
        &Recent::default(),
    );
    let ids: Vec<&str> = entries
        .iter()
        .map(|entry| entry.external_id.as_str())
        .collect();
    assert_eq!(ids, vec![C], "only the file the store itself holds");
}
