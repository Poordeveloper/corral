//! Deciding whether a process is a provider Corral supports.
//!
//! Only what the matrix established is sealed here (grill Q5′). The
//! primitives, measured 2026-09-02 across macOS and Linux and across the
//! native-installer and npm channels:
//!
//! - the **resolved executable** is evidence; `argv[0]` never is, because the
//!   invoked path is a symlink or a launcher on every channel measured;
//! - the provider's real binary may sit **one runtime hop below** a launcher —
//!   Codex's npm entry is a `node` script that spawns the native child on both
//!   platforms — so a walk follows measured shapes rather than stopping at the
//!   first runtime it meets;
//! - a truncated `comm` is never primary identity evidence (16 characters on
//!   macOS, observed directly);
//! - being a **descendant** of a provider is not recognition: providers spawn
//!   `git` and other children that are not the agent.
//!
//! Not sealed, and deliberately unreachable from here: the chain *above* a
//! provider — tmux, screen, `nohup`, wrapper scripts — and the Homebrew
//! install shape. Until the matrix has those rows, no claim may depend on
//! them.

use std::path::Path;

use super::KnownProvider;

/// Whether this executable is a provider Corral supports.
///
/// By the executable's own name where the channel gives it one, and by the
/// installer's layout where it does not: Claude's native installer resolves
/// to `…/claude/versions/<version>`, whose file name is a version string and
/// says nothing on its own (measured 2026-09-02). Both are exact — a program
/// merely *containing* a provider's name is a different program, and matching
/// loosely would let `corral-helper` be read as `corral`.
#[must_use]
pub fn provider_of(executable: &Path) -> Option<KnownProvider> {
    KnownProvider::ALL
        .into_iter()
        .find(|provider| recognizes(*provider, executable))
}

fn recognizes(provider: KnownProvider, executable: &Path) -> bool {
    let name = file_name(executable);
    match provider {
        // `claude.exe` is the local channel's Mach-O binary on macOS, not a
        // Windows artifact.
        KnownProvider::Claude => {
            matches!(name, Some("claude" | "claude.exe")) || is_versioned_claude(executable)
        }
        // Both measured channels name the native agent plainly. The npm entry
        // that spawns it is `node`, which is a launcher and not this.
        KnownProvider::Codex => name == Some("codex"),
    }
}

/// The native installer's layout: `<anywhere>/claude/versions/<version>`.
///
/// Read as a layout rather than as a version pattern. What a release names
/// its files is the installer's business and changes; that the binary lives
/// two components below a `claude` directory is what the channel actually
/// guarantees, and a version regular expression would stop recognizing the
/// provider the first time a release numbered itself differently.
fn is_versioned_claude(executable: &Path) -> bool {
    let components: Vec<&str> = executable
        .components()
        .filter_map(|component| component.as_os_str().to_str())
        .collect();
    let [.., "claude", "versions", _version] = components.as_slice() else {
        return false;
    };
    true
}

fn file_name(executable: &Path) -> Option<&str> {
    executable.file_name()?.to_str()
}

/// Whether this executable is a language runtime a provider may be one hop
/// below.
///
/// Measured: Codex's npm channel is a `node` wrapper that spawns the native
/// binary, identically on macOS and Linux. A walk that stopped at the wrapper
/// would recognize `node` and conclude nothing; one that treats every runtime
/// as transparent would walk past unrelated programs. This names the shapes
/// the matrix measured and nothing else.
#[must_use]
pub fn is_provider_launcher(executable: &Path) -> bool {
    executable
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| matches!(name, "node" | "node.exe"))
}

#[cfg(test)]
#[path = "recognition_tests.rs"]
mod tests;
