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

pub mod pty;
pub mod wire;

use std::io::Read;
use std::os::unix::fs::DirBuilderExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

/// Built by cargo alongside this test binary.
pub const CORRAL_BINARY: &str = env!("CARGO_BIN_EXE_corral");

/// How long a test waits for a condition it expects to become true.
pub const SETTLE: Duration = Duration::from_secs(10);

static COUNTER: AtomicU32 = AtomicU32::new(0);

/// The daemon, resolved exactly the way the product resolves it: as `corral`'s
/// sibling. Cargo puts both binaries in the same directory.
///
/// The seam check is not ceremony: a `corrald` built without `test-support`
/// resolves the developer's real account home instead of this test's root, so
/// the suite would quietly drive their own daemon. Failing loudly here is the
/// difference between a red test and a damaged machine.
pub fn corrald_binary() -> PathBuf {
    static DAEMON: OnceLock<PathBuf> = OnceLock::new();
    DAEMON
        .get_or_init(|| {
            let directory = Path::new(CORRAL_BINARY)
                .parent()
                .expect("the corral binary has a directory");
            let daemon = directory.join("corrald");
            let image = std::fs::read(&daemon).unwrap_or_else(|source| {
                panic!(
                    "{} could not be read ({source}); build the whole workspace, which the \
                     merge gate does",
                    daemon.display()
                )
            });
            assert!(
                contains(&image, b"CORRAL_TEST_ROOT"),
                "{} was built without the test-support rendezvous seam and would serve the \
                 real account; rebuild with --features corral/test-support,corrald/test-support",
                daemon.display()
            );
            daemon
        })
        .clone()
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
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

    /// A `corral` invocation for a pty, which builds its command differently
    /// from `std::process::Command` and so needs the environment as values.
    pub fn corral_on_pty(&self, arguments: &[&str]) -> portable_pty::CommandBuilder {
        let mut command = portable_pty::CommandBuilder::new(CORRAL_BINARY);
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
        vec![
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
        ]
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
