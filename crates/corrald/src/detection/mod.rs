//! Screen detection: versioned manifest data over the emulator the daemon owns
//! (ADR 0015 D6).

pub(crate) mod manifest;

use std::path::Path;

pub use manifest::{Loadout, Manifest, Screen, evaluate, load};

/// The manifests compiled into this build, one per provider.
const BUILT_IN: [&str; 2] = [
    include_str!("../../manifests/claude.toml"),
    include_str!("../../manifests/codex.toml"),
];

/// The built-in manifests, with whatever `override_dir` replaces. Refusals
/// are reported in the loadout, never swallowed.
#[must_use]
pub fn load_built_in(override_dir: Option<&Path>) -> Loadout {
    let loadout = load(&BUILT_IN, override_dir);
    for refused in &loadout.refused {
        tracing::warn!(path = %refused.path.display(), reason = %refused.reason, "a detection manifest was refused");
    }
    for refused in &loadout.rules_refused {
        tracing::warn!(rule = %refused.rule, reason = %refused.reason, "a detection rule was refused");
    }
    loadout
}
