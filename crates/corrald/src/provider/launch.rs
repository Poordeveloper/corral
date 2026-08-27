//! What a managed provider launch leaves behind: its token, and its file.
//!
//! Both exist so that hook evidence can find its Session, and both have a
//! lifetime rule that is not "clean up whatever looks unused". A token
//! resolves for as long as this daemon remembers the launch it names; a file
//! is destroyed only on ownership evidence as strong as the destruction
//! (ADR 0004 D5, D6).
//!
//! > Cleanup requires ownership evidence strong enough for the artifact being
//! > destroyed. An Unverifiable Run does not provide that evidence.

use std::collections::HashMap;
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use corral_core::{CorralSessionId, RunId};
use tracing::{debug, warn};

use super::KnownProvider;

/// How the relay is invoked from a provider's hook configuration.
const RELAY_SUBCOMMAND: &str = "hook-relay";
const RELAY_PROVIDER_FLAG: &str = "--provider";
const RELAY_TOKEN_FLAG: &str = "--token";

/// The binary the injected hook command line names.
const CLIENT_BINARY: &str = "corral";

/// What every Corral-owned per-launch file is named with.
///
/// Provenance in the name itself, so a sweep can never mistake somebody else's
/// file for one of Corral's: the prefix says who wrote it and the middle says
/// which Run owns it, which is the fact cleanup has to check.
const FILE_PREFIX: &str = "corral-launch-";
const FILE_SUFFIX: &str = ".json";
/// The name a file carries while it is being written. A leftover one is a
/// write that never completed and never reached any provider.
const PARTIAL_SUFFIX: &str = ".partial";
/// The two together, spelled once. The sweep asks about it per directory
/// entry, and building the answer there allocated a constant on every file.
/// A test holds it to the pair it stands for.
const PARTIAL_FILE_SUFFIX: &str = concat!(".json", ".partial");

/// The opaque single-launch token a hook event carries back.
///
/// 128 bits from the OS CSPRNG. Correlation evidence and protection against
/// accidental cross-session confusion — proof that an event matches a
/// Corral-created launch under the non-malicious-same-user threat model. It is
/// not cryptographic authorization and not a privilege boundary: it authorizes
/// nothing and controls nothing (ADR 0004 D5).
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct LaunchToken([u8; 16]);

/// The launch a token names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LaunchScope {
    pub session: CorralSessionId,
    pub run: RunId,
    pub provider: KnownProvider,
}

/// The launches this daemon remembers.
///
/// Process memory, and that is the whole lifetime: a daemon restart forgets
/// every token, and the launches they named cannot have survived it
/// (ADR 0007 L6). An event bearing a forgotten token is late evidence, dropped
/// with diagnostics and never correlated heuristically.
#[derive(Default)]
pub struct LaunchTokens {
    minted: HashMap<LaunchToken, LaunchScope>,
}

/// The operating system could not supply randomness, so no token was minted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NoRandomness;

impl LaunchTokens {
    pub fn new() -> Self {
        Self::default()
    }

    /// Mint a token for one launch.
    ///
    /// Unlike an attach token this is not consumed by use and does not expire:
    /// a launch fires hooks for as long as it runs, and every one of them has
    /// to resolve to the same Session.
    pub fn mint(&mut self, scope: LaunchScope) -> Result<LaunchToken, NoRandomness> {
        let mut bytes = [0_u8; 16];
        getrandom::fill(&mut bytes).map_err(|_| NoRandomness)?;
        let token = LaunchToken(bytes);
        self.minted.insert(token, scope);
        Ok(token)
    }

    /// The launch a token names, if this daemon minted it.
    ///
    /// A read, never a consume. Resolution says only which launch an event
    /// belongs to; whether the fact it carries may be believed is decided
    /// afterwards, by the binding rules.
    pub fn resolve(&self, token: &LaunchToken) -> Option<LaunchScope> {
        self.minted.get(token).copied()
    }

    /// Forget a launch that never became one.
    ///
    /// A spawn that failed, or a durable start the store would not take, leaves
    /// a token naming a Session and Run that do not exist. It resolves to
    /// nothing useful and nothing can ever present it, so it is dropped rather
    /// than kept for the daemon's life.
    pub fn forget(&mut self, token: LaunchToken) {
        self.minted.remove(&token);
    }

    /// Forget the launch of a Run that is over.
    ///
    /// A token outlives its Run only as a way to be wrong. Evidence arriving
    /// afterwards is late evidence about a dead Run, which may claim nothing
    /// (ADR 0004 D5) — and once a continuation has replaced the Session's
    /// runtime, a token from the Run before it would let a provider process
    /// that outlived its episode contest the identity of the one that
    /// replaced it. It also bounds the map: without this, a daemon that runs
    /// for weeks holds one entry per launch it ever made.
    pub fn forget_run(&mut self, run: RunId) {
        self.minted.retain(|_, scope| scope.run != run);
    }

    pub fn outstanding(&self) -> usize {
        self.minted.len()
    }
}

impl LaunchToken {
    /// The form the injected command line and the hook wire carry: lowercase
    /// hex, so the value survives a shell word and a JSON string without an
    /// encoding question.
    pub fn to_wire(self) -> String {
        self.0.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    /// Read a token back, or refuse it.
    ///
    /// Canonical, deliberately: exactly the form `to_wire` emits, and no other.
    /// `from_str_radix` accepts a leading sign and either case, so `"+f"`,
    /// `"0F"` and `"0f"` would all decode to one byte and three distinct wire
    /// strings would name one token. The value is not authorization, so nothing
    /// escalates — but a decode that is not injective makes `to_wire` and
    /// `from_wire` stop being each other's inverse, and anything that later
    /// logs or compares the raw form would disagree with what resolution did.
    pub fn from_wire(raw: &str) -> Option<Self> {
        let lowercase_hex = |byte: u8| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte);
        if raw.len() != 32 || !raw.bytes().all(lowercase_hex) {
            return None;
        }
        let mut bytes = [0_u8; 16];
        for (index, byte) in bytes.iter_mut().enumerate() {
            *byte = u8::from_str_radix(raw.get(index * 2..index * 2 + 2)?, 16).ok()?;
        }
        Some(Self(bytes))
    }
}

/// Never prints the value. A token in a log is a correlation handle that
/// outlives the launch it names.
impl std::fmt::Debug for LaunchToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("LaunchToken(redacted)")
    }
}

/// The Corral-owned provider configuration written for one launch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InjectedSettings {
    path: PathBuf,
}

/// Why a launch could not be given its Corral-owned configuration.
#[derive(Debug)]
pub enum InjectionFailed {
    /// The relay the hook command line has to name could not be located. The
    /// launch is refused rather than started without hooks: a session that
    /// looks managed and reports nothing is worse than one that did not start.
    RelayUnresolvable(String),
    NoRandomness,
    Write(std::io::Error),
}

impl InjectedSettings {
    /// Where this Run's file lives.
    ///
    /// Derived from the `RunId` rather than remembered, so the party that
    /// learns a Run ended can find the file without holding a map that a
    /// restart would empty.
    pub fn path_for(launch_dir: &Path, run: RunId) -> PathBuf {
        launch_dir.join(format!("{FILE_PREFIX}{run}{FILE_SUFFIX}"))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Write the file one launch will be given, and publish it atomically.
    ///
    /// A partial write is never a file a provider can read: the content lands
    /// in a `.partial` beside it and is renamed into place, so a launch either
    /// gets a whole file or none. 0600 from creation, never chmod'd afterwards
    /// — a window where the file is readable is a window, however short.
    pub fn write(
        launch_dir: &Path,
        run: RunId,
        provider: KnownProvider,
        relay_command: &str,
    ) -> Result<Self, InjectionFailed> {
        let path = Self::path_for(launch_dir, run);
        let partial = launch_dir.join(format!("{FILE_PREFIX}{run}{FILE_SUFFIX}{PARTIAL_SUFFIX}"));
        let document = match provider {
            KnownProvider::Claude => super::claude::settings_document(relay_command),
        };

        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&partial)
            .map_err(InjectionFailed::Write)?;
        file.write_all(document.as_bytes())
            .and_then(|()| file.sync_all())
            .map_err(InjectionFailed::Write)?;
        drop(file);
        std::fs::rename(&partial, &path).map_err(InjectionFailed::Write)?;
        Ok(Self { path })
    }

    /// Remove a file whose Run's exit is established.
    ///
    /// Best-effort: a file that will not go away costs a stale artifact, and
    /// nothing about a Run's ending depends on it.
    pub fn remove_for(launch_dir: &Path, run: RunId) {
        let path = Self::path_for(launch_dir, run);
        match std::fs::remove_file(&path) {
            Ok(()) => debug!(%run, "the injected settings of a finished run were removed"),
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => {}
            Err(source) => warn!(%run, %source, "an injected settings file could not be removed"),
        }
    }
}

/// The injected hook command line, quoted for the shell a provider runs it in.
///
/// The relay is the installed `corral` binary, resolved as this daemon's own
/// sibling. A shell-local `PATH` must not decide which relay an account's
/// agents talk to, and skew is expected anyway: this file is written at launch
/// and invokes whatever is installed by the time an event fires
/// (ADR 0004 D1, D3).
pub fn relay_command(
    provider: KnownProvider,
    token: LaunchToken,
) -> Result<String, InjectionFailed> {
    let relay = sibling_relay().map_err(InjectionFailed::RelayUnresolvable)?;
    Ok(format!(
        "{} {RELAY_SUBCOMMAND} {RELAY_PROVIDER_FLAG} {} {RELAY_TOKEN_FLAG} {}",
        shell_word(&relay.to_string_lossy()),
        provider.as_str(),
        token.to_wire(),
    ))
}

/// Quote one word so a shell passes it through unchanged.
///
/// Single quotes, because inside them a shell interprets nothing at all; the
/// only thing that has to be handled is a single quote itself.
fn shell_word(raw: &str) -> String {
    format!("'{}'", raw.replace('\'', r"'\''"))
}

fn sibling_relay() -> Result<PathBuf, String> {
    let running = std::env::current_exe()
        .map_err(|source| format!("this daemon is not locatable: {source}"))?;
    let real = running.canonicalize().unwrap_or(running);
    let directory = real
        .parent()
        .ok_or_else(|| "this daemon has no directory".to_owned())?;
    usable_relay(&directory.join(CLIENT_BINARY))
}

/// Whether this path is something a provider's shell could actually run.
///
/// Asked here, where the refusal is actionable. A truncated or non-executable
/// `corral` — an interrupted install, a partial upgrade — would otherwise
/// compose a hook command that can only fail inside the provider's shell,
/// which is a session that looks managed and can never report. The client side
/// already asks the same question of the daemon binary for the same reason.
fn usable_relay(relay: &Path) -> Result<PathBuf, String> {
    let metadata = std::fs::metadata(relay)
        .map_err(|source| format!("{} is unusable: {source}", relay.display()))?;
    if !metadata.is_file() {
        return Err(format!("{} is not a regular file", relay.display()));
    }
    if metadata.permissions().mode() & 0o111 == 0 {
        return Err(format!("{} is not executable", relay.display()));
    }
    Ok(relay.to_path_buf())
}

/// What a startup sweep concluded about one Corral-owned file.
///
/// Named rather than boolean because the reasons are the decision: three
/// classes may be removed, and everything else is retained (grill Q10).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SweepVerdict {
    /// A write that never completed, so no provider ever read it.
    NeverPublished,
    /// No launch was ever committed under this name.
    NoLaunchCommitted,
    /// The owning Run is durably recorded as having exited.
    OwnerExited,
    /// The owning Run's fate is not established, or it is still open. Losing
    /// Corral's ownership is not proof the provider process is dead.
    OwnerUnverified,
    /// Corral's prefix over a name that does not say which Run owns it. There
    /// is no owner to check, so there is no evidence to destroy it on — and a
    /// name Corral cannot read is a name Corral should not act on.
    MalformedName,
    /// Not a file Corral wrote. Nothing here touches it.
    NotOurs,
}

/// Remove the Corral-owned launch files whose destruction is evidenced, and
/// only those.
///
/// `owner_exited` answers, for one Run, whether the durable log records an
/// established exit — `None` when the log has never heard of the Run at all.
/// It is a parameter rather than a store call so that this module owns the
/// artifact rules and the store keeps owning durable truth.
pub fn sweep_launch_dir(launch_dir: &Path, owner_exited: impl Fn(RunId) -> Option<bool>) {
    let Ok(entries) = std::fs::read_dir(launch_dir) else {
        // A directory that is not there is a daemon that has launched nothing
        // yet. Nothing to sweep and nothing to report.
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(std::ffi::OsStr::to_str) else {
            continue;
        };
        let verdict = classify(name, &owner_exited);
        match verdict {
            SweepVerdict::NeverPublished
            | SweepVerdict::NoLaunchCommitted
            | SweepVerdict::OwnerExited => {
                if let Err(source) = std::fs::remove_file(&path) {
                    warn!(path = %path.display(), %source, "a stale launch file could not be removed");
                } else {
                    debug!(path = %path.display(), ?verdict, "a stale launch file was removed");
                }
            }
            // Logged rather than deleted, and logged because a person looking
            // at a directory that never empties deserves to know why.
            SweepVerdict::OwnerUnverified | SweepVerdict::MalformedName => {
                debug!(
                    path = %path.display(),
                    ?verdict,
                    "a Corral-owned launch file is retained without evidence to remove it",
                );
            }
            SweepVerdict::NotOurs => {}
        }
    }
}

/// Decide one file's fate from its name and the log.
fn classify(name: &str, owner_exited: &impl Fn(RunId) -> Option<bool>) -> SweepVerdict {
    let Some(rest) = name.strip_prefix(FILE_PREFIX) else {
        return SweepVerdict::NotOurs;
    };
    if let Some(run) = rest.strip_suffix(PARTIAL_FILE_SUFFIX) {
        return if run.parse::<RunId>().is_ok() {
            SweepVerdict::NeverPublished
        } else {
            SweepVerdict::MalformedName
        };
    }
    let Some(run) = rest.strip_suffix(FILE_SUFFIX) else {
        return SweepVerdict::MalformedName;
    };
    let Ok(run) = run.parse::<RunId>() else {
        return SweepVerdict::MalformedName;
    };
    match owner_exited(run) {
        // The window this covers is narrow and deliberate: the file is written
        // before the spawn, and `RunStarted` commits after it. A daemon that
        // died in between leaves a file naming a Run the log never heard of,
        // and that is the "creation remnant for which no launch was committed"
        // class — not an Unverifiable owner, which is retained below.
        None => SweepVerdict::NoLaunchCommitted,
        Some(true) => SweepVerdict::OwnerExited,
        Some(false) => SweepVerdict::OwnerUnverified,
    }
}

impl std::fmt::Display for InjectionFailed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::RelayUnresolvable(detail) => {
                write!(f, "the Corral hook relay could not be located: {detail}")
            }
            Self::NoRandomness => {
                f.write_str("the operating system could not supply randomness for a launch token")
            }
            Self::Write(source) => {
                write!(
                    f,
                    "the launch's settings file could not be written: {source}"
                )
            }
        }
    }
}

impl std::error::Error for InjectionFailed {}

#[cfg(test)]
#[path = "launch_tests.rs"]
mod tests;
