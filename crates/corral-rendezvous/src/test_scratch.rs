//! A throwaway directory for tests that need real filesystem behaviour.
//!
//! The rendezvous rules are about inodes, modes, and link types, so testing
//! them against a fake filesystem would prove nothing about the rules that
//! matter.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

static COUNTER: AtomicU32 = AtomicU32::new(0);

pub struct ScratchDir {
    path: PathBuf,
}

impl ScratchDir {
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// Tests bind real Unix sockets under these directories, and a socket address
/// is limited to about a hundred bytes — less than a per-user `TMPDIR` leaves
/// on macOS. So the base is short and absolute rather than `temp_dir()`.
pub fn scratch_dir(name: &str) -> ScratchDir {
    let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
    let short: String = name.chars().take(6).collect();
    let path = PathBuf::from("/tmp").join(format!("crl-r{}-{unique}-{short}", std::process::id()));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).expect("create scratch directory");
    ScratchDir { path }
}

/// Permission tests prove nothing when the checks do not apply to the caller.
pub fn permission_checks_apply() -> bool {
    uzers::get_effective_uid() != 0
}
