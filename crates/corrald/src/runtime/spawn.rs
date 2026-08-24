//! Creating a managed process on a PTY, and the facts that creation yields.

use std::io;

use portable_pty::{CommandBuilder, MasterPty, PtySize, native_pty_system};

use super::launch::LaunchRequest;

/// The size of a managed terminal, in cells.
///
/// Pixel dimensions are not carried: nothing in Corral's terminal model reads
/// them, and a number no owner maintains is a number that goes stale.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PtyGeometry {
    pub rows: u16,
    pub cols: u16,
}

impl PtyGeometry {
    fn to_pty_size(self) -> PtySize {
        PtySize {
            rows: self.rows,
            cols: self.cols,
            pixel_width: 0,
            pixel_height: 0,
        }
    }
}

/// Why a validated request still did not become a running process.
///
/// `Exec` is the case the vendored backend patch exists to preserve: the
/// process was never replaced by the requested program. Corral must never
/// record a Run for it, and must never mistake it for a Run that exited.
#[derive(Debug)]
pub enum SpawnError {
    Pty(io::Error),
    Exec(io::Error),
}

/// A process running under Corral's management, with its PTY.
pub struct SpawnedRuntime {
    master: Box<dyn MasterPty + Send>,
    child: Box<dyn portable_pty::Child + Send + Sync>,
    process_id: Option<u32>,
}

/// Start a validated request on a new PTY.
///
/// `TERM` is set for the child, not inherited: the emulator Corral runs is the
/// terminal the child actually talks to, so the child is told what that
/// terminal is (ADR 0003 D1, grill mechanism defaults). This is a property of
/// each managed Run and says nothing about the daemon's own environment, which
/// must never determine daemon identity or lifetime.
pub fn spawn(request: &LaunchRequest, geometry: PtyGeometry) -> Result<SpawnedRuntime, SpawnError> {
    let pair = native_pty_system()
        .openpty(geometry.to_pty_size())
        .map_err(|error| SpawnError::Pty(io::Error::other(error.to_string())))?;

    let mut command = CommandBuilder::new(request.program());
    for arg in request.args() {
        command.arg(arg);
    }
    command.cwd(request.working_directory());
    command.env("TERM", "xterm-256color");

    let child = pair
        .slave
        .spawn_command(command)
        .map_err(|error| SpawnError::Exec(io::Error::other(error.to_string())))?;

    // The slave descriptor's only job was to become the child's controlling
    // terminal. Holding it open here would keep the master readable after the
    // child exits, so the reader would never see EOF.
    drop(pair.slave);

    let process_id = child.process_id();
    Ok(SpawnedRuntime {
        master: pair.master,
        child,
        process_id,
    })
}

impl SpawnedRuntime {
    pub fn process_id(&self) -> Option<u32> {
        self.process_id
    }

    /// The foreground process group of the managed terminal.
    ///
    /// Read from the terminal rather than remembered from the spawn, because
    /// it is what the terminal currently reports: a child that starts its own
    /// job control moves it. Teardown targets this group, which is why Corral
    /// asks the tty rather than assuming the first pid it saw still speaks for
    /// every descendant.
    pub fn process_group_leader(&self) -> Option<i32> {
        self.master.process_group_leader()
    }

    pub fn resize(&self, geometry: PtyGeometry) -> io::Result<()> {
        self.master
            .resize(geometry.to_pty_size())
            .map_err(|error| io::Error::other(error.to_string()))
    }

    pub fn geometry(&self) -> io::Result<PtyGeometry> {
        self.master
            .get_size()
            .map(|size| PtyGeometry {
                rows: size.rows,
                cols: size.cols,
            })
            .map_err(|error| io::Error::other(error.to_string()))
    }

    pub fn reader(&self) -> io::Result<Box<dyn io::Read + Send>> {
        self.master
            .try_clone_reader()
            .map_err(|error| io::Error::other(error.to_string()))
    }

    pub fn writer(&self) -> io::Result<Box<dyn io::Write + Send>> {
        self.master
            .take_writer()
            .map_err(|error| io::Error::other(error.to_string()))
    }

    /// Block until the child exits, yielding its exit code.
    pub fn wait(&mut self) -> io::Result<u32> {
        self.child.wait().map(|status| status.exit_code())
    }
}

/// Hand-written because a `MasterPty` is not `Debug`: what identifies a
/// managed runtime in a diagnostic is the process it holds, not the terminal
/// machinery underneath it.
impl std::fmt::Debug for SpawnedRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SpawnedRuntime")
            .field("process_id", &self.process_id)
            .finish_non_exhaustive()
    }
}

impl std::fmt::Display for SpawnError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Pty(error) => write!(f, "could not allocate a pty: {error}"),
            Self::Exec(error) => write!(f, "the command did not start: {error}"),
        }
    }
}

impl std::error::Error for SpawnError {}

#[cfg(test)]
#[path = "spawn_tests.rs"]
mod tests;
