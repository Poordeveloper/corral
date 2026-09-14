//! An installation inside a test account: the staged, validated binaries laid
//! out the way the installer lays them out, under the home the test namespace
//! resolves, with the `PATH` link into it.
//!
//! The shape is this machine's — the bundle on macOS, the tree elsewhere — so
//! what the suite drives through the link is what the installer on this
//! machine would have produced.

use std::path::{Path, PathBuf};

use super::{TestAccount, binaries, create_private_dir_all};

#[cfg(target_os = "macos")]
const EXECUTABLES: &str = "Applications/Corral.app/Contents/MacOS";
#[cfg(target_os = "macos")]
const ROOT: &str = "Applications/Corral.app";
#[cfg(not(target_os = "macos"))]
const EXECUTABLES: &str = ".local/share/corral/bin";
#[cfg(not(target_os = "macos"))]
const ROOT: &str = ".local/share/corral";

/// Where the pieces of an installation went.
pub struct Installed {
    /// What `uninstall` removes.
    pub root: PathBuf,
    pub corral: PathBuf,
    pub corrald: PathBuf,
    /// `~/.local/bin/corral`, the way a shell reaches the installation.
    pub symlink: PathBuf,
}

impl Installed {
    /// Every piece still where the installer put it.
    pub fn is_intact(&self) -> bool {
        self.corral.is_file()
            && self.corrald.is_file()
            && std::fs::symlink_metadata(&self.symlink).is_ok()
    }
}

/// Lay the staged pair out as an installation under this account's home.
pub fn install(account: &TestAccount) -> Installed {
    let home = account.user_home();
    let executables = home.join(EXECUTABLES);
    create_private_dir_all(&executables);
    let staged = binaries::staged();
    let corral = place(&staged.corral(), &executables.join("corral"));
    let corrald = place(&staged.corrald(), &executables.join("corrald"));

    let bin = home.join(".local/bin");
    create_private_dir_all(&bin);
    let symlink = bin.join("corral");
    std::os::unix::fs::symlink(&corral, &symlink).expect("link the installed corral onto PATH");

    Installed {
        root: home.join(ROOT),
        corral,
        corrald,
        symlink,
    }
}

/// A hard link where the filesystem allows one — the staged copy is already
/// validated, and a link is the same bytes at a second name — and a copy
/// where it does not, such as a `/tmp` on its own filesystem.
fn place(from: &Path, to: &Path) -> PathBuf {
    if std::fs::hard_link(from, to).is_err() {
        std::fs::copy(from, to).expect("copy a staged binary into the installation");
    }
    to.to_path_buf()
}
