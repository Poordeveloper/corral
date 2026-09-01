//! The closed list of conditions that refuse a write.
//!
//! "Merge ambiguity fails safe" is not a mood (ADR 0013 D4): it is this list,
//! sealed for the provider versions the matrix names, and a condition outside
//! it fails closed to the same behavior. On any of them: no write, the
//! provider enters Limited awareness, and the cause is surfaced once with
//! something the user can act on. Never an overwrite, never a retry loop.

use std::fmt;
use std::path::{Path, PathBuf};

use serde_json::Value;

/// Why Corral did not write.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Trigger {
    /// The file is not the format the provider documents. Measured
    /// consequence, both directions: Claude drops every setting in an invalid
    /// `settings.json` silently, and Codex refuses to start at all on an
    /// invalid `config.toml`.
    Unparseable { detail: String },
    /// A path the merge must traverse holds a type the merge cannot use — a
    /// `hooks` that is not an object, a `notify` that is not an array.
    IncompatibleStructure { at: String },
    /// A Corral-owned entry declares a version this binary does not
    /// understand. An older Corral never rewrites what a newer one wrote.
    NewerIntegrationVersion { version: u32 },
    /// The file or its directory cannot be written safely.
    NotWritable { detail: String },
    /// The file changed between the read this operation reasoned about and
    /// the rename that would have replaced it. Corral loses that race on
    /// purpose (ADR 0013 D3).
    ChangedWhileWriting,
    /// The content Corral was about to publish does not parse. The last gate
    /// before an atomic replacement, and the one that keeps a Corral bug from
    /// reaching a user's provider as a broken file (grill Q2′).
    CandidateRejected { detail: String },
    /// Claude's own semantics silence every hook, at a layer Corral can read.
    /// Never overridden at global scope: a user who turned hooks off gets
    /// honest Limited awareness, not a silently re-enabled hooks system.
    HooksDisabled { layer: PathBuf },
    /// Codex's single notifier slot holds something that is not Corral's.
    /// Preserved, degraded, explained — never overwritten to obtain
    /// awareness (ADR 0013 D7).
    NotifierOccupied,
    /// Corral could not work out how to name itself, so there is no entry to
    /// write. Not a fact about the user's file — D4's rule that a condition
    /// outside the list fails closed to the same behavior is why it is a
    /// trigger rather than an error the user's configuration pays for.
    NotResolvable { detail: String },
}

impl fmt::Display for Trigger {
    /// Phrased for a person reading why Corral is not integrated. Every arm
    /// says what Corral found and what it did *not* do, because the thing a
    /// user needs to know first is that their file is untouched.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unparseable { detail } => {
                write!(
                    formatter,
                    "the configuration file could not be read ({detail}); it was left unchanged"
                )
            }
            Self::IncompatibleStructure { at } => write!(
                formatter,
                "`{at}` in the configuration file is not the shape Corral knows how to merge into; it was left unchanged"
            ),
            Self::NewerIntegrationVersion { version } => write!(
                formatter,
                "the configuration file carries a Corral integration (version {version}) newer than this Corral understands; it was left unchanged"
            ),
            Self::NotWritable { detail } => {
                write!(
                    formatter,
                    "the configuration file cannot be written ({detail})"
                )
            }
            Self::ChangedWhileWriting => write!(
                formatter,
                "the configuration file changed while Corral was writing it; Corral stopped rather than overwrite the other change"
            ),
            Self::CandidateRejected { detail } => write!(
                formatter,
                "Corral's own edit did not pass its check ({detail}); the configuration file was left unchanged"
            ),
            Self::HooksDisabled { layer } => write!(
                formatter,
                "hooks are turned off by `disableAllHooks` in {}; Corral did not override that",
                layer.display()
            ),
            Self::NotifierOccupied => write!(
                formatter,
                "another notifier is configured in Codex; Corral did not replace it"
            ),
            Self::NotResolvable { detail } => write!(
                formatter,
                "Corral could not work out how a provider would invoke it ({detail}); nothing was written"
            ),
        }
    }
}

/// Whether Claude's hooks are silenced by a layer Corral can read.
///
/// Measured 2026-09-02: `disableAllHooks: true` at *any* of the four effective
/// layers silences every hook, and Corral writes only the user layer. Two of
/// the four — a project's `settings.json` and its `settings.local.json` — are
/// per-project and unknowable when integration is installed; they are why the
/// integration's health is evidence rather than an assumption, and are a
/// recorded M1 limitation rather than something this check pretends to cover.
pub fn hooks_silenced(user_layer: &Path, user_settings: &Value) -> Option<Trigger> {
    if disables_hooks(user_settings) {
        return Some(Trigger::HooksDisabled {
            layer: user_layer.to_path_buf(),
        });
    }
    let managed = Path::new(MANAGED_SETTINGS);
    let raw = std::fs::read_to_string(managed).ok()?;
    let parsed: Value = serde_json::from_str(&raw).ok()?;
    disables_hooks(&parsed).then(|| Trigger::HooksDisabled {
        layer: managed.to_path_buf(),
    })
}

fn disables_hooks(settings: &Value) -> bool {
    settings
        .get(crate::provider::claude_integration::DISABLE_ALL_HOOKS)
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

/// Where an organization's Claude policy lives on this platform.
///
/// Read and never written. A policy file is the administrator's, and Corral
/// refusing to install against it is the whole interaction Corral has with
/// it.
#[cfg(target_os = "macos")]
const MANAGED_SETTINGS: &str = "/Library/Application Support/ClaudeCode/managed-settings.json";
#[cfg(not(target_os = "macos"))]
const MANAGED_SETTINGS: &str = "/etc/claude-code/managed-settings.json";
