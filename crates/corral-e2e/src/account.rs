//! A private Corral root standing in for one OS account's rendezvous, and the
//! daemon a test starts under it.

use std::ffi::OsStr;
use std::io::Read;
use std::os::unix::fs::DirBuilderExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use crate::binaries;

/// How long a test waits for a condition it expects to become true.
pub const SETTLE: Duration = Duration::from_secs(10);

/// Where a test namespace puts the home whose provider dotfiles Corral reads
/// and writes: under the Corral root, because that is the directory the
/// namespace seam is set to. A real account keeps them beside `.corral`
/// instead, which is the one layout difference a test namespace has.
const PROVIDER_HOME: &str = "provider-home";

static COUNTER: AtomicU32 = AtomicU32::new(0);

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
    /// Directories placed at the front of every process's `PATH`.
    ///
    /// A daemon resolves a provider's program through `PATH`, exactly as a
    /// person's own shell would: Corral integrates the agent the user
    /// installed. A test substitutes the agent rather than the resolution.
    path_entries: Vec<PathBuf>,
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
            path_entries: Vec::new(),
        }
    }

    /// Put a directory in front of `PATH` for every process of this account.
    pub fn with_path_entry(mut self, directory: PathBuf) -> Self {
        self.path_entries.push(directory);
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

    /// The home this account's daemon reads and writes provider files in,
    /// as `corral_rendezvous::provider_home` resolves it under the test
    /// namespace.
    pub fn provider_home(&self) -> PathBuf {
        self.corral_root.join(PROVIDER_HOME)
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

    /// A `corrald` invocation bound to this account's rendezvous, for the tests
    /// that need to drive the daemon directly. The staged daemon, never the
    /// one cargo holds.
    pub fn corrald(&self) -> Command {
        self.command(binaries::staged().corrald())
    }

    /// Any program, bound to this account's rendezvous.
    pub fn command(&self, program: impl AsRef<OsStr>) -> Command {
        let mut command = Command::new(program);
        for (name, value) in self.environment() {
            command.env(name, value);
        }
        // A stray override in the developer's shell must not decide what a
        // test measures.
        command.env_remove("CORRAL_ENDPOINT");
        command
    }

    /// This account's rendezvous, as the environment that binds a process to
    /// it. One list, so a surface started on a pty and one started as an
    /// ordinary child cannot end up in different accounts.
    pub fn environment(&self) -> Vec<(&'static str, String)> {
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
        if !self.path_entries.is_empty() {
            let inherited = std::env::var("PATH").unwrap_or_default();
            let mut path: Vec<String> = self
                .path_entries
                .iter()
                .map(|entry| entry.to_string_lossy().into_owned())
                .collect();
            path.push(inherited);
            environment.push(("PATH", path.join(":")));
        }
        environment
    }

    /// Start a daemon directly and wait until it is serving.
    pub fn start_daemon(&self) -> DaemonProcess {
        self.start_daemon_with(&[])
    }

    /// The same, carrying extra environment into the daemon.
    ///
    /// A managed provider launch is spawned by the daemon, so what a stand-in
    /// provider reads has to be in the daemon's environment: putting it on the
    /// client would script a process that never runs the provider.
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
