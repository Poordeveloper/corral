use corral_core::{Sealing, SemanticState};

use super::*;

const CLAUDE: &str = r#"
schema = 1
min_engine_version = 1
version = "2026.09.02.1"
provider = "claude"
sealed_versions = ["2.1.258"]

[[rule]]
id = "permission-prompt"
asserts = "needs_input"
region = "bottom_non_empty_lines"
lines = 12
all = ["Do you want to proceed?", "Esc to cancel"]
none = ["manual mode on"]
priority = 10
sealed_by = "docs/references/2026-09-02-pr8-attention-matrix.md#c2"

[[rule]]
id = "idle-prompt"
asserts = "turn_complete"
region = "bottom_non_empty_lines"
lines = 3
all = ["? for shortcuts"]
priority = 1
"#;

fn screen(rows: &[&str], title: &str) -> Screen {
    Screen {
        rows: rows.iter().map(|row| (*row).to_owned()).collect(),
        title: title.to_owned(),
    }
}

#[test]
fn a_manifest_parses_its_rules_and_their_seals() {
    let (manifest, refused) = parse(CLAUDE).expect("a valid manifest");
    assert!(refused.is_empty());
    assert_eq!(manifest.provider, "claude");
    assert_eq!(manifest.version, "2026.09.02.1");
    assert_eq!(manifest.rules.len(), 2);
    assert_eq!(manifest.rules[0].asserts, SemanticState::NeedsYou);
    assert_eq!(manifest.rules[0].sealing(), Sealing::Sealed);
    assert_eq!(manifest.rules[1].sealing(), Sealing::Unsealed);
}

/// Compatibility is per level (ADR 0015 D6): an unknown field is ignored, a
/// rule with an unknown word is refused alone, and a schema or engine
/// requirement above this build refuses the whole document.
#[test]
fn an_unknown_field_is_ignored_and_an_unknown_word_refuses_one_rule() {
    let text = CLAUDE.replace("priority = 1\n", "priority = 1\nfuture_field = true\n")
        + "\n[[rule]]\nid = \"later\"\nasserts = \"levitating\"\nregion = \"whole_screen\"\nall = [\"x\"]\n"
        + "\n[[rule]]\nid = \"elsewhere\"\nasserts = \"needs_input\"\nregion = \"hologram\"\nall = [\"x\"]\n";
    let (manifest, refused) = parse(&text).expect("still a manifest");
    assert_eq!(manifest.rules.len(), 2);
    assert_eq!(refused.len(), 2);
    assert_eq!(refused[0].rule, "later");
    assert_eq!(refused[1].rule, "elsewhere");
}

#[test]
fn a_schema_or_engine_requirement_above_this_build_refuses_the_document() {
    let newer_schema = CLAUDE.replace("schema = 1", "schema = 2");
    assert!(matches!(
        parse(&newer_schema),
        Err(ManifestRefused::Schema(2))
    ));
    let newer_engine = CLAUDE.replace("min_engine_version = 1", "min_engine_version = 9");
    assert!(matches!(
        parse(&newer_engine),
        Err(ManifestRefused::EngineVersion(9))
    ));
    assert!(matches!(
        parse("not = [toml"),
        Err(ManifestRefused::Malformed(_))
    ));
}

/// Gates are substring gates over the region's rows; `all`, `any`, `none`
/// mean what they say; the highest priority match is the reading.
#[test]
fn a_rule_reads_the_region_it_names_and_the_highest_priority_match_wins() {
    let (manifest, _) = parse(CLAUDE).expect("a valid manifest");
    let blocked = screen(
        &[
            "header",
            "",
            " Bash command",
            "   ls -la /tmp",
            " Do you want to proceed?",
            " ❯ 1. Yes",
            "   3. No",
            " Esc to cancel · Tab to amend",
        ],
        "✳ Claude Code",
    );
    let reading = evaluate(&manifest, &blocked, Some("2.1.258")).expect("a reading");
    assert_eq!(reading.rule, "permission-prompt");
    assert_eq!(reading.asserts, SemanticState::NeedsYou);
    assert_eq!(reading.sealing, Sealing::Sealed);

    let idle = screen(
        &[
            "❯ ",
            "",
            "  ⏸ manual mode on · ? for shortcuts · ← for agents",
        ],
        "✳ Claude Code",
    );
    let reading = evaluate(&manifest, &idle, None).expect("a reading");
    assert_eq!(reading.rule, "idle-prompt");
    assert_eq!(reading.sealing, Sealing::Unsealed);

    // The permission words as ordinary output, under the mode bar: the
    // `none` gate holds the rule off (matrix C9).
    let near_miss = screen(
        &[
            "  ⎿  Do you want to proceed?",
            "     1. Yes",
            "  Esc to cancel",
            "❯ ",
            "  ⏸ manual mode on · ? for shortcuts",
        ],
        "✳ Claude Code",
    );
    assert_eq!(
        evaluate(&manifest, &near_miss, None).map(|r| r.rule),
        Some("idle-prompt".to_owned())
    );
}

#[test]
fn the_title_region_reads_the_osc_title() {
    let text = r#"
schema = 1
min_engine_version = 1
version = "1"
provider = "codex"
[[rule]]
id = "action-required"
asserts = "needs_input"
region = "osc_title"
all = ["Action Required"]
"#;
    let (manifest, _) = parse(text).expect("a valid manifest");
    let blocked = screen(&["anything"], "[ ! ] Action Required | proj");
    assert_eq!(
        evaluate(&manifest, &blocked, None).map(|r| r.rule),
        Some("action-required".to_owned())
    );
    let idle = screen(&["anything"], "proj");
    assert_eq!(evaluate(&manifest, &idle, None), None);
}

/// Built-in manifests are the floor; an override replaces its provider's
/// built-in, and a refused override leaves the built-in standing and is
/// reported (ADR 0015 D6).
#[test]
fn an_override_replaces_its_provider_and_a_refused_one_is_reported() {
    let dir = std::env::temp_dir().join(format!(
        "corral-manifests-{}",
        corral_core::CorralSessionId::mint()
    ));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    std::fs::write(
        dir.join("claude.toml"),
        CLAUDE.replace("2026.09.02.1", "override"),
    )
    .expect("override");
    std::fs::write(dir.join("codex.toml"), "schema = 7\nprovider = \"codex\"\n")
        .expect("bad override");
    let built_in = [
        CLAUDE,
        "schema = 1\nmin_engine_version = 1\nversion = \"built-in\"\nprovider = \"codex\"\n",
    ];
    let loadout = load(&built_in, Some(&dir));
    assert_eq!(
        loadout.manifest("claude").map(|m| m.version.as_str()),
        Some("override")
    );
    assert_eq!(
        loadout.manifest("codex").map(|m| m.version.as_str()),
        Some("built-in")
    );
    assert_eq!(loadout.refused.len(), 1);
    assert!(loadout.refused[0].path.ends_with("codex.toml"));
}

/// A rule is sealed for the versions its manifest was measured on and no
/// others. Sealing that ignored the running version would let a rule measured
/// on one build assert a person-visible state on a build nobody looked at,
/// which is the inheritance grill Q13 forbids.
#[test]
fn a_sealed_rule_reads_unsealed_on_a_version_the_manifest_does_not_name() {
    let manifest = parse(
        r#"
schema = 1
min_engine_version = 1
version = "2026.09.03.1"
provider = "claude"
sealed_versions = ["2.1.258"]

[[rule]]
id = "blocked"
asserts = "needs_input"
region = "whole_screen"
all = ["Do you want to proceed?"]
sealed_by = "docs/evidence/pr8-attention-semantics-2026-09-03.md"
priority = 10
"#,
    )
    .expect("a usable manifest")
    .0;
    let screen = Screen {
        rows: vec!["Do you want to proceed?".to_owned()],
        title: String::new(),
    };

    assert_eq!(
        evaluate(&manifest, &screen, Some("2.1.258"))
            .expect("a reading")
            .sealing,
        Sealing::Sealed
    );
    for unmeasured in [Some("2.1.259"), Some("2.1.257"), Some(""), None] {
        assert_eq!(
            evaluate(&manifest, &screen, unmeasured)
                .expect("a reading")
                .sealing,
            Sealing::Unsealed,
            "{unmeasured:?}"
        );
    }
}

/// A manifest naming no versions seals nothing, whatever its rules say.
#[test]
fn a_manifest_that_names_no_version_seals_nothing() {
    let manifest = parse(
        r#"
schema = 1
min_engine_version = 1
version = "2026.09.03.1"
provider = "claude"

[[rule]]
id = "blocked"
asserts = "needs_input"
region = "whole_screen"
all = ["Do you want to proceed?"]
sealed_by = "somewhere"
priority = 10
"#,
    )
    .expect("a usable manifest")
    .0;
    let screen = Screen {
        rows: vec!["Do you want to proceed?".to_owned()],
        title: String::new(),
    };

    assert_eq!(
        evaluate(&manifest, &screen, Some("2.1.258"))
            .expect("a reading")
            .sealing,
        Sealing::Unsealed
    );
}
