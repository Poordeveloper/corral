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
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("corrald")
        .join("tests")
        .join("corpus")
        .join("terminal")
        .join("osc-title-truncation-splits-a-character.bin")
}
