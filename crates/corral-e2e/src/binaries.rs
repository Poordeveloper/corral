//! The binaries an end-to-end test runs, staged where cargo cannot reach them.
//!
//! `target/debug/corral` is a moving target: an ordinary `cargo build -p
//! corral` beside a running suite replaces it with a production build, which
//! ignores `CORRAL_TEST_ROOT` and resolves the developer's own account. That
//! happened under `./scripts/verify` on 2026-09-02 — the suite started a
//! daemon under `~/.corral`. Validating the binaries once at startup is not
//! enough, because the swap can land after the check and before the spawn.
//!
//! So the suite copies both binaries out of cargo's way, validates the bytes
//! it wrote, and runs those. The staging directory is keyed by the identity of
//! the sources, so one copy serves every test process of a build and a rebuilt
//! source stages afresh (and is refused if it is not a test-support build).
//! It lives under the target directory, which is where a `cargo clean`
//! reaches.
//!
//! What is staged is what `./scripts/verify` built: a plain `cargo test -p
//! <surface>` finds a missing or production daemon here and fails before
//! anything is spawned, naming the build it needs.

use std::hash::{DefaultHasher, Hash, Hasher};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// The environment variable the rendezvous namespace seam reads. It exists
/// only in a `test-support` build — `scripts/check-test-support-boundary`
/// keeps it out of production binaries — so its presence in an image is the
/// build's own answer to "were you built for this suite".
const SEAM: &[u8] = b"CORRAL_TEST_ROOT";

const STAGED: [&str; 2] = ["corral", "corrald"];

/// The validated pair a test run drives.
#[derive(Debug)]
pub struct Staged {
    directory: PathBuf,
}

impl Staged {
    pub fn corral(&self) -> PathBuf {
        self.directory.join("corral")
    }

    /// The daemon, resolved as the client's sibling exactly the way the
    /// product resolves it. Staging both together is what keeps that true.
    pub fn corrald(&self) -> PathBuf {
        self.directory.join("corrald")
    }
}

#[derive(Debug)]
pub enum NotStaged {
    Unreadable {
        path: PathBuf,
        detail: String,
    },
    /// Built without the rendezvous seam: this binary would serve the
    /// developer's real account.
    NotTestSupport {
        path: PathBuf,
    },
}

impl std::fmt::Display for NotStaged {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unreadable { path, detail } => {
                write!(f, "{} could not be staged: {detail}", path.display())
            }
            Self::NotTestSupport { path } => write!(
                f,
                "{} was built without the test-support rendezvous seam and would serve the real \
                 account; build the workspace with --features corral/test-support,\
                 corrald/test-support, which ./scripts/verify does",
                path.display()
            ),
        }
    }
}

/// The staged pair for this test process, built once.
pub fn staged() -> &'static Staged {
    static STAGE: OnceLock<Staged> = OnceLock::new();
    STAGE.get_or_init(|| {
        let source = cargo_output_dir().unwrap_or_else(|refusal| panic!("{refusal}"));
        let into = staging_root(&source).join(fingerprint(&source));
        stage(&source, &into).unwrap_or_else(|refusal| panic!("{refusal}"))
    })
}

/// Where cargo put the workspace binaries this test process belongs to: the
/// profile directory above the `deps/` every test executable lives in.
///
/// Resolved from the executable rather than from a `CARGO_BIN_EXE_*` variable,
/// which cargo sets only for the package's own binaries — and the daemon is
/// never that for a surface's tests. A process that is not a cargo test
/// executable is refused rather than guessed about.
fn cargo_output_dir() -> Result<PathBuf, NotStaged> {
    let exe = std::env::current_exe().map_err(|error| NotStaged::Unreadable {
        path: PathBuf::from("<current executable>"),
        detail: error.to_string(),
    })?;
    let deps = exe
        .parent()
        .filter(|deps| deps.file_name().is_some_and(|name| name == "deps"));
    deps.and_then(Path::parent)
        .map(Path::to_path_buf)
        .ok_or_else(|| NotStaged::Unreadable {
            path: exe.clone(),
            detail: "not a cargo test executable under a deps/ directory, so the workspace \
                     binaries cannot be located"
                .to_owned(),
        })
}

/// Copy `corral` and `corrald` out of `from` into `into`, refusing either that
/// was not built for this suite. An `into` that already holds a staged pair is
/// re-validated rather than rebuilt, so concurrent test processes share one
/// copy and none of them trusts an answer it did not check.
pub fn stage(from: &Path, into: &Path) -> Result<Staged, NotStaged> {
    if into.is_dir() {
        return validate(into);
    }
    let partial = into.with_file_name(format!(
        "{}.partial.{}",
        into.file_name().unwrap_or_default().to_string_lossy(),
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&partial);
    create_dir_all(&partial)?;

    for name in STAGED {
        let source = from.join(name);
        let image = std::fs::read(&source).map_err(|error| NotStaged::Unreadable {
            path: source.clone(),
            detail: error.to_string(),
        })?;
        if !contains(&image, SEAM) {
            let _ = std::fs::remove_dir_all(&partial);
            return Err(NotStaged::NotTestSupport { path: source });
        }
        let destination = partial.join(name);
        write_executable(&destination, &image)?;
    }

    if let Err(error) = std::fs::rename(&partial, into) {
        let _ = std::fs::remove_dir_all(&partial);
        if !into.is_dir() {
            return Err(NotStaged::Unreadable {
                path: into.to_path_buf(),
                detail: error.to_string(),
            });
        }
        // Another test process staged the same build first. Its copy is
        // validated below like any other.
    }
    prune_stale(into);
    validate(into)
}

/// Every rebuild stages a new directory, so without this a developer's target
/// tree grows by a copy of the workspace binaries per build. Only pairs older
/// than a working session go, because a shorter horizon could delete a pair a
/// concurrently running suite is executing from.
fn prune_stale(keep: &Path) {
    const STALE: std::time::Duration = std::time::Duration::from_secs(6 * 60 * 60);
    let Some(root) = keep.parent() else {
        return;
    };
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path == keep || !path.is_dir() {
            continue;
        }
        let stale = entry
            .metadata()
            .and_then(|metadata| metadata.modified())
            .map(|modified| modified.elapsed().unwrap_or_default() > STALE)
            .unwrap_or(false);
        if stale {
            let _ = std::fs::remove_dir_all(&path);
        }
    }
}

/// Read back what is on disk and check it, rather than trusting that whoever
/// wrote it did.
fn validate(directory: &Path) -> Result<Staged, NotStaged> {
    for name in STAGED {
        let path = directory.join(name);
        let image = std::fs::read(&path).map_err(|error| NotStaged::Unreadable {
            path: path.clone(),
            detail: error.to_string(),
        })?;
        if !contains(&image, SEAM) {
            return Err(NotStaged::NotTestSupport { path });
        }
    }
    Ok(Staged {
        directory: directory.to_path_buf(),
    })
}

/// Where staged pairs live: beside cargo's own output, so `cargo clean` takes
/// them and nothing outside the project accumulates.
fn staging_root(source: &Path) -> PathBuf {
    source
        .parent()
        .unwrap_or(source)
        .join("e2e-staged-binaries")
}

/// What identifies the build being staged: the sources' own identity, so a
/// rebuild stages afresh and an unchanged build is staged once.
fn fingerprint(source: &Path) -> String {
    let mut hasher = DefaultHasher::new();
    for name in STAGED {
        let path = source.join(name);
        match std::fs::metadata(&path) {
            Ok(metadata) => {
                metadata.dev().hash(&mut hasher);
                metadata.ino().hash(&mut hasher);
                metadata.size().hash(&mut hasher);
                metadata.mtime().hash(&mut hasher);
                metadata.mtime_nsec().hash(&mut hasher);
            }
            // Unreadable is not this function's answer to give: staging says
            // so, with the path and the reason.
            Err(_) => path.to_string_lossy().hash(&mut hasher),
        }
    }
    format!("{:016x}", hasher.finish())
}

fn create_dir_all(path: &Path) -> Result<(), NotStaged> {
    std::fs::create_dir_all(path).map_err(|error| NotStaged::Unreadable {
        path: path.to_path_buf(),
        detail: error.to_string(),
    })
}

fn write_executable(path: &Path, image: &[u8]) -> Result<(), NotStaged> {
    let unreadable = |error: std::io::Error| NotStaged::Unreadable {
        path: path.to_path_buf(),
        detail: error.to_string(),
    };
    std::fs::write(path, image).map_err(unreadable)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).map_err(unreadable)
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}
