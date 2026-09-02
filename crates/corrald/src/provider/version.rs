//! Which provider version a runtime is, from the installation it runs.
//!
//! No provider event carries a version, and the version on disk is not the
//! version a process is running once the installation has been updated
//! underneath it. So a version is established from installation metadata
//! and bound to a process only when that metadata predates the process
//! (grill Q12): a sealed versioned path binds directly; a mutable package
//! root binds while it is older than the runtime; anything else is Unknown,
//! which seals nothing.

use std::path::Path;
use std::time::SystemTime;

use super::KnownProvider;

/// A version read from an installation, and when that installation's
/// metadata last changed — `None` for a shape that carries the version in
/// its path, which cannot change under a running process.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstalledVersion {
    pub version: String,
    pub metadata_changed_at: Option<SystemTime>,
}

impl InstalledVersion {
    /// The version bound to a process that started at `process_started`, or
    /// `None` when the metadata changed after it did and may describe a
    /// different binary than the one running.
    #[must_use]
    pub fn bound_to(&self, process_started: SystemTime) -> Option<String> {
        match self.metadata_changed_at {
            Some(changed) if changed > process_started => None,
            Some(_) | None => Some(self.version.clone()),
        }
    }
}

/// The version the installation behind `executable` carries, by the shapes
/// the matrix measured, or `None` for a shape this build has no rule for.
#[must_use]
pub fn installed_version(provider: KnownProvider, executable: &Path) -> Option<InstalledVersion> {
    match provider {
        KnownProvider::Claude => versioned_path(executable).or_else(|| {
            package_beside(
                executable.parent()?,
                "node_modules/@anthropic-ai/claude-code",
                "@anthropic-ai/claude-code",
            )
        }),
        KnownProvider::Codex => {
            // npm: `<root>/bin/codex.js` under the package whose manifest
            // names the version.
            let bin = executable.parent()?;
            let root = bin.parent()?;
            package_at(root, "@openai/codex")
        }
    }
}

/// `…/claude/versions/<version>/…` — the channel `provider::recognition`
/// already seals; the version is the path component and nothing is read.
fn versioned_path(executable: &Path) -> Option<InstalledVersion> {
    let components: Vec<&str> = executable
        .components()
        .filter_map(|component| component.as_os_str().to_str())
        .collect();
    let at = components
        .windows(2)
        .position(|pair| pair == ["claude", "versions"])?;
    let version = components.get(at + 2)?;
    Some(InstalledVersion {
        version: (*version).to_owned(),
        metadata_changed_at: None,
    })
}

fn package_beside(dir: &Path, relative: &str, name: &str) -> Option<InstalledVersion> {
    package_at(&dir.join(relative), name)
}

/// Read `<root>/package.json`, insisting on the package name so a directory
/// that merely looks like an installation is not read as one.
fn package_at(root: &Path, name: &str) -> Option<InstalledVersion> {
    let path = root.join("package.json");
    let text = std::fs::read_to_string(&path).ok()?;
    let package: serde_json::Value = serde_json::from_str(&text).ok()?;
    if package["name"].as_str() != Some(name) {
        return None;
    }
    let version = package["version"].as_str()?.to_owned();
    let metadata_changed_at = std::fs::metadata(&path).ok()?.modified().ok();
    Some(InstalledVersion {
        version,
        metadata_changed_at,
    })
}

/// Where a program name resolves on this daemon's PATH, with symlinks
/// followed — the executable a managed launch will actually run, and the
/// installation whose metadata names its version.
#[must_use]
pub fn resolve_program(program: &Path) -> Option<std::path::PathBuf> {
    if program.components().count() > 1 {
        return std::fs::canonicalize(program).ok();
    }
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(program))
        .find(|candidate| candidate.is_file())
        .and_then(|found| std::fs::canonicalize(found).ok())
}

#[cfg(test)]
#[path = "version_tests.rs"]
mod tests;
