//! Enumeration, not parsing (ADR 0016 D1): a provider session store is read
//! for three facts — the identity in a file's name, the recency in its
//! modification time, and the label of the directory it is filed under —
//! and nothing inside a file is ever opened.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use corral_core::ExternalId;

use crate::provider::KnownProvider;

/// One session the store holds, as the list may show it before Corral knows
/// anything else about it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HistoryEntry {
    pub provider: KnownProvider,
    pub external_id: ExternalId,
    pub last_active: SystemTime,
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

/// Whether the matrix sealed this provider's store layout for the versions
/// in use (ADR 0016 D1). Nothing is sealed until the acceptance
/// reconciliation; until then the daemon enumerates nothing.
#[must_use]
pub fn layout_sealed(provider: KnownProvider) -> bool {
    match provider {
        KnownProvider::Claude | KnownProvider::Codex => false,
    }
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
        let Ok(modified) = std::fs::metadata(&path).and_then(|m| m.modified()) else {
            continue;
        };
        if oldest.is_some_and(|oldest| modified < oldest) {
            continue;
        }
        let entry = HistoryEntry {
            provider,
            external_id: id.clone(),
            last_active: modified,
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
        // `<root>/YYYY/MM/DD/rollout-<timestamp>-<uuid>.jsonl`.
        KnownProvider::Codex => directories(root)
            .into_iter()
            .flat_map(|year| directories(&year))
            .flat_map(|month| directories(&month))
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
            if !rest.is_char_boundary(cut) || (cut > 0 && !rest[..cut].ends_with('-')) {
                return None;
            }
            &rest[cut..]
        }
    };
    looks_like_uuid(candidate).then(|| ExternalId::new(candidate).ok())?
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
    entries(dir).into_iter().filter(|p| p.is_dir()).collect()
}

fn files(dir: &Path) -> Vec<PathBuf> {
    entries(dir).into_iter().filter(|p| p.is_file()).collect()
}

fn entries(dir: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(dir)
        .map(|read| read.filter_map(Result::ok).map(|e| e.path()).collect())
        .unwrap_or_default()
}

#[cfg(test)]
#[path = "enumerate_tests.rs"]
mod tests;
