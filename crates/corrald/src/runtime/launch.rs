//! What a launch request must satisfy before any process exists.
//!
//! Validation lives here rather than in the PTY backend for two reasons. The
//! backend silently substitutes `$HOME` for a working directory that is not a
//! directory, which answers a question nobody asked; and a rejection Corral
//! makes itself is a typed domain fact, not a diagnostic recovered from a
//! child's exit. Deliberately not patched in the vendored crate so the
//! vendored delta stays one hunk (grill Q1).

use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// A request to start a process under Corral's management.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LaunchRequest {
    program: OsString,
    args: Vec<OsString>,
    working_directory: PathBuf,
}

/// Why a launch request cannot become a process.
///
/// Each variant is a refusal made before anything is spawned, so none of them
/// can be confused with a Run that existed. `NotADirectory` is separate from
/// `WorkingDirectoryMissing` because the two are different user mistakes and
/// the backend would silently paper over both.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LaunchRejection {
    EmptyProgram,
    WorkingDirectoryMissing(PathBuf),
    WorkingDirectoryNotADirectory(PathBuf),
}

impl LaunchRequest {
    /// Validate a request, or refuse it before any process can exist.
    ///
    /// The executable itself is deliberately not probed here: any check would
    /// be a TOCTOU guess, and the backend reports a missing or non-executable
    /// program as a spawn error already. What cannot be recovered after the
    /// fact — a working directory quietly replaced by `$HOME` — is what this
    /// refuses.
    pub fn new(
        program: impl Into<OsString>,
        args: impl IntoIterator<Item = OsString>,
        working_directory: impl AsRef<Path>,
    ) -> Result<Self, LaunchRejection> {
        let program = program.into();
        if program.is_empty() {
            return Err(LaunchRejection::EmptyProgram);
        }

        let working_directory = working_directory.as_ref();
        match std::fs::metadata(working_directory) {
            Err(_) => {
                return Err(LaunchRejection::WorkingDirectoryMissing(
                    working_directory.to_path_buf(),
                ));
            }
            Ok(metadata) if !metadata.is_dir() => {
                return Err(LaunchRejection::WorkingDirectoryNotADirectory(
                    working_directory.to_path_buf(),
                ));
            }
            Ok(_) => {}
        }

        Ok(Self {
            program,
            args: args.into_iter().collect(),
            working_directory: working_directory.to_path_buf(),
        })
    }

    pub fn program(&self) -> &OsString {
        &self.program
    }

    pub fn args(&self) -> &[OsString] {
        &self.args
    }

    pub fn working_directory(&self) -> &Path {
        &self.working_directory
    }

    /// The display label for this launch.
    ///
    /// The executable's basename, never the joined argv: argv routinely
    /// carries tokens, passwords, URLs, and customer identifiers, and a list
    /// needing one line of text is no reason to spread a command line into
    /// `session.list`, logs, and screenshots (grill Q3).
    pub fn display_title(&self) -> String {
        Path::new(&self.program)
            .file_name()
            .unwrap_or(self.program.as_os_str())
            .to_string_lossy()
            .into_owned()
    }
}

impl std::fmt::Display for LaunchRejection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyProgram => f.write_str("no program was given"),
            Self::WorkingDirectoryMissing(path) => {
                write!(f, "working directory does not exist: {}", path.display())
            }
            Self::WorkingDirectoryNotADirectory(path) => {
                write!(
                    f,
                    "working directory is not a directory: {}",
                    path.display()
                )
            }
        }
    }
}

impl std::error::Error for LaunchRejection {}

#[cfg(test)]
#[path = "launch_tests.rs"]
mod tests;
