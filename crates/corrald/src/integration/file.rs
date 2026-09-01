//! Reading, merging, and replacing a provider's configuration file.
//!
//! The write rule in one place, because every operation must obey the same
//! one (ADR 0013 D3): back the original up before touching it, apply the
//! merge to a parsed document, re-parse the complete candidate with Corral's
//! own strict validator, write a temporary file in the same directory, check
//! that the original has not changed underneath, and rename. A failure at any
//! step leaves the user's bytes exactly as they were.
//!
//! Two representations, for measured reasons (grill Q3′). Claude's JSON is
//! parsed whole and reserialized — the provider itself normalizes the
//! document on its own writes, so preserving byte layout buys nothing, and a
//! comment would make the file invalid. Codex's TOML is edited in place with
//! a format-preserving editor — the provider preserves what it did not write,
//! and the user's comments are legal and theirs.

use std::io::Write as _;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::Path;
use std::time::SystemTime;

use serde_json::Value;
use toml_edit::DocumentMut;

use super::trigger::{self, Trigger};
use super::{Standing, Target};
use crate::provider::launch::RelayInvocation;
use crate::provider::{KnownProvider, claude_integration, codex_integration};
use corral_core::RepairableDrift;

/// The mode a configuration file Corral creates is born with.
///
/// The user's own file keeps whatever mode it has: Corral is a guest in it.
/// A file Corral creates gets the mode a provider's own configuration
/// carries, not 0600 — this is the user's configuration, not Corral state,
/// and a file only Corral could read would be a surprise in their home.
const CREATED_MODE: u32 = 0o644;

/// What was read, and what it means.
pub(super) struct Read {
    document: Document,
    /// What the file looked like when it was read, and `None` when there was
    /// no file. The write compares against it immediately before renaming.
    identity: Option<Identity>,
    /// The bytes as they were, for the backup a mutation takes first.
    original: Option<String>,
}

/// One provider's configuration, parsed the way that provider's format needs.
enum Document {
    Claude(Value),
    Codex(Box<DocumentMut>),
}

/// Enough of a file's metadata to notice it was replaced.
///
/// Not a content hash: the question is whether *another writer* published a
/// different file, and a provider that reserializes its own settings changes
/// all three of these. Corral loses that race on purpose.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Identity {
    inode: u64,
    size: u64,
    modified: Option<SystemTime>,
}

impl Identity {
    fn of(metadata: &std::fs::Metadata) -> Self {
        Self {
            inode: metadata.ino(),
            size: metadata.size(),
            modified: metadata.modified().ok(),
        }
    }
}

/// Read and parse the provider's configuration.
///
/// `Ok(None)` is a file that is not there, which is the ordinary case on a
/// fresh machine and not a failure: both providers ship without one
/// (measured 2026-09-02).
pub(super) fn read(target: &Target) -> Result<Option<Read>, Trigger> {
    let raw = match std::fs::read_to_string(target.path()) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(Trigger::NotWritable {
                detail: error.to_string(),
            });
        }
    };
    let identity = std::fs::metadata(target.path())
        .ok()
        .map(|data| Identity::of(&data));
    let document = parse(target.provider(), &raw)?;
    Ok(Some(Read {
        document,
        identity,
        original: Some(raw),
    }))
}

fn parse(provider: KnownProvider, raw: &str) -> Result<Document, Trigger> {
    match provider {
        KnownProvider::Claude => {
            let value: Value = serde_json::from_str(raw).map_err(|error| Trigger::Unparseable {
                detail: error.to_string(),
            })?;
            if !value.is_object() {
                return Err(Trigger::IncompatibleStructure {
                    at: "the settings document".to_owned(),
                });
            }
            Ok(Document::Claude(value))
        }
        KnownProvider::Codex => {
            let document: DocumentMut =
                raw.parse()
                    .map_err(|error: toml_edit::TomlError| Trigger::Unparseable {
                        detail: error.to_string(),
                    })?;
            Ok(Document::Codex(Box::new(document)))
        }
    }
}

impl Read {
    /// The empty document an install starts from when there is no file.
    pub(super) fn absent(target: &Target) -> Self {
        let document = match target.provider() {
            KnownProvider::Claude => Document::Claude(Value::Object(serde_json::Map::new())),
            KnownProvider::Codex => Document::Codex(Box::new(DocumentMut::new())),
        };
        Self {
            document,
            identity: None,
            original: None,
        }
    }

    /// What this document says about Corral's integration.
    pub(super) fn standing(&self, relay: &RelayInvocation) -> Standing {
        match &self.document {
            Document::Claude(value) => match claude_integration::installed(value, relay) {
                claude_integration::Installed::Absent => Standing::NotInstalled,
                claude_integration::Installed::Current => Standing::Installed,
                claude_integration::Installed::Stale => {
                    Standing::Drifted(RepairableDrift::OldRepresentation)
                }
                claude_integration::Installed::Newer(version) => {
                    Standing::Refused(Trigger::NewerIntegrationVersion { version })
                }
            },
            Document::Codex(document) => match codex_integration::slot(document, relay) {
                codex_integration::Slot::Absent => Standing::NotInstalled,
                codex_integration::Slot::Current => Standing::Installed,
                codex_integration::Slot::Stale => {
                    Standing::Drifted(RepairableDrift::OldRepresentation)
                }
                codex_integration::Slot::Newer(version) => {
                    Standing::Refused(Trigger::NewerIntegrationVersion { version })
                }
                codex_integration::Slot::Occupied => Standing::Refused(Trigger::NotifierOccupied),
                codex_integration::Slot::Malformed => {
                    Standing::Refused(Trigger::IncompatibleStructure {
                        at: codex_integration::NOTIFY.to_owned(),
                    })
                }
            },
        }
    }
}

/// A document being edited, and the two edits an operation may make.
///
/// A closed pair rather than a `&mut Document`: the only mutations this
/// module performs are the ones the adapters define, and a caller that could
/// reach the parsed value could make a third.
pub(super) struct Editing<'a> {
    document: &'a mut Document,
    path: &'a Path,
}

impl Editing<'_> {
    pub(super) fn install(&mut self, relay: &RelayInvocation) -> Result<(), Trigger> {
        match self.document {
            Document::Claude(value) => {
                if let Some(trigger) = trigger::hooks_silenced(self.path, value) {
                    return Err(trigger);
                }
                refuse_incompatible_hooks(value)?;
                claude_integration::install(value, relay);
            }
            Document::Codex(document) => codex_integration::install(document, relay),
        }
        Ok(())
    }

    pub(super) fn uninstall(&mut self) {
        match self.document {
            Document::Claude(value) => claude_integration::uninstall(value),
            Document::Codex(document) => codex_integration::uninstall(document),
        }
    }
}

/// Refuse a `hooks` tree whose shape the merge cannot traverse.
///
/// The producer of that shape is the user or another tool, and a consumer
/// that repaired it would be silently reinterpreting configuration it does
/// not own.
fn refuse_incompatible_hooks(value: &Value) -> Result<(), Trigger> {
    let Some(hooks) = value.get("hooks") else {
        return Ok(());
    };
    if !hooks.is_object() {
        return Err(Trigger::IncompatibleStructure {
            at: "hooks".to_owned(),
        });
    }
    for event in claude_integration::EVENTS {
        if let Some(entries) = hooks.get(event)
            && !entries.is_array()
        {
            return Err(Trigger::IncompatibleStructure {
                at: format!("hooks.{event}"),
            });
        }
    }
    Ok(())
}

/// Apply an edit and publish the result, or leave the file untouched.
pub(super) fn replace(
    target: &Target,
    original: &Read,
    now: SystemTime,
    state_dir: &Path,
    edit: impl FnOnce(&mut Editing<'_>) -> Result<(), Trigger>,
) -> Result<(), Trigger> {
    let mut document = clone_of(&original.document);
    let mut editing = Editing {
        document: &mut document,
        path: target.path(),
    };
    edit(&mut editing)?;
    let candidate = render(&document);
    validate(target.provider(), &candidate)?;

    // Before the first byte is written anywhere: a backup a later mutation
    // could not take is a mutation that must not happen (ADR 0013 D3).
    if let Some(bytes) = &original.original {
        back_up(target, bytes, now, state_dir)?;
    }
    publish(target, &candidate, original.identity)
}

fn clone_of(document: &Document) -> Document {
    match document {
        Document::Claude(value) => Document::Claude(value.clone()),
        Document::Codex(toml) => Document::Codex(toml.clone()),
    }
}

fn render(document: &Document) -> String {
    match document {
        // Pretty-printed with a trailing newline, the shape Claude's own
        // writes produce. Matching it means a `git diff` of a dotfiles
        // repository shows Corral's entry and not a whole-file reformat.
        Document::Claude(value) => format!(
            "{}\n",
            serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
        ),
        Document::Codex(toml) => toml.to_string(),
    }
}

/// Re-parse the complete candidate with Corral's own strict validator for
/// this provider's measured configuration grammar (grill Q2′).
///
/// Corral's parser, not the provider's — this is never a claim that the
/// provider accepted anything, and the supported-version matrix remains the
/// empirical authority. It is necessary and not sufficient, and it is cheap:
/// the alternative is publishing a file that stops the user's agent.
fn validate(provider: KnownProvider, candidate: &str) -> Result<(), Trigger> {
    let outcome = match provider {
        KnownProvider::Claude => serde_json::from_str::<Value>(candidate)
            .map(|_| ())
            .map_err(|error| error.to_string()),
        KnownProvider::Codex => candidate
            .parse::<DocumentMut>()
            .map(|_| ())
            .map_err(|error| error.to_string()),
    };
    outcome.map_err(|detail| Trigger::CandidateRejected { detail })
}

/// Copy the file as it is into Corral's own state directory before changing
/// it.
///
/// A disclosed recovery artifact, not a byte-for-byte restore promise
/// (ADR 0006 refused that promise and this does not reinstate it).
fn back_up(target: &Target, bytes: &str, now: SystemTime, state_dir: &Path) -> Result<(), Trigger> {
    let directory = state_dir.join(BACKUP_DIR);
    std::fs::create_dir_all(&directory).map_err(|error| Trigger::NotWritable {
        detail: format!("the backup directory could not be created: {error}"),
    })?;
    let stamp = now
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|since| since.as_secs())
        .unwrap_or_default();
    let name = target.path().file_name().map_or_else(
        || "configuration".to_owned(),
        |name| name.to_string_lossy().into_owned(),
    );
    let backup = directory.join(format!("{}-{stamp}-{name}", target.provider().as_str()));
    std::fs::write(&backup, bytes).map_err(|error| Trigger::NotWritable {
        detail: format!("the backup could not be written: {error}"),
    })
}

/// Where copies of a user's configuration taken before a mutation live.
const BACKUP_DIR: &str = "integration-backups";

/// Write the candidate beside the target and rename it into place.
///
/// The identity check sits between the temporary file and the rename, as late
/// as it can: a provider that reserialized its own settings in that window
/// published a file this operation never read, and renaming over it would
/// silently discard the provider's write.
fn publish(target: &Target, candidate: &str, expected: Option<Identity>) -> Result<(), Trigger> {
    let path = target.path();
    let directory = path.parent().ok_or_else(|| Trigger::NotWritable {
        detail: "the configuration path has no directory".to_owned(),
    })?;
    std::fs::create_dir_all(directory).map_err(|error| Trigger::NotWritable {
        detail: error.to_string(),
    })?;
    let partial = directory.join(format!(
        ".{}.corral-partial",
        path.file_name().map_or_else(
            || "configuration".to_owned(),
            |name| name.to_string_lossy().into_owned()
        )
    ));

    let mode = std::fs::metadata(path)
        .map(|data| data.permissions().mode() & 0o777)
        .unwrap_or(CREATED_MODE);
    let written = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(mode)
        .open(&partial)
        .and_then(|mut file| {
            file.write_all(candidate.as_bytes())?;
            file.sync_all()
        });
    if let Err(error) = written {
        let _ = std::fs::remove_file(&partial);
        return Err(Trigger::NotWritable {
            detail: error.to_string(),
        });
    }

    if current_identity(path) != expected {
        let _ = std::fs::remove_file(&partial);
        return Err(Trigger::ChangedWhileWriting);
    }
    std::fs::rename(&partial, path).map_err(|error| {
        let _ = std::fs::remove_file(&partial);
        Trigger::NotWritable {
            detail: error.to_string(),
        }
    })
}

fn current_identity(path: &Path) -> Option<Identity> {
    std::fs::metadata(path).ok().map(|data| Identity::of(&data))
}

/// The path a backup of this target would be written to, for a test to read.
#[cfg(test)]
pub(super) fn backup_dir(state_dir: &Path) -> std::path::PathBuf {
    state_dir.join(BACKUP_DIR)
}
