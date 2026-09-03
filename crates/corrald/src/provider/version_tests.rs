use std::path::Path;

use super::*;

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("corral-version-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch");
    dir
}

/// The versioned channel carries the version in the path the recognizer
/// already seals (grill Q12, tier 1); nothing is read.
#[test]
fn a_versioned_claude_path_names_its_version() {
    let found = installed_version(
        KnownProvider::Claude,
        Path::new("/home/x/.local/share/claude/versions/2.1.258/claude"),
    );
    assert_eq!(found.map(|v| v.version), Some("2.1.258".to_owned()));
}

/// The local channel is a script beside a `node_modules` tree whose package
/// carries the version (tier 2): read, with the metadata's own time kept
/// so a change after a process started can be told apart.
#[test]
fn the_local_claude_channel_reads_its_package() {
    let root = scratch("claude-local");
    let package = root.join("node_modules/@anthropic-ai/claude-code");
    std::fs::create_dir_all(&package).expect("package dir");
    std::fs::write(
        package.join("package.json"),
        r#"{"name":"@anthropic-ai/claude-code","version":"2.1.258"}"#,
    )
    .expect("package");
    std::fs::write(root.join("claude"), "#!/bin/bash\nexec node ...\n").expect("script");
    let found = installed_version(KnownProvider::Claude, &root.join("claude")).expect("found");
    assert_eq!(found.version, "2.1.258");
    assert!(found.metadata_changed_at.is_some());
}

/// npm's layout: `bin/codex.js` under the package root whose `package.json`
/// names the version; the executable a shell resolves is a symlink to it.
#[test]
fn the_npm_codex_channel_reads_its_package_root() {
    let root = scratch("codex-npm");
    let package = root.join("lib/node_modules/@openai/codex");
    std::fs::create_dir_all(package.join("bin")).expect("package dir");
    std::fs::write(
        package.join("package.json"),
        r#"{"name":"@openai/codex","version":"0.152.0"}"#,
    )
    .expect("package");
    std::fs::write(package.join("bin/codex.js"), "#!/usr/bin/env node\n").expect("bin");
    let found =
        installed_version(KnownProvider::Codex, &package.join("bin/codex.js")).expect("found");
    assert_eq!(found.version, "0.152.0");
}

/// Metadata that cannot be bound to the runtime seals nothing: a path this
/// build has no shape for answers `None`, never a guess.
#[test]
fn an_unknown_shape_answers_nothing() {
    assert_eq!(
        installed_version(KnownProvider::Claude, Path::new("/usr/bin/claude")),
        None
    );
    assert_eq!(
        installed_version(KnownProvider::Codex, Path::new("/opt/codex")),
        None
    );
}

/// A version is bound to a process only if its metadata predates the
/// process (grill Q12, tier 2).
#[test]
fn metadata_newer_than_the_process_is_not_bound_to_it() {
    let earlier = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_000);
    let later = earlier + std::time::Duration::from_secs(60);
    let found = InstalledVersion {
        version: "2.1.258".to_owned(),
        metadata_changed_at: Some(later),
    };
    assert_eq!(found.bound_to(earlier), None);
    assert_eq!(
        found
            .bound_to(later + std::time::Duration::from_secs(1))
            .as_deref(),
        Some("2.1.258")
    );
    let pathbound = InstalledVersion {
        version: "2.1.258".to_owned(),
        metadata_changed_at: None,
    };
    assert_eq!(pathbound.bound_to(earlier).as_deref(), Some("2.1.258"));
}
