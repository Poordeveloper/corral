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

use std::io::{Read as _, Write as _};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
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
    /// The file the bytes came from, which is the file a replacement goes
    /// over: the configured path, or the file it is a link to.
    location: PathBuf,
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
    let location = location_of(target.path())?;
    let mut file = match std::fs::File::open(&location) {
        Ok(file) => file,
        // Only the configured path may be absent. A link whose file vanished
        // between resolving it and opening it is not "no file yet": creating
        // one at the configured path would replace the link.
        Err(error) if error.kind() == std::io::ErrorKind::NotFound && location == target.path() => {
            return Ok(None);
        }
        Err(error) => return Err(not_writable(error)),
    };
    // Bytes and identity from the one descriptor, and the identity taken on
    // both sides of the read. A provider that renames a new file into place
    // cannot then leave Corral holding one file's bytes under the other's
    // identity, and a provider writing in place is caught by the two
    // readings disagreeing — a prefix of a document can parse, and a
    // candidate built from one would publish the truncation.
    let before = file.metadata().map_err(not_writable)?;
    let mut raw = String::new();
    file.read_to_string(&mut raw).map_err(not_writable)?;
    let identity = Identity::of(&file.metadata().map_err(not_writable)?);
    if Identity::of(&before) != identity {
        return Err(Trigger::ChangedUnderCorral);
    }
    let document = parse(target.provider(), &raw)?;
    Ok(Some(Read {
        document,
        location,
        identity: Some(identity),
        original: Some(raw),
    }))
}

/// The file the configured path actually names.
///
/// A dotfiles user keeps `~/.claude/settings.json` as a link into a
/// repository. Following it is what keeps that arrangement intact: the
/// replacement is renamed over the file the link names, and the link stays
/// as the user made it. A link to nothing is refused rather than resolved,
/// because where the user's configuration should come to exist is not
/// Corral's to decide.
fn location_of(path: &Path) -> Result<PathBuf, Trigger> {
    match std::fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(path.to_path_buf()),
        Err(error) => Err(not_writable(error)),
        Ok(data) if !data.file_type().is_symlink() => Ok(path.to_path_buf()),
        Ok(_) => std::fs::canonicalize(path).map_err(|error| Trigger::NotWritable {
            detail: format!(
                "{} is a symbolic link that could not be followed ({error})",
                path.display()
            ),
        }),
    }
}

fn not_writable(error: std::io::Error) -> Trigger {
    Trigger::NotWritable {
        detail: error.to_string(),
    }
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
            location: target.path().to_path_buf(),
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
    publish(
        target.path(),
        &original.location,
        &candidate,
        original.identity,
    )
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
///
/// One backup per mutation, never one per second: the name carries the
/// moment for a person to find it by, and a second mutation in the same
/// moment — an enable and a disable back to back — takes the next name
/// rather than the first backup's place, which held the bytes the user had
/// before either.
///
/// Retention is bounded (ADR 0013 D3): once this file has more copies than
/// are kept, the oldest go. Only this file's copies, and only the names this
/// module wrote — anything else in the directory is somebody's and stays.
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
    let provider = target.provider().as_str();
    let mut kept = backups_of(&directory, provider, &name)?;
    let not_writable = |error: std::io::Error| Trigger::NotWritable {
        detail: format!("the backup could not be written: {error}"),
    };
    // Past the highest number this second already holds, never into a gap:
    // a name freed by retention and taken again would sort as the oldest
    // copy and be the next to go — the copy just taken.
    let first = kept
        .iter()
        .filter(|(moment, _)| moment.stamp == stamp)
        .map(|(moment, _)| moment.attempt + 1)
        .max()
        .unwrap_or(0);
    let mut opened = None;
    for attempt in first..first.saturating_add(BACKUPS_PER_MOMENT) {
        let backup = directory.join(BackupMoment { stamp, attempt }.file_name(provider, &name));
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&backup)
        {
            Ok(file) => {
                opened = Some((backup, file));
                break;
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(not_writable(error)),
        }
    }
    let Some((backup, mut file)) = opened else {
        return Err(Trigger::NotWritable {
            detail: format!(
                "every backup name for this moment is taken: {provider}-{stamp}-{name}"
            ),
        });
    };
    // Durable before the overwrite it exists to survive: the bytes, and then
    // the name, which syncing the file alone leaves unpromised. A copy that
    // failed part-way is removed rather than left to be counted as whole by
    // the next scan and retire a whole one in its place.
    let durable = file
        .write_all(bytes.as_bytes())
        .and_then(|()| file.sync_all())
        .and_then(|()| std::fs::File::open(&directory)?.sync_all());
    if let Err(error) = durable {
        let _ = std::fs::remove_file(&backup);
        return Err(not_writable(error));
    }

    // A copy that will not go refuses the mutation: the bound is part of the
    // backup contract, and a state directory that will not give up a file is
    // broken in a way to hear about before more is written into it. The
    // removals are not synced — a copy that outlives a crash is over the
    // bound until the next mutation, which is nothing to sync a directory for.
    if kept.len() >= BACKUPS_RETAINED {
        kept.sort();
        for (_, stale) in kept.drain(..kept.len() + 1 - BACKUPS_RETAINED) {
            match std::fs::remove_file(&stale) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(Trigger::NotWritable {
                        detail: format!(
                            "a stale backup could not be removed: {}: {error}",
                            stale.display()
                        ),
                    });
                }
            }
        }
    }
    Ok(())
}

/// Every copy of one file the backup directory holds, by the moment its
/// name carries.
///
/// A regular file with a name this module writes for this file, and nothing
/// else: a directory or a link under such a name is not a backup, is not
/// counted, and is never removed.
fn backups_of(
    directory: &Path,
    provider: &str,
    name: &str,
) -> Result<Vec<(BackupMoment, PathBuf)>, Trigger> {
    let unreadable = |error: std::io::Error| Trigger::NotWritable {
        detail: format!("the backup directory could not be read: {error}"),
    };
    let mut backups = Vec::new();
    for entry in std::fs::read_dir(directory).map_err(unreadable)? {
        let entry = entry.map_err(unreadable)?;
        let Some(moment) =
            BackupMoment::parse(&entry.file_name().to_string_lossy(), provider, name)
        else {
            continue;
        };
        if entry.file_type().is_ok_and(|kind| kind.is_file()) {
            backups.push((moment, entry.path()));
        }
    }
    Ok(backups)
}

/// The order a backup's name carries: the second it was taken in and its
/// place among the copies taken that second. Sorting on it is sorting by
/// age, because names within a second only ever count up.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct BackupMoment {
    stamp: u64,
    attempt: u32,
}

impl BackupMoment {
    fn file_name(self, provider: &str, name: &str) -> String {
        match self.attempt {
            0 => format!("{provider}-{}-{name}", self.stamp),
            attempt => format!("{provider}-{}-{name}.{attempt}", self.stamp),
        }
    }

    fn parse(file_name: &str, provider: &str, name: &str) -> Option<Self> {
        let rest = file_name.strip_prefix(provider)?.strip_prefix('-')?;
        let (stamp, rest) = rest.split_once('-')?;
        let stamp = stamp.parse().ok()?;
        let attempt = match rest.strip_prefix(name)? {
            "" => 0,
            suffix => suffix.strip_prefix('.')?.parse().ok()?,
        };
        Some(Self { stamp, attempt })
    }
}

/// How many backups one second can hold before a mutation is refused rather
/// than allowed to overwrite one. Two is the realistic number; the bound
/// exists so a name that is taken for a reason other than a backup — a
/// directory, say — ends the search instead of extending it forever.
const BACKUPS_PER_MOMENT: u32 = 64;

/// How many copies of one configuration file are kept.
///
/// A dogfood-tunable policy default. The repair breaker allows three
/// automatic mutations a day per drift, so this holds the better part of a
/// week of the worst case alongside the user's own enables and disables.
pub(super) const BACKUPS_RETAINED: usize = 20;

/// Where copies of a user's configuration taken before a mutation live.
const BACKUP_DIR: &str = "integration-backups";

/// Write the candidate beside the file and rename it into place.
///
/// The temporary file is this publish's alone: a fresh name, opened
/// exclusively. Two mutations of one file can be in flight at once, and a name
/// they shared would let one rename the other's candidate into place and
/// report success for bytes it never validated. With its own file each publish
/// renames exactly what it wrote, and a name already taken is a refusal rather
/// than a file to reuse.
///
/// The checks sit between the temporary file and the rename, as late as they
/// can. The configured path must still name the file that was read: a
/// dotfiles manager that re-pointed the link in the window would otherwise
/// have the old file edited and an installation reported that the user's
/// configuration does not carry. And that file's identity must be the one
/// read from: a provider that reserialized its own settings in the window
/// published a file this operation never read, and renaming over it would
/// silently discard the provider's write.
fn publish(
    configured: &Path,
    path: &Path,
    candidate: &str,
    expected: Option<Identity>,
) -> Result<(), Trigger> {
    let directory = path.parent().ok_or_else(|| Trigger::NotWritable {
        detail: "the configuration path has no directory".to_owned(),
    })?;
    std::fs::create_dir_all(directory).map_err(|error| Trigger::NotWritable {
        detail: error.to_string(),
    })?;
    let partial = partial_beside(path, directory)?;

    let mode = std::fs::metadata(path)
        .map(|data| data.permissions().mode() & 0o777)
        .unwrap_or(CREATED_MODE);
    // A name that is already taken is someone else's file, and it is left
    // alone: nothing of this publish exists on disk until the open succeeds.
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(mode)
        .open(&partial)
        .map_err(not_writable)?;
    let written = file
        .write_all(candidate.as_bytes())
        .and_then(|()| file.sync_all());
    if let Err(error) = written {
        let _ = std::fs::remove_file(&partial);
        return Err(not_writable(error));
    }

    let still_named = match location_of(configured) {
        Ok(location) => location == path,
        Err(trigger) => {
            let _ = std::fs::remove_file(&partial);
            return Err(trigger);
        }
    };
    if !still_named || current_identity(path) != expected {
        let _ = std::fs::remove_file(&partial);
        return Err(Trigger::ChangedUnderCorral);
    }
    std::fs::rename(&partial, path).map_err(|error| {
        let _ = std::fs::remove_file(&partial);
        not_writable(error)
    })
}

/// A name in the file's directory for one publish's candidate, and no other's.
fn partial_beside(path: &Path, directory: &Path) -> Result<PathBuf, Trigger> {
    let mut nonce = [0_u8; 8];
    getrandom::fill(&mut nonce).map_err(|error| Trigger::NotWritable {
        detail: format!("no randomness to name a temporary file: {error}"),
    })?;
    let name = path.file_name().map_or_else(
        || "configuration".to_owned(),
        |name| name.to_string_lossy().into_owned(),
    );
    let nonce = nonce
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    Ok(directory.join(format!(".{name}.corral-partial-{nonce}")))
}

fn current_identity(path: &Path) -> Option<Identity> {
    std::fs::metadata(path).ok().map(|data| Identity::of(&data))
}

/// The temporary files a publish of this path would write beside it, for a
/// test to count what a mutation left behind.
#[cfg(test)]
pub(super) fn partials_beside(path: &Path) -> Vec<PathBuf> {
    let Some(directory) = path.parent() else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(directory) else {
        return Vec::new();
    };
    entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|entry| {
            entry
                .file_name()
                .is_some_and(|name| name.to_string_lossy().contains(".corral-partial-"))
        })
        .collect()
}

/// The path a backup of this target would be written to, for a test to read.
#[cfg(test)]
pub(super) fn backup_dir(state_dir: &Path) -> std::path::PathBuf {
    state_dir.join(BACKUP_DIR)
}
