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

/// A screen built from hostile input must still be expressible, or a client
/// could be locked out of a session precisely when something has gone wrong.
#[test]
fn every_corpus_case_still_yields_a_snapshot() {
    for (name, bytes) in corpus() {
        let terminal = consume_all(&bytes, 997);

        match encode(&terminal) {
            Ok(snapshot) => assert!(
                !snapshot.payload().is_empty(),
                "{name} produced an empty snapshot"
            ),
            // A refusal is a legitimate answer — it is the typed failure ADR
            // 0003 D8 requires — but it must be a refusal, not a panic.
            Err(error) => assert!(
                error.to_string().contains("ceiling"),
                "{name} failed for a reason the contract does not name: {error}"
            ),
        }
    }
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
