//! The end-to-end harness must fail rather than reach the developer's own
//! Corral, and must not be redirected by whatever cargo writes mid-run.
//!
//! `./scripts/verify` observed the second of those on 2026-09-02: an ordinary
//! `cargo build -p corral` running beside the suite replaced
//! `target/debug/corral` with a production binary, which resolved the real
//! account and started a daemon under `~/.corral`. These tests hold the
//! harness to the invariant the accident broke.

#![allow(clippy::expect_used, clippy::unwrap_used)]

mod support;

use support::binaries::{self, NotStaged};

/// A binary that is not a test-support build is refused before anything runs,
/// naming which one it was — the developer is looking at a stale
/// `target/debug`, not at a bug in the test.
#[test]
fn a_binary_without_the_test_support_seam_is_refused_before_it_runs() {
    let scratch = scratch("refuses-production");
    let source = scratch.join("source");
    std::fs::create_dir_all(&source).expect("create the source directory");
    write(&source.join("corral"), b"a build carrying CORRAL_TEST_ROOT");
    write(&source.join("corrald"), b"a production build, no seam");

    let refusal = binaries::stage(&source, &scratch.join("staged")).expect_err("refused");

    let NotStaged::NotTestSupport { path } = &refusal else {
        panic!("{refusal}");
    };
    assert!(path.ends_with("corrald"), "{}", path.display());
    assert!(
        !scratch.join("staged").exists(),
        "a refused pair was staged anyway"
    );
}

/// The point of staging: once a test run holds its binaries, rebuilding the
/// ones cargo owns cannot reach them. This is the accident, in miniature.
#[test]
fn a_rebuild_of_the_source_cannot_reach_a_staged_binary() {
    let scratch = scratch("rebuild-cannot-reach");
    let source = scratch.join("source");
    std::fs::create_dir_all(&source).expect("create the source directory");
    write(&source.join("corral"), b"client CORRAL_TEST_ROOT build");
    write(&source.join("corrald"), b"daemon CORRAL_TEST_ROOT build");

    let staged = binaries::stage(&source, &scratch.join("staged")).expect("staged");

    // What `cargo build -p corral` does to `target/debug` while a suite runs.
    write(&source.join("corral"), b"production client");
    write(&source.join("corrald"), b"production daemon");

    let image = std::fs::read(staged.corral()).expect("read the staged client");
    assert!(
        contains(&image, b"CORRAL_TEST_ROOT"),
        "the staged client followed the rebuild"
    );
    let image = std::fs::read(staged.corrald()).expect("read the staged daemon");
    assert!(
        contains(&image, b"CORRAL_TEST_ROOT"),
        "the staged daemon followed the rebuild"
    );
    // And re-validating them says so, rather than trusting the earlier answer.
    binaries::stage(&source, &scratch.join("staged")).expect("the staged pair still validates");
}

/// The suite runs what it staged, not what cargo is holding.
#[test]
fn the_account_runs_the_staged_binaries() {
    let account = support::TestAccount::new("staged-binaries");
    let program = account.corral().get_program().to_owned();

    assert_ne!(
        std::path::Path::new(&program),
        std::path::Path::new(support::CARGO_CORRAL_BINARY),
        "the suite ran cargo's own binary, which a concurrent build can replace"
    );
    assert_eq!(
        std::path::Path::new(&program),
        binaries::staged().corral(),
        "the client is not the staged one"
    );
    assert_eq!(
        support::corrald_binary(),
        binaries::staged().corrald(),
        "the daemon is not the staged one"
    );
}

fn scratch(name: &str) -> std::path::PathBuf {
    let base =
        std::path::PathBuf::from("/tmp").join(format!("crl-harness-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).expect("create the scratch directory");
    base
}

fn write(path: &std::path::Path, bytes: &[u8]) {
    std::fs::write(path, bytes).expect("write a stand-in binary");
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}
