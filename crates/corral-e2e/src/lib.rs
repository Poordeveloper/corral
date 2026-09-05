#![forbid(unsafe_code)]
// A harness fails loudly at the point a precondition breaks; a `Result` nobody
// handles is the wrong shape for it. Every test file that used this code
// allowed the same two lints for the same reason.
#![allow(clippy::expect_used, clippy::unwrap_used)]

//! The isolation contract every end-to-end test of a Corral surface inherits.
//!
//! Every test runs against a private canonical rendezvous, so tests never touch
//! the developer's own account and can run in parallel. That substitution is
//! the test-support input described in ADR 0001; nothing else about activation
//! is faked, because activation is what those tests exist to prove.
//!
//! The daemon a test drives is a staged, validated test-support build
//! ([`binaries`]), never whatever cargo currently holds under `target/`: an
//! ordinary `cargo build` beside a running suite replaces that binary with a
//! production one, which ignores the test root and serves the developer's real
//! account (`./scripts/verify`, 2026-09-02). One crate holds the contract so a
//! second surface's tests — the Desktop's bridge, after the CLI's — cannot
//! inherit it partially.

mod account;
pub mod binaries;

pub use account::{
    DaemonProcess, SETTLE, TestAccount, create_private_dir_all, lock_is_held, run, stderr, stdout,
    wait_until,
};
