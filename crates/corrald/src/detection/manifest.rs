//! Detection manifests: Corral-owned TOML, one per provider, loaded once at
//! daemon start (ADR 0015 D6).
//!
//! A manifest is data about what a provider's screen looks like in each
//! state, and the compatibility rules are per level: an unknown field is
//! ignored, a rule with an unknown word is refused alone, and a `schema` or
//! `min_engine_version` above this build refuses the whole document. A rule
//! asserts a user-visible state only when sealed — `sealed_by` names the
//! matrix evidence — and an unsealed rule loads, is evaluated, and asserts
//! nothing (grill Q14).

use std::path::{Path, PathBuf};

use corral_core::{Sealing, SemanticState};
use toml_edit::{DocumentMut, Item, Table, Value};

/// The manifest format this build reads.
pub const SCHEMA: u32 = 1;
/// The rule engine this build is; a manifest may require at least this.
pub const ENGINE_VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Manifest {
    pub provider: String,
    pub version: String,
    /// The provider versions this manifest's rules were measured on. A rule
    /// asserts a person-visible state only while the runtime drawing the
    /// screen is one of them; every other version reads the same rules and
    /// asserts nothing (grill Q13, ADR 0015 D6).
    pub sealed_versions: Vec<String>,
    pub rules: Vec<Rule>,
}

impl Manifest {
    /// Whether this manifest's evidence covers the version drawing the screen.
    /// A version Corral could not establish is not one of them.
    #[must_use]
    pub fn seals(&self, version: Option<&str>) -> bool {
        version.is_some_and(|version| {
            self.sealed_versions
                .iter()
                .any(|sealed| sealed.as_str() == version)
        })
    }
}

/// One rule: a region of the screen, substring gates over it, and the state
/// a match asserts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rule {
    pub id: String,
    pub asserts: SemanticState,
    pub region: Region,
    pub all: Vec<String>,
    pub any: Vec<String>,
    pub none: Vec<String>,
    pub priority: i64,
    /// The matrix evidence that sealed this rule, when any did.
    pub sealed_by: Option<String>,
}

impl Rule {
    #[must_use]
    pub fn sealing(&self) -> Sealing {
        match self.sealed_by {
            Some(_) => Sealing::Sealed,
            None => Sealing::Unsealed,
        }
    }
}

/// Where on the screen a rule looks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Region {
    WholeScreen,
    /// The last `n` rows that hold anything.
    BottomNonEmptyLines(usize),
    /// The OSC 0 title the child set.
    OscTitle,
}

/// A rule this build could not take, and why. The rest of the manifest
/// stands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuleRefused {
    pub rule: String,
    pub reason: String,
}

/// A document this build could not take at all.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ManifestRefused {
    Malformed(String),
    /// A `schema` above `SCHEMA`.
    Schema(u32),
    /// A `min_engine_version` above `ENGINE_VERSION`.
    EngineVersion(u32),
    MissingProvider,
}

impl std::fmt::Display for ManifestRefused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Malformed(reason) => write!(f, "not a manifest: {reason}"),
            Self::Schema(schema) => write!(
                f,
                "schema {schema} is newer than this build reads ({SCHEMA})"
            ),
            Self::EngineVersion(needed) => {
                write!(
                    f,
                    "needs rule engine {needed}; this build is {ENGINE_VERSION}"
                )
            }
            Self::MissingProvider => f.write_str("names no provider"),
        }
    }
}

/// What the screen thread hands a rule: the rows as text, and the title.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Screen {
    pub rows: Vec<String>,
    pub title: String,
}

/// A rule's match, as the ledger will hear about it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reading {
    pub rule: String,
    pub asserts: SemanticState,
    pub sealing: Sealing,
    pub manifest_version: String,
}

/// Read one manifest.
pub fn parse(text: &str) -> Result<(Manifest, Vec<RuleRefused>), ManifestRefused> {
    let document: DocumentMut = text
        .parse()
        .map_err(|source: toml_edit::TomlError| ManifestRefused::Malformed(source.to_string()))?;
    let schema = integer(document.get("schema")).unwrap_or(1);
    if schema > u64::from(SCHEMA) {
        return Err(ManifestRefused::Schema(
            u32::try_from(schema).unwrap_or(u32::MAX),
        ));
    }
    let engine = integer(document.get("min_engine_version")).unwrap_or(1);
    if engine > u64::from(ENGINE_VERSION) {
        return Err(ManifestRefused::EngineVersion(
            u32::try_from(engine).unwrap_or(u32::MAX),
        ));
    }
    let provider = string(document.get("provider")).ok_or(ManifestRefused::MissingProvider)?;
    let version = string(document.get("version")).unwrap_or_else(|| "unversioned".to_owned());
    let sealed_versions = strings(document.get("sealed_versions"));
    let mut rules = Vec::new();
    let mut refused = Vec::new();
    if let Some(Item::ArrayOfTables(tables)) = document.get("rule") {
        for table in tables.iter() {
            match rule(table) {
                Ok(rule) => rules.push(rule),
                Err(refusal) => refused.push(refusal),
            }
        }
    }
    Ok((
        Manifest {
            provider,
            version,
            sealed_versions,
            rules,
        },
        refused,
    ))
}

fn rule(table: &Table) -> Result<Rule, RuleRefused> {
    let id = string(table.get("id")).unwrap_or_else(|| "unnamed".to_owned());
    let refuse = |reason: String| RuleRefused {
        rule: id.clone(),
        reason,
    };
    let asserts = match string(table.get("asserts")).as_deref() {
        Some("needs_input") => SemanticState::NeedsYou,
        Some("turn_complete") => SemanticState::Ready,
        Some("working") => SemanticState::Working,
        other => {
            return Err(refuse(format!(
                "asserts {other:?} is not a state this build names"
            )));
        }
    };
    let region = match string(table.get("region")).as_deref() {
        Some("whole_screen") => Region::WholeScreen,
        Some("bottom_non_empty_lines") => {
            let lines = integer(table.get("lines")).unwrap_or(1);
            Region::BottomNonEmptyLines(usize::try_from(lines).unwrap_or(1).max(1))
        }
        Some("osc_title") => Region::OscTitle,
        other => {
            return Err(refuse(format!(
                "region {other:?} is not one this build reads"
            )));
        }
    };
    Ok(Rule {
        id: id.clone(),
        asserts,
        region,
        all: strings(table.get("all")),
        any: strings(table.get("any")),
        none: strings(table.get("none")),
        priority: table
            .get("priority")
            .and_then(Item::as_integer)
            .unwrap_or(0),
        sealed_by: string(table.get("sealed_by")),
    })
}

fn integer(item: Option<&Item>) -> Option<u64> {
    item.and_then(Item::as_integer)
        .and_then(|value| u64::try_from(value).ok())
}

fn string(item: Option<&Item>) -> Option<String> {
    item.and_then(Item::as_str).map(str::to_owned)
}

fn strings(item: Option<&Item>) -> Vec<String> {
    item.and_then(Item::as_array)
        .map(|array| {
            array
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

/// The highest-priority rule the screen satisfies, if any.
///
/// Every rule is evaluated, sealed or not: an unsealed match is a reading the
/// ledger will refuse to act on, and one the journal can still count.
#[must_use]
pub fn evaluate(manifest: &Manifest, screen: &Screen, version: Option<&str>) -> Option<Reading> {
    manifest
        .rules
        .iter()
        .filter(|rule| matches(rule, screen))
        .max_by_key(|rule| rule.priority)
        .map(|rule| Reading {
            rule: rule.id.clone(),
            asserts: rule.asserts,
            // Both halves, and the version is the half a rule cannot carry:
            // a rule sealed on the build it was measured on says nothing
            // about the build actually drawing this screen (grill Q13).
            sealing: if rule.sealing() == Sealing::Sealed && manifest.seals(version) {
                Sealing::Sealed
            } else {
                Sealing::Unsealed
            },
            manifest_version: manifest.version.clone(),
        })
}

fn matches(rule: &Rule, screen: &Screen) -> bool {
    let region: Vec<&str> = match rule.region {
        Region::WholeScreen => screen.rows.iter().map(String::as_str).collect(),
        Region::BottomNonEmptyLines(n) => {
            let mut rows: Vec<&str> = screen
                .rows
                .iter()
                .map(String::as_str)
                .filter(|row| !row.trim().is_empty())
                .collect();
            let keep = rows.len().saturating_sub(n);
            rows.drain(..keep);
            rows
        }
        Region::OscTitle => vec![screen.title.as_str()],
    };
    let contains = |needle: &str| region.iter().any(|row| row.contains(needle));
    rule.all.iter().all(|needle| contains(needle))
        && (rule.any.is_empty() || rule.any.iter().any(|needle| contains(needle)))
        && !rule.none.iter().any(|needle| contains(needle))
}

/// An override this build could not take. Reported, never silently ignored.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OverrideRefused {
    pub path: PathBuf,
    pub reason: String,
}

/// Every manifest this daemon runs with, by provider.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Loadout {
    manifests: Vec<Manifest>,
    pub refused: Vec<OverrideRefused>,
    pub rules_refused: Vec<RuleRefused>,
}

impl Loadout {
    #[must_use]
    pub fn manifest(&self, provider: &str) -> Option<&Manifest> {
        self.manifests
            .iter()
            .find(|manifest| manifest.provider == provider)
    }
}

/// Built-in manifests first, then whatever the override directory holds; an
/// override replaces its provider's built-in, and a refused one leaves the
/// built-in standing.
#[must_use]
pub fn load(built_in: &[&str], override_dir: Option<&Path>) -> Loadout {
    let mut loadout = Loadout::default();
    for text in built_in {
        match parse(text) {
            Ok((manifest, refused)) => {
                loadout.rules_refused.extend(refused);
                loadout.manifests.push(manifest);
            }
            Err(refused) => loadout.refused.push(OverrideRefused {
                path: PathBuf::from("<built-in>"),
                reason: refused.to_string(),
            }),
        }
    }
    let Some(dir) = override_dir else {
        return loadout;
    };
    let Ok(entries) = std::fs::read_dir(dir) else {
        return loadout;
    };
    let mut paths: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "toml"))
        .collect();
    paths.sort();
    for path in paths {
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(source) => {
                loadout.refused.push(OverrideRefused {
                    path,
                    reason: source.to_string(),
                });
                continue;
            }
        };
        match parse(&text) {
            Ok((manifest, refused)) => {
                loadout.rules_refused.extend(refused);
                loadout
                    .manifests
                    .retain(|held| held.provider != manifest.provider);
                loadout.manifests.push(manifest);
            }
            Err(refused) => loadout.refused.push(OverrideRefused {
                path,
                reason: refused.to_string(),
            }),
        }
    }
    loadout
}

#[cfg(test)]
#[path = "manifest_tests.rs"]
mod tests;
