//! What an installed Corral is on disk, and whether the running executable
//! is part of one.
//!
//! Two shapes, both hanging off the user's home as Corral resolves it
//! (`corral_rendezvous::user_home`):
//!
//! ```text
//! macOS  ~/Applications/Corral.app/Contents/MacOS/{corral-desktop,corrald,corral}
//! Linux  ~/.local/share/corral/bin/{corral,corrald,corral-desktop}
//! both   ~/.local/bin/corral -> the installed corral
//! ```
//!
//! Recognition is by pathname and nothing else: the canonicalized running
//! executable either is the `corral` of one of these shapes under this home,
//! or this process is not an installation — a `target/` build, a copied
//! binary — and whatever asked is told so by name. The three executables are
//! siblings because the daemon and the hook relay are resolved as siblings
//! (`corral-client::spawn`, `corrald::provider::launch`), so the directory
//! is what an installation is.

use std::fmt;
use std::path::{Path, PathBuf};

/// The executable an installation is recognized by.
const CLIENT: &str = "corral";

/// Where the CLI is reachable from `PATH`, under both shapes.
const SYMLINK: &str = ".local/bin/corral";

/// One shape: the executables' directory and the directory removal takes
/// away, both relative to the home.
///
/// Both shapes are recognized on every platform. A shape is a pathname, not
/// platform behaviour, and one machine's tests exercise both.
struct Shape {
    executables: &'static str,
    root: &'static str,
}

const SHAPES: [Shape; 2] = [
    Shape {
        executables: "Applications/Corral.app/Contents/MacOS",
        root: "Applications/Corral.app",
    },
    Shape {
        executables: ".local/share/corral/bin",
        root: ".local/share/corral",
    },
];

/// The installation the running executable belongs to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Installation {
    root: PathBuf,
    corral: PathBuf,
    symlink: PathBuf,
}

/// Why the running executable is not part of an installation.
#[derive(Debug)]
pub enum NotAnInstallation {
    /// The executable or the home could not be resolved at all.
    Unlocatable(String),
    /// Resolved, and not the `corral` of either shape under this home.
    Elsewhere { executable: PathBuf, home: PathBuf },
}

impl Installation {
    /// The installation this process runs from, or why there is none.
    pub fn of_running_executable() -> Result<Self, NotAnInstallation> {
        let running = std::env::current_exe().map_err(|source| {
            NotAnInstallation::Unlocatable(format!(
                "the running executable could not be located: {source}"
            ))
        })?;
        // Through the symlink, the way a shell reached it: what is recognized
        // is where the bytes are, never the name they were invoked by.
        let real = running.canonicalize().map_err(|source| {
            NotAnInstallation::Unlocatable(format!(
                "{} could not be resolved: {source}",
                running.display()
            ))
        })?;
        let home = corral_rendezvous::user_home()
            .map_err(|source| NotAnInstallation::Unlocatable(source.to_string()))?;
        // Resolved the same way as the executable, so a home reached through
        // a link — `/tmp` on macOS is one — compares equal to a path resolved
        // beneath it. A home that cannot be resolved is compared as named.
        let home = home.canonicalize().unwrap_or(home);
        Self::recognize(&real, &home)
    }

    /// Which shape, if any, puts `corral` at `executable` under `home`.
    pub(crate) fn recognize(executable: &Path, home: &Path) -> Result<Self, NotAnInstallation> {
        for shape in SHAPES {
            let corral = home.join(shape.executables).join(CLIENT);
            if executable == corral {
                return Ok(Self {
                    root: home.join(shape.root),
                    corral,
                    symlink: home.join(SYMLINK),
                });
            }
        }
        Err(NotAnInstallation::Elsewhere {
            executable: executable.to_path_buf(),
            home: home.to_path_buf(),
        })
    }

    /// The directory removal takes away: the bundle, or the Linux tree.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The installed `corral`, resolved.
    pub fn corral(&self) -> &Path {
        &self.corral
    }

    /// Where `PATH` reaches the installed `corral`.
    pub fn symlink(&self) -> &Path {
        &self.symlink
    }
}

impl fmt::Display for NotAnInstallation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unlocatable(detail) => f.write_str(detail),
            Self::Elsewhere { executable, home } => write!(
                f,
                "{} is not an installed Corral; an installation is {} or {}",
                executable.display(),
                home.join(SHAPES[0].executables).join(CLIENT).display(),
                home.join(SHAPES[1].executables).join(CLIENT).display(),
            ),
        }
    }
}

#[cfg(test)]
#[path = "installation_tests.rs"]
mod tests;
