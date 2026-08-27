//! The fuzz corpus, addressed once.
//!
//! Two tests drive a session through the same reproducer, and a third copy of
//! the walk to it is a third place to fix when the corpus moves.

use std::path::{Path, PathBuf};

/// Output that poisons the daemon's terminal parser.
///
/// The reproducer the pre-merge fuzz campaign distilled, read from the corpus
/// it lives in rather than copied: two files carrying the same bytes would be
/// one file too many to keep true
/// (`docs/evidence/pr3-terminal-fuzz-2026-08-24.md`).
pub fn poisoning_input() -> PathBuf {
    let reproducer = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("corrald")
        .join("tests")
        .join("corpus")
        .join("terminal")
        .join("osc-title-truncation-splits-a-character.bin");

    // Checked here, because the tests that use it feed the path to `cat` in a
    // session: a corpus entry that moved would make `cat` write to stderr, the
    // screen would never be poisoned, and the failure would arrive as an
    // assertion blaming the daemon for serving a screen it cannot read.
    assert!(
        reproducer.is_file(),
        "{} is missing; the tests that need it cannot say so for themselves",
        reproducer.display()
    );

    reproducer
}
