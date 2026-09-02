use super::*;

use std::path::PathBuf;

fn path(raw: &str) -> PathBuf {
    PathBuf::from(raw)
}

/// The shapes measured 2026-09-02, one per channel, verbatim.
#[test]
fn every_measured_provider_shape_is_recognized() {
    let measured = [
        // macOS, Claude local channel: the true executable behind the symlink
        // the process was invoked by.
        (
            "/Users/someone/.claude/local/node_modules/@anthropic-ai/claude-code/bin/claude.exe",
            KnownProvider::Claude,
        ),
        // Linux, Claude native installer: a versioned file behind a symlink.
        (
            "/root/.local/share/claude/versions/2.1.252",
            KnownProvider::Claude,
        ),
        // macOS, Codex npm channel: the native child of the node wrapper.
        (
            "/Users/someone/.local/node/lib/node_modules/@openai/codex/node_modules/@openai/codex-darwin-arm64/vendor/aarch64-apple-darwin/bin/codex",
            KnownProvider::Codex,
        ),
        // Linux, Codex npm channel: the same shape.
        (
            "/usr/local/lib/node_modules/@openai/codex/node_modules/@openai/codex-linux-x64/vendor/x86_64-unknown-linux-musl/bin/codex",
            KnownProvider::Codex,
        ),
    ];

    for (executable, expected) in measured {
        assert_eq!(
            provider_of(&path(executable)),
            Some(expected),
            "{executable}",
        );
    }
}

/// A versioned Claude binary is recognized by the symlink's name, not the
/// version, so a Claude release does not silently stop being recognized.
#[test]
fn a_version_in_the_path_is_not_part_of_recognition() {
    assert_eq!(
        provider_of(&path("/root/.local/share/claude/versions/9.9.9")),
        Some(KnownProvider::Claude),
    );
}

/// A program merely containing a provider's name is a different program.
#[test]
fn a_similarly_named_program_is_not_a_provider() {
    assert_eq!(provider_of(&path("/usr/bin/claude-helper")), None);
    assert_eq!(provider_of(&path("/usr/bin/codexctl")), None);
    assert_eq!(provider_of(&path("/usr/bin/my-claude")), None);
}

/// The runtime a provider may sit one hop below is not itself a provider.
/// Measured: Codex's npm entry is a node script that spawns the real agent.
#[test]
fn a_language_runtime_is_a_launcher_and_never_the_provider() {
    assert_eq!(provider_of(&path("/usr/local/bin/node")), None);
    assert!(is_provider_launcher(&path("/usr/local/bin/node")));
    assert!(!is_provider_launcher(&path("/usr/local/bin/codex")));
}

/// Providers spawn children that are not the agent. Recognition never says
/// yes to one.
#[test]
fn a_child_a_provider_spawns_is_not_a_provider() {
    assert_eq!(provider_of(&path("/usr/bin/git")), None);
    assert_eq!(provider_of(&path("/bin/sh")), None);
    assert_eq!(provider_of(&path("/bin/dash")), None);
}

/// Recognition reads a path and asserts nothing about paths it cannot read.
#[test]
fn a_path_with_no_file_name_recognizes_nothing() {
    assert_eq!(provider_of(&path("/")), None);
    assert_eq!(provider_of(&path("")), None);
}
