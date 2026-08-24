//! The permanent regression floor under the terminal's fuzz requirement.
//!
//! ADR 0003 D9 splits that requirement into three layers with different jobs.
//! This is the second: a deterministic, bounded corpus that every PR must
//! clear. It is not fuzzing — it never generates anything — it is the set of
//! inputs already known to be worth checking, and where every scheduled fuzz
//! finding is distilled to (`docs/decisions/2026-08-24-pr3-plan-grill.md`).
//!
//! What it asserts is narrow on purpose: malformed provider output degrades a
//! session rather than panicking `corrald` (`ARCHITECTURE.md` §5), and it does
//! so in bounded time. It says nothing about what any of these inputs should
//! *render*, because a screen is not what a hostile stream threatens.

// The repository allows unwrap/expect in tests; that setting does not reach
// helpers outside a `#[test]` function in an integration-test crate.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::time::{Duration, Instant};

use corrald::runtime::{AuthoritativeTerminal, PtyGeometry, encode};

const GEOMETRY: PtyGeometry = PtyGeometry { rows: 24, cols: 80 };

/// Generous enough that a slow machine never fails, tight enough that
/// quadratic behaviour on a 100 KB input does.
const PER_CASE_BUDGET: Duration = Duration::from_secs(5);

fn corpus() -> Vec<(String, Vec<u8>)> {
    let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("corpus")
        .join("terminal");

    let mut cases: Vec<(String, Vec<u8>)> = std::fs::read_dir(&directory)
        .unwrap_or_else(|error| {
            panic!(
                "the corpus at {} is unreadable: {error}",
                directory.display()
            )
        })
        .filter_map(Result::ok)
        .filter(|entry| entry.path().extension().is_some_and(|kind| kind == "bin"))
        .map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            let bytes = std::fs::read(entry.path()).expect("a corpus file");
            (name, bytes)
        })
        .collect();
    cases.sort_by(|left, right| left.0.cmp(&right.0));
    cases
}

fn consume_all(bytes: &[u8], chunk: usize) -> AuthoritativeTerminal {
    let mut terminal = AuthoritativeTerminal::new(GEOMETRY);
    for piece in bytes.chunks(chunk.max(1)) {
        let _reply = terminal.consume(piece);
    }
    terminal
}

/// An empty corpus would pass every assertion below while proving nothing, so
/// the suite refuses to be silently disarmed by a deleted directory.
#[test]
fn the_corpus_is_not_empty() {
    assert!(
        corpus().len() >= 20,
        "the terminal corpus has shrunk; a reproducer is only a regression while it is still run"
    );
}

#[test]
fn every_corpus_case_is_consumed_without_panicking() {
    for (name, bytes) in corpus() {
        let started = Instant::now();

        // Fed in chunks, because a PTY delivers whatever the kernel had ready
        // and a parser that only survives whole-message input has not been
        // tested against what it will actually meet.
        let _terminal = consume_all(&bytes, 997);

        assert!(
            started.elapsed() < PER_CASE_BUDGET,
            "{name} took {:?}, past the {PER_CASE_BUDGET:?} budget",
            started.elapsed()
        );
    }
}

/// A screen built from hostile input must be either expressible or refused in
/// so many words. What must never happen is a plausible-looking snapshot of a
/// screen nobody can vouch for.
#[test]
fn every_corpus_case_is_serialized_or_refused_by_name() {
    for (name, bytes) in corpus() {
        let terminal = consume_all(&bytes, 997);

        match encode(&terminal) {
            Ok(snapshot) => assert!(
                !snapshot.payload().is_empty(),
                "{name} produced an empty snapshot"
            ),
            // Two refusals the contract names: a viewport past the ceiling
            // (ADR 0003 D8), and a screen whose parser panicked and may no
            // longer be read at all. Anything else means a failure mode
            // nobody has decided about.
            Err(error) => {
                let stated = error.to_string();
                assert!(
                    stated.contains("ceiling") || stated.contains("can no longer be read"),
                    "{name} failed for a reason the contract does not name: {error}"
                );
            }
        }
    }
}

/// What the pre-merge fuzz campaign found: the emulator truncates an OSC title
/// at 1024 bytes with a raw string slice, so a multi-byte character straddling
/// the cut panics its parser. An agent that sets a long title containing
/// anything but ASCII reaches this — it is not an exotic input.
///
/// Corral contains it rather than repairing it: the screen is marked
/// unreadable and every reader refuses, which is the fail-closed path
/// AGENTS.md §Scope discipline allows while the root cause sits upstream
/// (`docs/evidence/pr3-terminal-fuzz-2026-08-24.md`).
#[test]
fn a_parser_panic_poisons_a_screen_instead_of_taking_the_daemon() {
    let name = "osc-title-truncation-splits-a-character.bin";
    let bytes = corpus()
        .into_iter()
        .find(|(case, _)| case == name)
        .map(|(_, bytes)| bytes)
        .unwrap_or_else(|| panic!("{name} is missing from the corpus"));

    let terminal = consume_all(&bytes, 997);

    assert!(
        terminal.poisoned().is_some(),
        "{name} no longer panics the parser; if upstream fixed it, this test \
         and the containment it guards should be revisited together"
    );
    let refusal = encode(&terminal).expect_err("a poisoned screen is refused");
    assert!(
        refusal.to_string().contains("can no longer be read"),
        "{refusal}"
    );
    assert_eq!(terminal.geometry(), None);
    assert_eq!(terminal.title(), None);
}

/// A poisoned screen never comes back. Feeding it more bytes is not a retry:
/// the structure a panic left behind is not something later input repairs.
#[test]
fn a_poisoned_screen_stays_poisoned() {
    let bytes = corpus()
        .into_iter()
        .find(|(case, _)| case == "osc-title-truncation-splits-a-character.bin")
        .map(|(_, bytes)| bytes)
        .expect("the reproducer is in the corpus");
    let mut terminal = consume_all(&bytes, 997);
    assert!(terminal.poisoned().is_some());

    let reply = terminal.consume(b"\x1b[2Jperfectly ordinary output\r\n");

    assert!(reply.is_empty(), "a poisoned screen answered a query");
    assert!(terminal.poisoned().is_some());
    assert!(encode(&terminal).is_err());
}

/// Resize is where a reflow touches every retained row, so it is where a
/// pathological screen would show up as unbounded work.
#[test]
fn every_corpus_case_survives_a_reflow() {
    for (name, bytes) in corpus() {
        let mut terminal = consume_all(&bytes, 997);

        let started = Instant::now();
        terminal.resize(PtyGeometry {
            rows: 60,
            cols: 200,
        });
        terminal.resize(PtyGeometry { rows: 5, cols: 20 });
        terminal.resize(GEOMETRY);

        assert!(
            started.elapsed() < PER_CASE_BUDGET,
            "{name} reflowed in {:?}, past the {PER_CASE_BUDGET:?} budget",
            started.elapsed()
        );
    }
}

/// Bytes split at every awkward offset, so a case that only survives aligned
/// chunks does not pass by luck. A multi-byte sequence torn across a read is
/// the ordinary case on a PTY, not the exotic one.
#[test]
fn a_corpus_case_survives_being_split_anywhere() {
    for (_name, bytes) in corpus() {
        for split in [1_usize, 2, 3, 7, 13] {
            let terminal = consume_all(&bytes, split);
            let _ = encode(&terminal);
        }
    }
}
