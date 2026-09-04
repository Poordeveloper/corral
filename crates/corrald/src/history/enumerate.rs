//! Enumeration, not parsing (ADR 0016 D1): a provider session store is read
//! for three facts — the identity in a file's name, the recency in its
//! modification time, and the label of the directory it is filed under —
//! and nothing inside a file is ever opened.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use corral_core::ExternalId;

use tracing::debug;

use crate::provider::{KnownProvider, program};

/// One session the store holds, as the list may show it before Corral knows
/// anything else about it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HistoryEntry {
    pub provider: KnownProvider,
    pub external_id: ExternalId,
    pub last_active: SystemTime,
    /// When Corral read the store and found this, which is what the evidence
    /// for a history claim is dated on. Not `last_active`: that is when the
    /// session acted, and freshness asks how old the *observation* is
    /// (ADR 0015 D5).
    pub observed_at: SystemTime,
    /// The directory the provider filed it under, as the provider spelled
    /// it. A label, never decoded into a path: Claude's encoding is not
    /// reversible (grill Q25).
    pub store_label: String,
    pub path: PathBuf,
}

/// The recent window and cap — query defaults, not wire constants
/// (grill Q25).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Recent {
    pub window: Duration,
    pub cap: usize,
}

impl Default for Recent {
    fn default() -> Self {
        Self {
            window: Duration::from_secs(14 * 24 * 60 * 60),
            cap: 30,
        }
    }
}

/// Where a provider keeps its sessions under an account home.
#[must_use]
pub fn store_root(provider: KnownProvider, home: &Path) -> PathBuf {
    match provider {
        KnownProvider::Claude => home.join(".claude/projects"),
        KnownProvider::Codex => home.join(".codex/sessions"),
    }
}

/// Whether the matrix measured this provider's store layout, at this exact
/// installed version (ADR 0016 D1).
///
/// A row is a claim that a session exists, and the shape it was read from is
/// what supports the claim. Sealing is therefore per version and exact: a
/// version whose layout nobody measured is not enumerated, however close its
/// number is to one that was. Evidence:
/// `docs/evidence/pr8b-history-store-and-resume-2026-09-02.md`.
#[must_use]
pub fn layout_sealed(provider: KnownProvider, version: &str) -> bool {
    match provider {
        // 2.1.258 in the first matrix run, 2.1.259 in the second: the same
        // dash-encoded project directory, the same `<uuid>.jsonl` per session,
        // the same `memory/` beside them. Two rows rather than a range,
        // because 2.1.258's binary no longer exists to re-measure and nothing
        // between or after them was looked at.
        KnownProvider::Claude => matches!(version, "2.1.258" | "2.1.259"),
        KnownProvider::Codex => version == "0.152.0",
    }
}

/// The provider install this machine has, when the matrix sealed its store
/// layout at that exact version.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SealedInstall {
    /// The executable the version was read from, canonicalized.
    ///
    /// Carried rather than re-derived: a bare program name resolved again at
    /// exec is a second question, and an install upgraded in between answers
    /// it differently. What was sealed is a version, and the only thing that
    /// ties a version to a launch is running the file it was read from
    /// (ADR 0016 D4).
    pub executable: PathBuf,
    pub version: String,
}

/// The sealed install, or `None` when this machine has none.
///
/// Never cached: an install can be upgraded under a running daemon, and the
/// answer must follow the binary that is there now — both for what may be
/// enumerated and for what a row learned earlier may still be used for.
/// Filesystem work, so callers on the reactor ask off it.
#[must_use]
pub fn sealed_here(provider: KnownProvider) -> Option<SealedInstall> {
    let executable =
        crate::provider::version::resolve_program(std::path::Path::new(program(provider)))?;
    let Some(installed) = crate::provider::version::installed_version(provider, &executable) else {
        debug!(
            provider = provider.as_str(),
            "the installed version could not be read; its store is not enumerated",
        );
        return None;
    };
    if !layout_sealed(provider, &installed.version) {
        debug!(
            provider = provider.as_str(),
            version = installed.version,
            "this version's store layout is unmeasured; its store is not enumerated",
        );
        return None;
    }
    Some(SealedInstall {
        executable,
        version: installed.version,
    })
}

/// `sealed_here`, asked off the reactor.
///
/// The one way a task on the reactor asks: `corrald` runs a single reactor
/// thread, so a slow `PATH` entry answered inline would stall every client
/// request, hook delivery and timer with it. A question this daemon could not
/// put fails closed, because an unanswered sealing question is not a sealed
/// one.
pub async fn sealed_now(
    provider: KnownProvider,
    sealed: fn(KnownProvider) -> Option<SealedInstall>,
) -> Option<SealedInstall> {
    tokio::task::spawn_blocking(move || sealed(provider))
        .await
        .ok()
        .flatten()
}

/// The store's recent sessions, newest first, one per identity.
#[must_use]
pub fn enumerate(
    provider: KnownProvider,
    root: &Path,
    now: SystemTime,
    recent: &Recent,
) -> Vec<HistoryEntry> {
    let oldest = now.checked_sub(recent.window);
    let mut newest: HashMap<ExternalId, HistoryEntry> = HashMap::new();
    for (path, label) in session_files(provider, root) {
        let Some(id) = identity_in_name(provider, &path) else {
            continue;
        };
        // The file's own time, never a link target's.
        let Ok(modified) = std::fs::symlink_metadata(&path).and_then(|m| m.modified()) else {
            continue;
        };
        if oldest.is_some_and(|oldest| modified < oldest) {
            continue;
        }
        let entry = HistoryEntry {
            provider,
            external_id: id.clone(),
            last_active: modified,
            observed_at: now,
            store_label: label,
            path,
        };
        match newest.get(&id) {
            Some(held) if held.last_active >= modified => {}
            _ => {
                newest.insert(id, entry);
            }
        }
    }
    let mut entries: Vec<HistoryEntry> = newest.into_values().collect();
    entries.sort_by(|a, b| {
        b.last_active
            .cmp(&a.last_active)
            .then_with(|| a.external_id.as_str().cmp(b.external_id.as_str()))
    });
    entries.truncate(recent.cap);
    entries
}

/// The files that are sessions under this provider's layout, with the label
/// of the directory each is filed under.
fn session_files(provider: KnownProvider, root: &Path) -> Vec<(PathBuf, String)> {
    match provider {
        // `<root>/<encoded cwd>/<uuid>.jsonl`; directories beside the files
        // and `memory/` are not sessions.
        KnownProvider::Claude => directories(root)
            .into_iter()
            .flat_map(|project| {
                let label = project
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default();
                files(&project)
                    .into_iter()
                    .map(move |path| (path, label.clone()))
            })
            .collect(),
        // `<root>/YYYY/MM/DD/rollout-<timestamp>-<uuid>.jsonl`. The date
        // components are checked rather than counted: three levels deep and a
        // uuid suffix is an approximation of the sealed shape, and what D1
        // seals is the shape itself — it is what tells a session apart from
        // everything else a provider keeps in its store (grill Q25).
        KnownProvider::Codex => named(root, 4)
            .into_iter()
            .flat_map(|year| named(&year, 2))
            .flat_map(|month| named(&month, 2))
            .flat_map(|day| {
                let label = day
                    .strip_prefix(root)
                    .map(|rel| rel.to_string_lossy().into_owned())
                    .unwrap_or_default();
                files(&day)
                    .into_iter()
                    .map(move |path| (path, label.clone()))
            })
            .collect(),
    }
}

/// The identity the provider wrote into the file's name, or `None` for a
/// file that is not a session under the sealed shape.
fn identity_in_name(provider: KnownProvider, path: &Path) -> Option<ExternalId> {
    let stem = path.file_stem()?.to_str()?;
    if path.extension()?.to_str()? != "jsonl" {
        return None;
    }
    let candidate = match provider {
        KnownProvider::Claude => stem,
        // `rollout-<timestamp>-<uuid>`: the uuid is the last 36 characters,
        // and it carries dashes of its own, so it is cut by length rather
        // than split.
        KnownProvider::Codex => {
            let rest = stem.strip_prefix("rollout-")?;
            let cut = rest.len().checked_sub(36)?;
            if !rest.is_char_boundary(cut) || !rest[..cut].ends_with('-') {
                return None;
            }
            if !looks_like_rollout_time(&rest[..cut - 1]) {
                return None;
            }
            &rest[cut..]
        }
    };
    looks_like_uuid(candidate).then(|| ExternalId::new(candidate).ok())?
}

/// `YYYY-MM-DDTHH-MM-SS`, the time Codex writes into a rollout's name.
fn looks_like_rollout_time(text: &str) -> bool {
    let mut parts = text.split('T');
    let (Some(date), Some(time), None) = (parts.next(), parts.next(), parts.next()) else {
        return false;
    };
    let date: Vec<&str> = date.split('-').collect();
    let time: Vec<&str> = time.split('-').collect();
    date.len() == 3
        && time.len() == 3
        && date
            .iter()
            .zip([4, 2, 2])
            .chain(time.iter().zip([2, 2, 2]))
            .all(|(part, len)| digits(part, len))
}

/// The subdirectories whose names are exactly `len` digits.
fn named(dir: &Path, len: usize) -> Vec<PathBuf> {
    directories(dir)
        .into_iter()
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| digits(name, len))
        })
        .collect()
}

fn digits(text: &str, len: usize) -> bool {
    text.len() == len && text.chars().all(|c| c.is_ascii_digit())
}

fn looks_like_uuid(text: &str) -> bool {
    let parts: Vec<&str> = text.split('-').collect();
    parts.len() == 5
        && parts
            .iter()
            .zip([8, 4, 4, 4, 12])
            .all(|(part, len)| part.len() == len && part.chars().all(|c| c.is_ascii_hexdigit()))
}

fn directories(dir: &Path) -> Vec<PathBuf> {
    entries(dir)
        .into_iter()
        .filter_map(|(path, kind)| kind.is_dir().then_some(path))
        .collect()
}

fn files(dir: &Path) -> Vec<PathBuf> {
    entries(dir)
        .into_iter()
        .filter_map(|(path, kind)| kind.is_file().then_some(path))
        .collect()
}

/// Each entry with the kind of the entry itself.
///
/// `DirEntry::file_type` does not follow a symlink, and that is the point: the
/// sealed layouts describe what a provider writes *under* its store, and a
/// link is a name in the store pointing at a file that is not. Following one
/// would enumerate whatever the filesystem can reach from there and call it a
/// session the provider holds — a claim the measurement never made, carrying
/// the assurance a history record is granted (ADR 0016 D1).
fn entries(dir: &Path) -> Vec<(PathBuf, std::fs::FileType)> {
    std::fs::read_dir(dir)
        .map(|read| {
            read.filter_map(Result::ok)
                .filter_map(|entry| Some((entry.path(), entry.file_type().ok()?)))
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
#[path = "enumerate_tests.rs"]
mod tests;
