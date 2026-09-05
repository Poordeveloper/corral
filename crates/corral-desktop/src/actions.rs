//! What the person may ask for, and of which daemon.
//!
//! Every action names the capability it needs and is absent, not disabled,
//! when the hello lacks it (PR9 plan, D2). New Session is a provider, an
//! explicit working directory, and the provider's own arguments — never a
//! raw command: `corral new -- <cmd>` is PR3's walking-skeleton harness, not
//! a promise that Corral manages arbitrary programs (round 2, Q8). What a
//! surface may check in advance is what it can see for itself; the provider
//! grammar is the daemon's (ADR 0012), and its refusal is shown in its words.

use std::fmt;
use std::path::{Path, PathBuf};

use corral_client::launch::{LaunchSite, Requested};

use crate::bridge::Capabilities;

/// The agents Corral composes a command for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Provider {
    ClaudeCode,
    Codex,
}

impl Provider {
    pub const ALL: [Self; 2] = [Self::ClaudeCode, Self::Codex];

    /// The product, as a person writes it.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::ClaudeCode => "Claude Code",
            Self::Codex => "Codex",
        }
    }

    /// The name the daemon knows it by.
    #[must_use]
    pub fn wire_name(self) -> &'static str {
        match self {
            Self::ClaudeCode => "claude",
            Self::Codex => "codex",
        }
    }
}

/// The New Session form, as typed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewSessionForm {
    pub provider: Provider,
    pub working_directory: String,
    /// The provider's own arguments, split on whitespace as the TUI splits
    /// them. Quoting is not interpreted: the daemon is given words, never a
    /// line for something to interpret.
    pub arguments: String,
}

/// What this surface can see is wrong before it asks. The daemon's own
/// refusals — an unknown provider, an argument its grammar rejects — are not
/// anticipated here; they arrive in its words.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Preflight {
    WorkingDirectoryMissing,
    WorkingDirectoryRelative(String),
    WorkingDirectoryNotFound(String),
}

impl fmt::Display for Preflight {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WorkingDirectoryMissing => write!(f, "A working directory is needed."),
            Self::WorkingDirectoryRelative(path) => {
                write!(f, "The working directory must be an absolute path: {path}")
            }
            Self::WorkingDirectoryNotFound(path) => {
                write!(f, "The working directory does not exist here: {path}")
            }
        }
    }
}

/// What the daemon will be asked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Launch {
    pub requested: Requested,
    pub site: LaunchSite,
}

impl NewSessionForm {
    /// A form for one provider, starting where this process is.
    #[must_use]
    pub fn here(provider: Provider) -> Self {
        Self {
            provider,
            working_directory: std::env::current_dir()
                .map(|path| path.to_string_lossy().into_owned())
                .unwrap_or_default(),
            arguments: String::new(),
        }
    }

    /// The request, once this surface's own checks pass.
    pub fn preflight(&self) -> Result<Launch, Preflight> {
        let directory = self.working_directory.trim();
        if directory.is_empty() {
            return Err(Preflight::WorkingDirectoryMissing);
        }
        let path = Path::new(directory);
        if !path.is_absolute() {
            return Err(Preflight::WorkingDirectoryRelative(directory.to_owned()));
        }
        if !path.is_dir() {
            return Err(Preflight::WorkingDirectoryNotFound(directory.to_owned()));
        }
        Ok(Launch {
            requested: Requested::Provider {
                name: self.provider.wire_name().to_owned(),
                args: self
                    .arguments
                    .split_whitespace()
                    .map(str::to_owned)
                    .collect(),
            },
            site: LaunchSite {
                working_directory: Some(PathBuf::from(directory)),
                // The size is the terminal view's to ask for once it exists;
                // the daemon supplies a first one.
                rows: None,
                cols: None,
            },
        })
    }
}

/// Which actions this daemon can serve.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Offered {
    pub new_session: bool,
    pub continue_in_corral: bool,
    pub acknowledge: bool,
}

impl Offered {
    #[must_use]
    pub fn by(capabilities: Capabilities) -> Self {
        Self {
            new_session: capabilities.managed_sessions,
            continue_in_corral: capabilities.managed_sessions,
            acknowledge: capabilities.attention,
        }
    }
}

#[cfg(test)]
#[path = "actions_tests.rs"]
mod tests;
