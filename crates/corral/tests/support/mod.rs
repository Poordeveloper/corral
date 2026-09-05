//! Harness for end-to-end tests of the client → daemon path.
//!
//! Every test runs against a private canonical rendezvous, so tests never touch
//! the developer's own account and can run in parallel. That substitution is
//! the test-support input described in ADR 0001; nothing else about activation
//! is faked, because activation is what these tests exist to prove.

// A shared toolbox: not every test binary uses every re-exported helper,
// which is what these allows acknowledge (the inline version relied on
// dead_code alone).
#![allow(dead_code, unused_imports)]

// The seam that lets these tests run against a private Corral root exists only
// in a test-support build, so without it the suite would silently drive the
// developer's own daemon.
#[cfg(not(feature = "test-support"))]
compile_error!(
    "the end-to-end suite needs the test-support rendezvous namespace: run ./scripts/verify, \
     or cargo test --features corral/test-support,corrald/test-support"
);

pub mod corpus;
pub mod provider;
pub mod pty;
pub mod wire;

use std::ops::{Deref, DerefMut};
use std::path::PathBuf;

pub use corral_e2e::binaries;
pub use corral_e2e::{
    DaemonProcess, SETTLE, create_private_dir_all, lock_is_held, run, stderr, stdout, wait_until,
};

/// Built by cargo alongside this test binary — and replaced by cargo whenever
/// something else builds the workspace, which is why no test runs it directly.
/// [`binaries::staged`] is what a test drives.
pub const CARGO_CORRAL_BINARY: &str = env!("CARGO_BIN_EXE_corral");

/// The scripted stand-in a managed provider launch actually runs. No test
/// calls a real provider (`AGENTS.md` §Tests).
pub const MOCK_PROVIDER_BINARY: &str = env!("CARGO_BIN_EXE_corral-mock-provider");

/// The daemon a test drives: the staged copy, resolved as the staged client's
/// sibling exactly the way the product resolves it.
pub fn corrald_binary() -> PathBuf {
    binaries::staged().corrald()
}

/// The client a test drives.
pub fn corral_binary() -> PathBuf {
    binaries::staged().corral()
}

/// The shared account, plus what only this suite adds to it: the stand-in
/// provider, the provider's own history, and the `corral` client itself.
pub struct TestAccount(corral_e2e::TestAccount);

impl Deref for TestAccount {
    type Target = corral_e2e::TestAccount;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for TestAccount {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl TestAccount {
    pub fn new(name: &str) -> Self {
        Self(corral_e2e::TestAccount::new(name))
    }

    /// Put the scripted stand-in where a managed provider launch will find it.
    pub fn with_mock_provider(self, provider: &str) -> Self {
        let bin = self.scratch().join("bin");
        create_private_dir_all(&bin);
        let stand_in = bin.join(provider);
        let _ = std::fs::remove_file(&stand_in);
        std::fs::copy(MOCK_PROVIDER_BINARY, &stand_in).expect("place the stand-in provider");
        Self(self.0.with_path_entry(bin))
    }

    /// The same stand-in, installed the way Claude Code's native installer
    /// lays itself out, so the daemon can read a version from the path it
    /// resolves to. A store layout is sealed per version (ADR 0016 D1), so a
    /// test about enumeration has to be about a version.
    pub fn with_versioned_claude(self, version: &str) -> Self {
        let bin = self.scratch().join("bin");
        create_private_dir_all(&bin);
        let installed = self.scratch().join("claude/versions").join(version);
        create_private_dir_all(&installed);
        let real = installed.join("claude");
        std::fs::copy(MOCK_PROVIDER_BINARY, &real).expect("place the stand-in provider");
        let link = bin.join("claude");
        let _ = std::fs::remove_file(&link);
        std::os::unix::fs::symlink(&real, &link).expect("link the stand-in onto PATH");
        Self(self.0.with_path_entry(bin))
    }

    /// A session file in the provider's own store, as the provider files it.
    /// Content is never read (ADR 0016 D1), so the bytes are a marker.
    ///
    /// Under the provider home, which is the one home Corral reads and writes
    /// a provider's own files in — the same place the hook installer works —
    /// so a test's layout is the layout production has.
    pub fn with_claude_history(self, label: &str, session_id: &str) -> Self {
        let directory = self.provider_home().join(".claude/projects").join(label);
        create_private_dir_all(&directory);
        std::fs::write(
            directory.join(format!("{session_id}.jsonl")),
            b"{\"type\":\"user\"}\n",
        )
        .expect("write a session file");
        self
    }

    pub fn with_idle_grace(self, idle_grace: std::time::Duration) -> Self {
        Self(self.0.with_idle_grace(idle_grace))
    }

    pub fn with_pre_hello_deadline(self, deadline: std::time::Duration) -> Self {
        Self(self.0.with_pre_hello_deadline(deadline))
    }

    pub fn with_activation_deadline(self, deadline: std::time::Duration) -> Self {
        Self(self.0.with_activation_deadline(deadline))
    }

    /// A `corral` invocation bound to this account's rendezvous.
    pub fn corral(&self) -> std::process::Command {
        self.0.command(corral_binary())
    }

    /// A `corral` invocation for a pty, which builds its command differently
    /// from `std::process::Command` and so needs the environment as values.
    pub fn corral_on_pty(&self, arguments: &[&str]) -> portable_pty::CommandBuilder {
        let mut command = portable_pty::CommandBuilder::new(corral_binary());
        for argument in arguments {
            command.arg(argument);
        }
        for (name, value) in self.0.environment() {
            command.env(name, value);
        }
        command.env_remove("CORRAL_ENDPOINT");
        command
    }
}
