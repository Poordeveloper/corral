//! Harness for end-to-end tests of the client → daemon path.
//!
//! Every test runs against a private canonical rendezvous, so tests never touch
//! the developer's own account and can run in parallel. That substitution is
//! the test-support input described in ADR 0001; nothing else about activation
//! is faked, because activation is what these tests exist to prove.

#![allow(dead_code)]

pub mod wire;

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

/// Built by cargo alongside this test binary.
pub const CORRAL_BINARY: &str = env!("CARGO_BIN_EXE_corral");

/// How long a test waits for a condition it expects to become true.
pub const SETTLE: Duration = Duration::from_secs(10);

static COUNTER: AtomicU32 = AtomicU32::new(0);

/// The daemon, resolved exactly the way the product resolves it: as `corral`'s
/// sibling. Cargo puts both binaries in the same directory.
pub fn corrald_binary() -> PathBuf {
    let directory = Path::new(CORRAL_BINARY)
        .parent()
        .expect("the corral binary has a directory");
    let daemon = directory.join("corrald");
    assert!(
        daemon.exists(),
        "{} is missing; build the whole workspace (the merge gate runs \
         `cargo test --workspace --all-targets`)",
        daemon.display()
    );
    daemon
}

/// A private OS-account home standing in for one user's canonical rendezvous.
pub struct TestAccount {
    root: PathBuf,
    idle_grace: Duration,
    pre_hello_deadline: Duration,
    activation_deadline: Duration,
}

impl TestAccount {
    /// A Unix socket address is limited to about a hundred bytes, and macOS's
    /// per-user `TMPDIR` alone consumes half of it, so the base is short and
    /// absolute rather than `temp_dir()`.
    pub fn new(name: &str) -> Self {
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let short: String = name.chars().take(6).collect();
        let root =
            PathBuf::from("/tmp").join(format!("crl-{}-{unique}-{short}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("create the test account home");

        Self {
            root,
            // Short enough that a daemon a test forgets about disappears on its
            // own, long enough that it does not exit mid-test.
            idle_grace: Duration::from_secs(3),
            pre_hello_deadline: Duration::from_secs(5),
            activation_deadline: Duration::from_secs(10),
        }
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

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn socket(&self) -> PathBuf {
        self.root.join(".corral/run/corrald.sock")
    }

    pub fn lock(&self) -> PathBuf {
        self.root.join(".corral/run/corrald.lock")
    }

    pub fn log(&self) -> PathBuf {
        self.root.join(".corral/log/corrald.log")
    }

    /// A `corral` invocation bound to this account's rendezvous.
    pub fn corral(&self) -> Command {
        let mut command = Command::new(CORRAL_BINARY);
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

    fn apply_environment(&self, command: &mut Command) {
        command
            .env("CORRAL_TEST_ROOT", &self.root)
            .env("CORRAL_TEST_IDLE_GRACE_MS", millis(self.idle_grace))
            .env(
                "CORRAL_TEST_PRE_HELLO_DEADLINE_MS",
                millis(self.pre_hello_deadline),
            )
            .env(
                "CORRAL_TEST_ACTIVATION_DEADLINE_MS",
                millis(self.activation_deadline),
            )
            // A stray override in the developer's shell must not decide what a
            // test measures.
            .env_remove("CORRAL_ENDPOINT");
    }

    /// Start a daemon directly and wait until it is serving.
    pub fn start_daemon(&self) -> DaemonProcess {
        let child = self
            .corrald()
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .expect("start corrald");
        let daemon = DaemonProcess { child };
        wait_until(SETTLE, || self.socket().exists());
        daemon
    }
}

impl Drop for TestAccount {
    fn drop(&mut self) {
        // Removing the rendezvous leaves any surviving daemon with nothing to
        // serve; it idles out on the short grace this account configured.
        let _ = std::fs::remove_dir_all(&self.root);
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
