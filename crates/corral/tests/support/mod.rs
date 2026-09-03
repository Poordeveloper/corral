//! Harness for end-to-end tests of the client → daemon path.
//!
//! Every test runs against a private canonical rendezvous, so tests never touch
//! the developer's own account and can run in parallel. That substitution is
//! the test-support input described in ADR 0001; nothing else about activation
//! is faked, because activation is what these tests exist to prove.

#![allow(dead_code)]

// The seam that lets these tests run against a private Corral root exists only
// in a test-support build, so without it the suite would silently drive the
// developer's own daemon.
#[cfg(not(feature = "test-support"))]
compile_error!(
    "the end-to-end suite needs the test-support rendezvous namespace: run ./scripts/verify, \
     or cargo test --features corral/test-support,corrald/test-support"
);

pub mod binaries;
pub mod corpus;
pub mod provider;
pub mod pty;
pub mod wire;

use std::io::Read;
use std::os::unix::fs::DirBuilderExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

/// Built by cargo alongside this test binary — and replaced by cargo whenever
/// something else builds the workspace, which is why no test runs it directly.
/// [`binaries::staged`] is what a test drives.
pub const CARGO_CORRAL_BINARY: &str = env!("CARGO_BIN_EXE_corral");

/// The scripted stand-in a managed provider launch actually runs. No test
/// calls a real provider (`AGENTS.md` §Tests).
pub const MOCK_PROVIDER_BINARY: &str = env!("CARGO_BIN_EXE_corral-mock-provider");

/// How long a test waits for a condition it expects to become true.
pub const SETTLE: Duration = Duration::from_secs(10);

static COUNTER: AtomicU32 = AtomicU32::new(0);

/// The daemon a test drives: the staged copy, resolved as the staged client's
/// sibling exactly the way the product resolves it.
pub fn corrald_binary() -> PathBuf {
    binaries::staged().corrald()
}

/// The client a test drives.
pub fn corral_binary() -> PathBuf {
    binaries::staged().corral()
}

/// A private Corral root standing in for one OS account's rendezvous.
pub struct TestAccount {
    /// Scratch space for fixtures. The Corral root is a directory inside it,
    /// so nothing a test leaves lying around can be mistaken for rendezvous
    /// state.
    base: PathBuf,
    corral_root: PathBuf,
    idle_grace: Duration,
    pre_hello_deadline: Duration,
    activation_deadline: Duration,
    /// A directory placed at the front of the daemon's `PATH`, holding the
    /// stand-in a provider launch resolves to.
    ///
    /// The daemon resolves the provider's program through `PATH`, exactly as a
    /// person's own shell would: Corral integrates the agent the user
    /// installed. A test substitutes the agent rather than the resolution.
    provider_path: Option<PathBuf>,
}

impl TestAccount {
    /// A Unix socket address is limited to about a hundred bytes, and macOS's
    /// per-user `TMPDIR` alone consumes half of it, so the base is short and
    /// absolute rather than `temp_dir()`.
    pub fn new(name: &str) -> Self {
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let short: String = name.chars().take(6).collect();
        let base =
            PathBuf::from("/tmp").join(format!("crl-{}-{unique}-{short}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let corral_root = base.join("corral");
        // Private, like the product's own root: Corral refuses a runtime tree
        // that anyone else can reach into, and a harness that ignored that
        // would be testing something the product never does.
        create_private_dir_all(&corral_root);

        Self {
            base,
            corral_root,
            // Short enough that a daemon a test forgets about disappears on its
            // own, long enough that it does not exit mid-test.
            idle_grace: Duration::from_secs(3),
            pre_hello_deadline: Duration::from_secs(5),
            activation_deadline: Duration::from_secs(10),
            provider_path: None,
        }
    }

    /// Put the scripted stand-in where a managed provider launch will find it.
    pub fn with_mock_provider(mut self, provider: &str) -> Self {
        let bin = self.base.join("bin");
        create_private_dir_all(&bin);
        let stand_in = bin.join(provider);
        let _ = std::fs::remove_file(&stand_in);
        std::fs::copy(MOCK_PROVIDER_BINARY, &stand_in).expect("place the stand-in provider");
        self.provider_path = Some(bin);
        self
    }

    /// The same stand-in, installed the way Claude Code's native installer
    /// lays itself out, so the daemon can read a version from the path it
    /// resolves to. A store layout is sealed per version (ADR 0016 D1), so a
    /// test about enumeration has to be about a version.
    pub fn with_versioned_claude(mut self, version: &str) -> Self {
        let bin = self.base.join("bin");
        create_private_dir_all(&bin);
        let installed = self.base.join("claude/versions").join(version);
        create_private_dir_all(&installed);
        let real = installed.join("claude");
        std::fs::copy(MOCK_PROVIDER_BINARY, &real).expect("place the stand-in provider");
        let link = bin.join("claude");
        let _ = std::fs::remove_file(&link);
        std::os::unix::fs::symlink(&real, &link).expect("link the stand-in onto PATH");
        self.provider_path = Some(bin);
        self
    }

    /// A session file in the provider's own store, as the provider files it.
    /// Content is never read (ADR 0016 D1), so the bytes are a marker.
    pub fn with_claude_history(self, label: &str, session_id: &str) -> Self {
        let directory = self.base.join(".claude/projects").join(label);
        create_private_dir_all(&directory);
        std::fs::write(
            directory.join(format!("{session_id}.jsonl")),
            b"{\"type\":\"user\"}\n",
        )
        .expect("write a session file");
        self
    }

    pub fn with_idle_grace(mut self, idle_grace: Duration) -> Self {
        self.idle_grace = idle_grace;
        self
    }

    pub fn with_pre_hello_deadline(mut self, deadline: Duration) -> Self {
        self.pre_hello_deadline = deadline;
        self
    }

    pub fn with_activation_deadline(mut self, deadline: Duration) -> Self {
        self.activation_deadline = deadline;
        self
    }

    /// The Corral root this account's processes resolve to.
    pub fn corral_root(&self) -> &Path {
        &self.corral_root
    }

    /// Somewhere to put fixtures that are not rendezvous state.
    pub fn scratch(&self) -> &Path {
        &self.base
    }

    pub fn socket(&self) -> PathBuf {
        self.corral_root.join("run/corrald.sock")
    }

    pub fn lock(&self) -> PathBuf {
        self.corral_root.join("run/corrald.lock")
    }

    pub fn log_dir(&self) -> PathBuf {
        self.corral_root.join("log")
    }

    pub fn state_dir(&self) -> PathBuf {
        self.corral_root.join("state")
    }

    pub fn registry(&self) -> PathBuf {
        self.state_dir().join("registry.sqlite3")
    }

    pub fn log(&self) -> PathBuf {
        self.log_dir().join("corrald.log")
    }

    /// A `corral` invocation bound to this account's rendezvous.
    pub fn corral(&self) -> Command {
        let mut command = Command::new(corral_binary());
        self.apply_environment(&mut command);
        command
    }

    /// A `corrald` invocation bound to this account's rendezvous, for the tests
    /// that need to drive the daemon directly.
    pub fn corrald(&self) -> Command {
        let mut command = Command::new(corrald_binary());
        self.apply_environment(&mut command);
        command
    }

    /// A `corral` invocation for a pty, which builds its command differently
    /// from `std::process::Command` and so needs the environment as values.
    pub fn corral_on_pty(&self, arguments: &[&str]) -> portable_pty::CommandBuilder {
        let mut command = portable_pty::CommandBuilder::new(corral_binary());
        for argument in arguments {
            command.arg(argument);
        }
        for (name, value) in self.environment() {
            command.env(name, value);
        }
        command.env_remove("CORRAL_ENDPOINT");
        command
    }

    fn apply_environment(&self, command: &mut Command) {
        for (name, value) in self.environment() {
            command.env(name, value);
        }
        // A stray override in the developer's shell must not decide what a
        // test measures.
        command.env_remove("CORRAL_ENDPOINT");
    }

    /// This account's rendezvous, as the environment that binds a process to
    /// it. One list, so a surface started on a pty and one started as an
    /// ordinary child cannot end up in different accounts.
    fn environment(&self) -> Vec<(&'static str, String)> {
        let mut environment = vec![
            (
                "CORRAL_TEST_ROOT",
                self.corral_root.to_string_lossy().into_owned(),
            ),
            ("CORRAL_TEST_IDLE_GRACE_MS", millis(self.idle_grace)),
            (
                "CORRAL_TEST_PRE_HELLO_DEADLINE_MS",
                millis(self.pre_hello_deadline),
            ),
            (
                "CORRAL_TEST_ACTIVATION_DEADLINE_MS",
                millis(self.activation_deadline),
            ),
        ];
        if let Some(bin) = &self.provider_path {
            let inherited = std::env::var("PATH").unwrap_or_default();
            environment.push(("PATH", format!("{}:{inherited}", bin.to_string_lossy())));
        }
        environment
    }

    /// Start a daemon directly and wait until it is serving.
    pub fn start_daemon(&self) -> DaemonProcess {
        self.start_daemon_with(&[])
    }

    /// The same, carrying extra environment into the daemon.
    ///
    /// A managed provider launch is spawned by the daemon, so what the
    /// stand-in provider reads has to be in the daemon's environment: putting
    /// it on the client would script a process that never runs the provider.
    pub fn start_daemon_with(&self, extra: &[(&str, String)]) -> DaemonProcess {
        let mut command = self.corrald();
        for (name, value) in extra {
            command.env(name, value);
        }
        let child = command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .expect("start corrald");
        let daemon = DaemonProcess { child };
        // Connectable, not merely present. A restart passes through a window
        // where the pathname is the departed daemon's artifact and nothing is
        // listening on it, and a harness that waited on the path alone would
        // hand that window to whatever connected next.
        wait_until(SETTLE, || {
            std::os::unix::net::UnixStream::connect(self.socket()).is_ok()
        });
        daemon
    }
}

impl Drop for TestAccount {
    fn drop(&mut self) {
        // Removing the rendezvous leaves any surviving daemon with nothing to
        // serve; it idles out on the short grace this account configured.
        let _ = std::fs::remove_dir_all(&self.base);
    }
}

/// A daemon a test started directly.
pub struct DaemonProcess {
    child: Child,
}

impl DaemonProcess {
    pub fn pid(&self) -> u32 {
        self.child.id()
    }

    pub fn signal(&self, signal: rustix::process::Signal) {
        let pid = rustix::process::Pid::from_raw(self.pid().cast_signed()).expect("a live pid");
        rustix::process::kill_process(pid, signal).expect("signal the daemon");
    }

    /// Wait for exit and report the status plus whatever it logged.
    pub fn wait(mut self) -> (Option<i32>, String) {
        let mut stderr = String::new();
        if let Some(mut pipe) = self.child.stderr.take() {
            let _ = pipe.read_to_string(&mut stderr);
        }
        let status = self.child.wait().expect("wait for corrald");
        (status.code(), stderr)
    }
}

impl Drop for DaemonProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Create a directory the way the product does: private to its owner.
///
/// Corral refuses a runtime directory that is readable by anyone else, so a
/// test that pre-creates one has to respect the same rule or it is testing
/// the refusal instead of what it meant to test.
pub fn create_private_dir_all(path: &Path) {
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path)
        .expect("create a private directory");
}

fn millis(duration: Duration) -> String {
    duration.as_millis().to_string()
}

/// Run a command to completion.
pub fn run(command: &mut Command) -> Output {
    command.output().expect("run the command")
}

pub fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

pub fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// Poll until the condition holds, or fail the test.
pub fn wait_until(limit: Duration, mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + limit;
    while Instant::now() < deadline {
        if condition() {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("the condition did not hold within {limit:?}");
}

/// Whether a canonical primary daemon holds the lock right now.
pub fn lock_is_held(lock: &Path) -> bool {
    let Ok(file) = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(lock)
    else {
        return false;
    };
    match file.try_lock_shared() {
        Ok(()) => {
            drop(file);
            false
        }
        Err(std::fs::TryLockError::WouldBlock) => true,
        Err(std::fs::TryLockError::Error(source)) => panic!("probing the lock failed: {source}"),
    }
}
