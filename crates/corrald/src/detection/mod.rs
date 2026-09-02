//! Screen detection: versioned manifest data over the emulator the daemon owns
//! (ADR 0015 D6).

mod manifest;

pub use manifest::{
    ENGINE_VERSION, Loadout, Manifest, ManifestRefused, OverrideRefused, Reading, Region, Rule,
    RuleRefused, SCHEMA, Screen, evaluate, load, parse,
};
