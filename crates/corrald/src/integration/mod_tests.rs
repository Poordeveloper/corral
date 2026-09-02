use super::*;

use std::os::unix::fs::PermissionsExt as _;
use std::path::Path;
use std::sync::atomic::{AtomicU32, Ordering};

use crate::provider::launch::RelayInvocation;

static COUNTER: AtomicU32 = AtomicU32::new(0);

/// A provider home and a Corral state directory on real files.
///
/// Real files rather than an in-memory shim: atomic rename, the mode a
/// created file is born with, the backup landing on disk, and noticing that
/// another writer replaced the file are the behaviours under test, and none
/// of them exist without a filesystem.
struct Scratch {
    root: PathBuf,
}

impl Scratch {
    fn new(name: &str) -> Self {
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "corral-integration-{}-{unique}-{name}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("state")).expect("create the scratch directory");
        Self { root }
    }

    fn state_dir(&self) -> PathBuf {
        self.root.join("state")
    }

    /// A target under this scratch home, bypassing account-database
    /// resolution: what is under test is the merge, not where the file lives.
    fn target(&self, provider: KnownProvider) -> Target {
        let (config_target, path) = match provider {
            KnownProvider::Claude => (
                ConfigTarget::ClaudeUserSettings,
                self.root.join(".claude").join("settings.json"),
            ),
            KnownProvider::Codex => (
                ConfigTarget::CodexUserConfig,
                self.root.join(".codex").join("config.toml"),
            ),
        };
        Target {
            provider,
            target: config_target,
            path,
        }
    }

    fn seed(&self, target: &Target, contents: &str) {
        let directory = target.path().parent().expect("a directory");
        std::fs::create_dir_all(directory).expect("create the provider directory");
        std::fs::write(target.path(), contents).expect("seed the configuration");
    }

    fn read(&self, target: &Target) -> String {
        std::fs::read_to_string(target.path()).expect("read the configuration")
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn relay(provider: KnownProvider) -> RelayInvocation {
    RelayInvocation::of_words(
        "/opt/corral/bin/corral",
        &[
            "hook-relay",
            "--provider",
            provider.as_str(),
            "--integration-version",
            "1",
        ],
    )
}

fn now() -> SystemTime {
    SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000)
}

/// The corpus, loaded rather than retyped: what the merge engine is proven
/// against is what people actually have, and a shape a test author imagined
/// would prove nothing (`crates/corrald/fixtures/provider-config/README.md`).
const A_USERS_CLAUDE_SETTINGS: &str =
    include_str!("../../fixtures/provider-config/claude-third-party-hooks.json");
const CLAUDE_HOOKS_DISABLED: &str =
    include_str!("../../fixtures/provider-config/claude-hooks-disabled.json");
const CLAUDE_NOT_JSON: &str = include_str!("../../fixtures/provider-config/claude-not-json.json");
const A_USERS_CODEX_CONFIG: &str =
    include_str!("../../fixtures/provider-config/codex-user-config.toml");
const CODEX_NOTIFIER_OCCUPIED: &str =
    include_str!("../../fixtures/provider-config/codex-notifier-occupied.toml");
const CODEX_NOTIFY_ILL_TYPED: &str =
    include_str!("../../fixtures/provider-config/codex-notify-ill-typed.toml");

#[test]
fn a_provider_with_no_configuration_file_is_not_installed() {
    let scratch = Scratch::new("absent");
    let target = scratch.target(KnownProvider::Claude);

    assert_eq!(
        status(&target, &relay(KnownProvider::Claude)),
        Standing::NotInstalled
    );
}

/// The measured fresh-install case: neither provider ships a configuration
/// file, so creating one is the ordinary path.
#[test]
fn installing_where_there_is_no_file_creates_one() {
    let scratch = Scratch::new("create");
    let target = scratch.target(KnownProvider::Claude);

    let standing = install(
        &target,
        &relay(KnownProvider::Claude),
        now(),
        &scratch.state_dir(),
    );

    assert_eq!(standing, Standing::Installed);
    let written = scratch.read(&target);
    serde_json::from_str::<serde_json::Value>(&written).expect("valid JSON");
    assert!(written.contains("hook-relay"));
}

#[test]
fn a_created_file_is_the_users_to_read() {
    let scratch = Scratch::new("mode");
    let target = scratch.target(KnownProvider::Claude);

    install(
        &target,
        &relay(KnownProvider::Claude),
        now(),
        &scratch.state_dir(),
    );

    let mode = std::fs::metadata(target.path())
        .expect("metadata")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o644);
}

#[test]
fn installing_preserves_everything_the_user_wrote() {
    let scratch = Scratch::new("preserve-claude");
    let target = scratch.target(KnownProvider::Claude);
    scratch.seed(&target, A_USERS_CLAUDE_SETTINGS);

    install(
        &target,
        &relay(KnownProvider::Claude),
        now(),
        &scratch.state_dir(),
    );

    let after: serde_json::Value =
        serde_json::from_str(&scratch.read(&target)).expect("valid JSON");
    let before: serde_json::Value =
        serde_json::from_str(A_USERS_CLAUDE_SETTINGS).expect("valid JSON");
    assert_eq!(after["env"], before["env"]);
    assert_eq!(after["permissions"], before["permissions"]);
    assert_eq!(after["model"], before["model"]);
    assert_eq!(
        after["hooks"]["Notification"][0],
        before["hooks"]["Notification"][0]
    );
}

/// Codex's file legally carries the user's comments, and the provider itself
/// preserves what it did not write. Corral matches that (grill Q3′).
#[test]
fn installing_into_codex_preserves_comments_and_layout() {
    let scratch = Scratch::new("preserve-codex");
    let target = scratch.target(KnownProvider::Codex);
    scratch.seed(&target, A_USERS_CODEX_CONFIG);

    let standing = install(
        &target,
        &relay(KnownProvider::Codex),
        now(),
        &scratch.state_dir(),
    );

    assert_eq!(standing, Standing::Installed);
    let after = scratch.read(&target);
    assert!(after.contains("# my codex setup"));
    assert!(after.contains(r#"trust_level = "trusted""#));
    after.parse::<toml_edit::DocumentMut>().expect("valid TOML");
}

#[test]
fn a_mutation_backs_up_what_it_is_about_to_change() {
    let scratch = Scratch::new("backup");
    let target = scratch.target(KnownProvider::Claude);
    scratch.seed(&target, A_USERS_CLAUDE_SETTINGS);

    install(
        &target,
        &relay(KnownProvider::Claude),
        now(),
        &scratch.state_dir(),
    );

    let backups = std::fs::read_dir(file::backup_dir(&scratch.state_dir()))
        .expect("the backup directory")
        .map(|entry| entry.expect("an entry").path())
        .collect::<Vec<_>>();
    assert_eq!(backups.len(), 1);
    assert_eq!(
        std::fs::read_to_string(&backups[0]).expect("read the backup"),
        A_USERS_CLAUDE_SETTINGS
    );
}

#[test]
fn uninstall_returns_the_file_to_what_the_user_had() {
    let scratch = Scratch::new("uninstall");
    let target = scratch.target(KnownProvider::Claude);
    scratch.seed(&target, A_USERS_CLAUDE_SETTINGS);
    install(
        &target,
        &relay(KnownProvider::Claude),
        now(),
        &scratch.state_dir(),
    );

    let standing = uninstall(
        &target,
        &relay(KnownProvider::Claude),
        now(),
        &scratch.state_dir(),
    );

    assert_eq!(standing, Standing::NotInstalled);
    let after: serde_json::Value =
        serde_json::from_str(&scratch.read(&target)).expect("valid JSON");
    let before: serde_json::Value =
        serde_json::from_str(A_USERS_CLAUDE_SETTINGS).expect("valid JSON");
    assert_eq!(after, before);
}

// Every trigger on D4's closed list, observed first-party.

/// Measured: Claude rejects JSONC, and an invalid settings file silently
/// drops every setting in it. Corral must never write into one.
#[test]
fn a_settings_file_with_comments_refuses_the_write_and_changes_nothing() {
    let scratch = Scratch::new("trigger-jsonc");
    let target = scratch.target(KnownProvider::Claude);
    scratch.seed(&target, CLAUDE_NOT_JSON);

    let standing = install(
        &target,
        &relay(KnownProvider::Claude),
        now(),
        &scratch.state_dir(),
    );

    assert!(matches!(
        standing,
        Standing::Refused(Trigger::Unparseable { .. })
    ));
    assert_eq!(scratch.read(&target), CLAUDE_NOT_JSON);
}

#[test]
fn a_hooks_key_of_the_wrong_shape_refuses_the_write() {
    let scratch = Scratch::new("trigger-structure");
    let target = scratch.target(KnownProvider::Claude);
    let odd = "{\"hooks\": \"none please\"}";
    scratch.seed(&target, odd);

    let standing = install(
        &target,
        &relay(KnownProvider::Claude),
        now(),
        &scratch.state_dir(),
    );

    assert!(matches!(
        standing,
        Standing::Refused(Trigger::IncompatibleStructure { .. })
    ));
    assert_eq!(scratch.read(&target), odd);
}

/// A user who turned hooks off gets honest Limited awareness, never a
/// silently re-enabled hooks system (ADR 0013 D4).
#[test]
fn hooks_disabled_by_the_user_refuses_the_write_and_is_never_overridden() {
    let scratch = Scratch::new("trigger-disabled");
    let target = scratch.target(KnownProvider::Claude);
    scratch.seed(&target, CLAUDE_HOOKS_DISABLED);

    let standing = install(
        &target,
        &relay(KnownProvider::Claude),
        now(),
        &scratch.state_dir(),
    );

    assert!(matches!(
        standing,
        Standing::Refused(Trigger::HooksDisabled { .. })
    ));
    assert_eq!(scratch.read(&target), CLAUDE_HOOKS_DISABLED);
}

/// Corral never overwrites a non-Corral Codex notifier merely to obtain
/// awareness (grill Q3).
#[test]
fn an_occupied_codex_notifier_refuses_the_write_and_is_preserved() {
    let scratch = Scratch::new("trigger-occupied");
    let target = scratch.target(KnownProvider::Codex);
    scratch.seed(&target, CODEX_NOTIFIER_OCCUPIED);

    let standing = install(
        &target,
        &relay(KnownProvider::Codex),
        now(),
        &scratch.state_dir(),
    );

    assert_eq!(standing, Standing::Refused(Trigger::NotifierOccupied));
    assert_eq!(scratch.read(&target), CODEX_NOTIFIER_OCCUPIED);
}

#[test]
fn an_entry_from_a_newer_corral_refuses_the_write() {
    let scratch = Scratch::new("trigger-newer");
    let target = scratch.target(KnownProvider::Claude);
    let newer = RelayInvocation::of_words(
        "/opt/corral/bin/corral",
        &[
            "hook-relay",
            "--provider",
            "claude",
            "--integration-version",
            "99",
        ],
    );
    install(&target, &newer, now(), &scratch.state_dir());
    let written = scratch.read(&target);

    let standing = install(
        &target,
        &relay(KnownProvider::Claude),
        now(),
        &scratch.state_dir(),
    );

    assert_eq!(
        standing,
        Standing::Refused(Trigger::NewerIntegrationVersion { version: 99 })
    );
    assert_eq!(scratch.read(&target), written);
}

/// Corral loses the race with a provider's own write on purpose: the provider
/// reserializes the whole document, so renaming over it would silently
/// discard the provider's change (measured 2026-09-02).
#[test]
fn a_file_replaced_while_corral_writes_aborts_rather_than_clobbering() {
    let scratch = Scratch::new("trigger-race");
    let target = scratch.target(KnownProvider::Claude);
    scratch.seed(&target, A_USERS_CLAUDE_SETTINGS);
    let original = file::read(&target).expect("read").expect("a file");
    // The provider publishes its own version between Corral's read and its
    // rename, exactly as a settings write from the running agent would.
    std::fs::write(target.path(), "{\"model\": \"written by the provider\"}")
        .expect("the provider's write");

    let outcome = file::replace(
        &target,
        &original,
        now(),
        &scratch.state_dir(),
        |document| document.install(&relay(KnownProvider::Claude)),
    );

    assert_eq!(outcome, Err(Trigger::ChangedWhileWriting));
    assert_eq!(
        scratch.read(&target),
        "{\"model\": \"written by the provider\"}"
    );
}

/// A directory Corral cannot write into refuses the write and changes
/// nothing.
///
/// The unwritability is a path that cannot be a directory rather than a mode
/// bit, because a mode bit does not bind every uid: root ignores directory
/// permissions, so a chmod-based test would assert nothing wherever Corral
/// runs as root — a container, a CI image — and would look like coverage
/// while providing none.
#[test]
fn a_configuration_directory_that_cannot_be_written_refuses_the_write() {
    let scratch = Scratch::new("trigger-unwritable");
    let target = scratch.target(KnownProvider::Claude);
    let directory = target.path().parent().expect("a directory");
    std::fs::create_dir_all(directory.parent().expect("a parent")).expect("create the parent");
    // Where the provider's directory would be, there is a file.
    std::fs::write(directory, b"not a directory").expect("occupy the path");

    let standing = install(
        &target,
        &relay(KnownProvider::Claude),
        now(),
        &scratch.state_dir(),
    );

    assert!(
        matches!(standing, Standing::Refused(Trigger::NotWritable { .. })),
        "{standing:?}",
    );
    assert_eq!(
        std::fs::read_to_string(directory).expect("read"),
        "not a directory",
        "corral wrote over something that was not its to touch",
    );
}

#[test]
fn installing_twice_is_one_installation() {
    let scratch = Scratch::new("idempotent");
    let target = scratch.target(KnownProvider::Claude);
    scratch.seed(&target, A_USERS_CLAUDE_SETTINGS);
    install(
        &target,
        &relay(KnownProvider::Claude),
        now(),
        &scratch.state_dir(),
    );
    let once = scratch.read(&target);

    let standing = install(
        &target,
        &relay(KnownProvider::Claude),
        now(),
        &scratch.state_dir(),
    );

    assert_eq!(standing, Standing::Installed);
    assert_eq!(scratch.read(&target), once);
}

/// A second install of an unchanged file writes nothing, so it takes no
/// backup either: the backups are a record of mutations, not of calls.
#[test]
fn an_installation_that_changes_nothing_takes_no_backup() {
    let scratch = Scratch::new("idempotent-backup");
    let target = scratch.target(KnownProvider::Claude);
    scratch.seed(&target, A_USERS_CLAUDE_SETTINGS);
    install(
        &target,
        &relay(KnownProvider::Claude),
        now(),
        &scratch.state_dir(),
    );

    install(
        &target,
        &relay(KnownProvider::Claude),
        now(),
        &scratch.state_dir(),
    );

    let backups = std::fs::read_dir(file::backup_dir(&scratch.state_dir()))
        .expect("the backup directory")
        .count();
    assert_eq!(backups, 1);
}

/// The provider ate Corral's entry in an ordinary race. That is the expected
/// path, and status names it as drift rather than as an untouched file.
#[test]
fn a_provider_rewrite_that_drops_corrals_entry_reads_as_missing() {
    let scratch = Scratch::new("drift-missing");
    let target = scratch.target(KnownProvider::Claude);
    scratch.seed(&target, A_USERS_CLAUDE_SETTINGS);
    install(
        &target,
        &relay(KnownProvider::Claude),
        now(),
        &scratch.state_dir(),
    );
    // What the provider's own reserialize leaves behind.
    scratch.seed(&target, A_USERS_CLAUDE_SETTINGS);

    assert_eq!(
        status(&target, &relay(KnownProvider::Claude)),
        Standing::NotInstalled
    );
}

#[test]
fn an_entry_an_older_corral_wrote_reads_as_drift_and_repairs_in_place() {
    let scratch = Scratch::new("drift-stale");
    let target = scratch.target(KnownProvider::Claude);
    let older = RelayInvocation::of_words(
        "/opt/corral/bin/corral",
        &["hook-relay", "--provider", "claude"],
    );
    scratch.seed(&target, A_USERS_CLAUDE_SETTINGS);
    install(&target, &older, now(), &scratch.state_dir());
    assert_eq!(
        status(&target, &relay(KnownProvider::Claude)),
        Standing::Drifted(RepairableDrift::OldRepresentation)
    );

    let standing = install(
        &target,
        &relay(KnownProvider::Claude),
        now(),
        &scratch.state_dir(),
    );

    assert_eq!(standing, Standing::Installed);
}

/// The state directory is Corral's, and a file with no configuration to back
/// up must not be blocked by a backup that has nothing to copy.
#[test]
fn creating_a_file_needs_no_backup() {
    let scratch = Scratch::new("no-backup");
    let target = scratch.target(KnownProvider::Codex);

    let standing = install(
        &target,
        &relay(KnownProvider::Codex),
        now(),
        &scratch.state_dir(),
    );

    assert_eq!(standing, Standing::Installed);
    assert!(!file::backup_dir(&scratch.state_dir()).exists());
}

#[test]
fn a_refusal_says_what_was_found_and_that_nothing_changed() {
    let refusal = Trigger::NotifierOccupied.to_string();

    assert!(refusal.contains("did not replace it"));
    assert!(!refusal.contains("binding"));
    assert!(!refusal.contains("assurance"));
}

/// `Path` is what the engine hands the adapters; this keeps the import honest
/// when the module is read on its own.
#[test]
fn a_target_names_the_file_it_acts_on() {
    let scratch = Scratch::new("target");
    let target = scratch.target(KnownProvider::Codex);

    assert_eq!(target.config_target(), ConfigTarget::CodexUserConfig);
    assert!(Path::new(target.path()).ends_with("config.toml"));
}

/// Measured: codex-cli 0.152.0 refuses to start on a `notify` of the wrong
/// type, naming a line and column. It is the user's file to fix, and never
/// something Corral quietly normalizes into an array.
#[test]
fn an_ill_typed_codex_notifier_refuses_the_write_and_is_left_as_it_is() {
    let scratch = Scratch::new("trigger-ill-typed");
    let target = scratch.target(KnownProvider::Codex);
    scratch.seed(&target, CODEX_NOTIFY_ILL_TYPED);

    let standing = install(
        &target,
        &relay(KnownProvider::Codex),
        now(),
        &scratch.state_dir(),
    );

    assert!(matches!(
        standing,
        Standing::Refused(Trigger::IncompatibleStructure { .. })
    ));
    assert_eq!(scratch.read(&target), CODEX_NOTIFY_ILL_TYPED);
}

/// The merge gate's own claim, asserted rather than asserted about: install
/// then uninstall over every file in the corpus returns each one to what the
/// user had, or refuses it and changes nothing at all. There is no third
/// outcome — normalizing a file until Corral can edit it is the thing this
/// forbids (grill Q7′).
#[test]
fn the_corpus_survives_an_install_and_uninstall_round_trip() {
    let corpus = [
        (
            KnownProvider::Claude,
            "claude-third-party",
            A_USERS_CLAUDE_SETTINGS,
        ),
        (
            KnownProvider::Claude,
            "claude-disabled",
            CLAUDE_HOOKS_DISABLED,
        ),
        (KnownProvider::Claude, "claude-not-json", CLAUDE_NOT_JSON),
        (KnownProvider::Codex, "codex-user", A_USERS_CODEX_CONFIG),
        (
            KnownProvider::Codex,
            "codex-occupied",
            CODEX_NOTIFIER_OCCUPIED,
        ),
        (
            KnownProvider::Codex,
            "codex-ill-typed",
            CODEX_NOTIFY_ILL_TYPED,
        ),
    ];

    for (provider, name, original) in corpus {
        let scratch = Scratch::new(name);
        let target = scratch.target(provider);
        scratch.seed(&target, original);

        let installed = install(&target, &relay(provider), now(), &scratch.state_dir());
        if matches!(installed, Standing::Refused(_)) {
            assert_eq!(
                scratch.read(&target),
                original,
                "{name}: a refused install changed the file",
            );
            continue;
        }
        assert_eq!(installed, Standing::Installed, "{name}");

        uninstall(&target, &relay(provider), now(), &scratch.state_dir());

        let after = scratch.read(&target);
        match provider {
            // Claude's own writes reserialize the document, so the round trip
            // is compared as values: matching the provider's normalization is
            // what preserving this file means (grill Q3′).
            KnownProvider::Claude => assert_eq!(
                serde_json::from_str::<serde_json::Value>(&after).expect("valid JSON"),
                serde_json::from_str::<serde_json::Value>(original).expect("valid JSON"),
                "{name}",
            ),
            // Codex's file is compared byte for byte: the user's comments,
            // key order and spacing are theirs and the provider preserves
            // them, so Corral must too.
            KnownProvider::Codex => assert_eq!(after, original, "{name}"),
        }
    }
}
