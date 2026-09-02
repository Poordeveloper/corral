use super::*;

use std::os::unix::fs::PermissionsExt as _;
use std::path::Path;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

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

/// An enable and a disable within one second are two mutations, and each
/// keeps the copy it took: the second must not land on the first's name and
/// lose the bytes the user had before either.
#[test]
fn two_mutations_in_one_second_keep_both_backups() {
    let scratch = Scratch::new("backup-same-second");
    let target = scratch.target(KnownProvider::Claude);
    scratch.seed(&target, A_USERS_CLAUDE_SETTINGS);

    install(
        &target,
        &relay(KnownProvider::Claude),
        now(),
        &scratch.state_dir(),
    );
    let installed = scratch.read(&target);
    uninstall(
        &target,
        &relay(KnownProvider::Claude),
        now(),
        &scratch.state_dir(),
    );

    let mut backups = std::fs::read_dir(file::backup_dir(&scratch.state_dir()))
        .expect("the backup directory")
        .map(|entry| std::fs::read_to_string(entry.expect("an entry").path()).expect("read"))
        .collect::<Vec<_>>();
    assert_eq!(backups.len(), 2, "one backup per mutation");
    backups.sort();
    let mut expected = vec![A_USERS_CLAUDE_SETTINGS.to_owned(), installed];
    expected.sort();
    assert_eq!(backups, expected);
}

/// Retention is bounded (ADR 0013 D3): past the bound the oldest copies of a
/// file go, the newest stay, and nothing that is not a copy of that file is
/// touched — not another provider's backups, not a file somebody left there.
#[test]
fn backups_of_one_file_are_bounded_and_the_newest_are_the_ones_kept() {
    let scratch = Scratch::new("backup-retention");
    let target = scratch.target(KnownProvider::Claude);
    scratch.seed(&target, A_USERS_CLAUDE_SETTINGS);
    let directory = file::backup_dir(&scratch.state_dir());
    std::fs::create_dir_all(&directory).expect("the backup directory");
    let start = now();
    let stamp = |seconds: u64| {
        start
            .duration_since(SystemTime::UNIX_EPOCH)
            .expect("after the epoch")
            .as_secs()
            + seconds
    };
    let somebody_elses = directory.join(format!("claude-{}-settings.json.orig", stamp(0)));
    let the_other_providers = directory.join(format!("codex-{}-config.toml", stamp(0)));
    std::fs::write(&somebody_elses, "theirs").expect("their file");
    std::fs::write(&the_other_providers, "codex").expect("the other provider's backup");

    let mut before_last = String::new();
    for mutation in 0..=file::BACKUPS_RETAINED as u64 {
        before_last = scratch.read(&target);
        let at = start + Duration::from_secs(mutation);
        if mutation % 2 == 0 {
            install(
                &target,
                &relay(KnownProvider::Claude),
                at,
                &scratch.state_dir(),
            );
        } else {
            uninstall(
                &target,
                &relay(KnownProvider::Claude),
                at,
                &scratch.state_dir(),
            );
        }
    }

    let mut names = std::fs::read_dir(&directory)
        .expect("the backup directory")
        .map(|entry| {
            entry
                .expect("an entry")
                .file_name()
                .into_string()
                .expect("utf-8")
        })
        .collect::<Vec<_>>();
    names.sort();
    let mut expected = (1..=file::BACKUPS_RETAINED as u64)
        .map(|seconds| format!("claude-{}-settings.json", stamp(seconds)))
        .collect::<Vec<_>>();
    expected.push(format!("claude-{}-settings.json.orig", stamp(0)));
    expected.push(format!("codex-{}-config.toml", stamp(0)));
    expected.sort();
    assert_eq!(names, expected, "the oldest copy went and nothing else did");
    let newest = directory.join(format!(
        "claude-{}-settings.json",
        stamp(file::BACKUPS_RETAINED as u64)
    ));
    assert_eq!(std::fs::read_to_string(newest).expect("read"), before_last);
    assert_eq!(
        std::fs::read_to_string(somebody_elses).expect("read"),
        "theirs"
    );
    assert_eq!(
        std::fs::read_to_string(the_other_providers).expect("read"),
        "codex"
    );
}

/// Within one second the names count up and never fill a gap retention
/// opened: a freed name taken again would sort as the oldest copy and be the
/// next to go — the copy just taken, which is the one the mutation owes.
#[test]
fn copies_taken_in_one_second_past_the_bound_keep_the_newest() {
    let scratch = Scratch::new("backup-retention-same-second");
    let target = scratch.target(KnownProvider::Claude);
    scratch.seed(&target, A_USERS_CLAUDE_SETTINGS);
    let mutations = file::BACKUPS_RETAINED + 3;

    let mut before_last = String::new();
    for mutation in 0..mutations {
        before_last = scratch.read(&target);
        if mutation % 2 == 0 {
            install(
                &target,
                &relay(KnownProvider::Claude),
                now(),
                &scratch.state_dir(),
            );
        } else {
            uninstall(
                &target,
                &relay(KnownProvider::Claude),
                now(),
                &scratch.state_dir(),
            );
        }
    }

    let directory = file::backup_dir(&scratch.state_dir());
    let stamp = now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .expect("after the epoch")
        .as_secs();
    let mut names = std::fs::read_dir(&directory)
        .expect("the backup directory")
        .map(|entry| {
            entry
                .expect("an entry")
                .file_name()
                .into_string()
                .expect("utf-8")
        })
        .collect::<Vec<_>>();
    names.sort();
    let mut expected = (mutations - file::BACKUPS_RETAINED..mutations)
        .map(|attempt| format!("claude-{stamp}-settings.json.{attempt}"))
        .collect::<Vec<_>>();
    expected.sort();
    assert_eq!(names, expected);
    let newest = directory.join(format!("claude-{stamp}-settings.json.{}", mutations - 1));
    assert_eq!(std::fs::read_to_string(newest).expect("read"), before_last);
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

    assert_eq!(outcome, Err(Trigger::ChangedUnderCorral));
    assert_eq!(
        scratch.read(&target),
        "{\"model\": \"written by the provider\"}"
    );
}

/// The same race when the provider publishes by rename: a new inode at the
/// same path, which is the replacement an atomic writer makes. The identity
/// Corral holds is the one it read from, so the file that appeared under it
/// is a different file and the write stops.
#[test]
fn a_file_renamed_into_place_while_corral_writes_aborts_rather_than_clobbering() {
    let scratch = Scratch::new("trigger-race-rename");
    let target = scratch.target(KnownProvider::Claude);
    scratch.seed(&target, A_USERS_CLAUDE_SETTINGS);
    let original = file::read(&target).expect("read").expect("a file");
    let providers_own = target.path().with_extension("json.provider-tmp");
    std::fs::write(&providers_own, "{\"model\": \"written by the provider\"}")
        .expect("the provider's temporary file");
    std::fs::rename(&providers_own, target.path()).expect("the provider's rename");

    let outcome = file::replace(
        &target,
        &original,
        now(),
        &scratch.state_dir(),
        |document| document.install(&relay(KnownProvider::Claude)),
    );

    assert_eq!(outcome, Err(Trigger::ChangedUnderCorral));
    assert_eq!(
        scratch.read(&target),
        "{\"model\": \"written by the provider\"}"
    );
}

/// A publish writes only into a file it created. A temporary file another
/// writer left beside the configuration — a publish that crashed, or one in
/// flight — is neither written into nor renamed into place, so what lands is
/// exactly the candidate this publish validated, and the other file is left
/// for its owner.
#[test]
fn a_publish_writes_only_into_a_temporary_file_of_its_own() {
    let scratch = Scratch::new("own-partial");
    let target = scratch.target(KnownProvider::Claude);
    scratch.seed(&target, A_USERS_CLAUDE_SETTINGS);
    let directory = target.path().parent().expect("a directory");
    let another_writers = [
        directory.join(".settings.json.corral-partial"),
        directory.join(".settings.json.corral-partial-0000000000000000"),
    ];
    for partial in &another_writers {
        std::fs::write(partial, "not a candidate").expect("another writer's file");
    }

    let standing = install(
        &target,
        &relay(KnownProvider::Claude),
        now(),
        &scratch.state_dir(),
    );

    assert_eq!(standing, Standing::Installed);
    for partial in &another_writers {
        assert_eq!(
            std::fs::read_to_string(partial).expect("the other writer's file is still there"),
            "not a candidate"
        );
    }
    assert_eq!(
        file::partials_beside(target.path()),
        vec![another_writers[1].clone()],
        "the publish left nothing of its own behind"
    );
}

/// Two operations on one provider in flight together — one connection
/// enabling while another disables — end with the recorded intent and the
/// file agreeing, and each reporting the standing it produced rather than a
/// refusal over a file only Corral touched. The write's own identity check
/// cannot provide this: both writers pass it against the same original, and
/// the second rename discards the first while both report success.
#[tokio::test]
async fn operations_in_flight_together_leave_intent_and_file_agreeing() {
    let scratch = Scratch::new("serialized");
    let state = daemon_state(&scratch);
    let target = scratch.target(KnownProvider::Claude);
    scratch.seed(&target, A_USERS_CLAUDE_SETTINGS);
    let relay = relay(KnownProvider::Claude);
    let provider = ProviderId::new("claude").expect("a provider id");

    for round in 0..6 {
        let enabling = enable(
            &state,
            target.clone(),
            relay.clone(),
            now(),
            scratch.state_dir(),
        );
        let disabling = disable(
            &state,
            target.clone(),
            relay.clone(),
            now(),
            scratch.state_dir(),
        );
        // Alternating which is polled first, so both orders are exercised.
        let (enabled, disabled) = if round % 2 == 0 {
            tokio::join!(enabling, disabling)
        } else {
            let (disabled, enabled) = tokio::join!(disabling, enabling);
            (enabled, disabled)
        };

        assert_eq!(
            enabled.expect("the store"),
            Some(Standing::Installed),
            "round {round}"
        );
        assert_eq!(
            disabled.expect("the store"),
            Some(Standing::NotInstalled),
            "round {round}"
        );
        let expected = match state
            .integration_intent(provider.clone())
            .await
            .expect("the store")
            .map(|recorded| recorded.intent())
        {
            Some(IntegrationIntent::Enabled) => Standing::Installed,
            Some(IntegrationIntent::Disabled) => Standing::NotInstalled,
            None => panic!("an operation ran without recording a decision"),
        };
        assert_eq!(status(&target, &relay), expected, "round {round}");
        assert!(
            file::partials_beside(target.path()).is_empty(),
            "round {round} left a temporary file behind"
        );
    }
}

/// A daemon's state on this scratch, for the operations that record intent.
fn daemon_state(scratch: &Scratch) -> Arc<DaemonState> {
    Arc::new(
        DaemonState::open(
            &scratch.root.join("registry.sqlite3"),
            &scratch.root.join("launch"),
            &scratch.state_dir(),
        )
        .expect("open the registry"),
    )
}

/// A dotfiles user's configuration is a link into a repository. Corral edits
/// the file the link names and leaves the link as the user made it: a
/// replacement that turned the link into a regular file would sever an
/// arrangement uninstall could never put back, while the provider went on
/// reading valid settings and nobody noticed.
#[test]
fn a_linked_configuration_is_edited_through_the_link_and_the_link_survives() {
    let scratch = Scratch::new("symlink");
    let target = scratch.target(KnownProvider::Codex);
    let repository = scratch.root.join("dotfiles").join("codex.toml");
    std::fs::create_dir_all(repository.parent().expect("a directory")).expect("the repository");
    std::fs::write(&repository, A_USERS_CODEX_CONFIG).expect("the user's file");
    std::fs::create_dir_all(target.path().parent().expect("a directory")).expect("the home");
    std::os::unix::fs::symlink(&repository, target.path()).expect("the user's link");

    let installed = install(
        &target,
        &relay(KnownProvider::Codex),
        now(),
        &scratch.state_dir(),
    );

    assert_eq!(installed, Standing::Installed);
    assert!(is_symlink(target.path()), "the link was replaced by a file");
    assert!(
        std::fs::read_to_string(&repository)
            .expect("read")
            .contains("hook-relay"),
        "the entry did not land in the file the link names",
    );

    let uninstalled = uninstall(
        &target,
        &relay(KnownProvider::Codex),
        now(),
        &scratch.state_dir(),
    );

    assert_eq!(uninstalled, Standing::NotInstalled);
    assert!(is_symlink(target.path()), "the link was replaced by a file");
    assert_eq!(
        std::fs::read_to_string(&repository).expect("read"),
        A_USERS_CODEX_CONFIG,
    );
}

/// The link is re-pointed while Corral writes — a dotfiles manager switching
/// profiles, atomically, the way such tools do. The file Corral read is then
/// no longer the user's configuration: editing it would report an
/// installation the configured path does not carry, so the write stops and
/// neither file is touched.
#[test]
fn a_link_re_pointed_while_corral_writes_aborts_rather_than_editing_the_old_file() {
    let scratch = Scratch::new("symlink-repointed");
    let target = scratch.target(KnownProvider::Codex);
    let dotfiles = scratch.root.join("dotfiles");
    std::fs::create_dir_all(&dotfiles).expect("the repository");
    let before = dotfiles.join("work.toml");
    let after = dotfiles.join("home.toml");
    std::fs::write(&before, A_USERS_CODEX_CONFIG).expect("the first profile");
    std::fs::write(&after, "# the other profile\n").expect("the second profile");
    std::fs::create_dir_all(target.path().parent().expect("a directory")).expect("the home");
    std::os::unix::fs::symlink(&before, target.path()).expect("the user's link");
    let original = file::read(&target).expect("read").expect("a file");
    let switching = target.path().with_extension("toml.switching");
    std::os::unix::fs::symlink(&after, &switching).expect("the manager's new link");
    std::fs::rename(&switching, target.path()).expect("the manager's switch");

    let outcome = file::replace(
        &target,
        &original,
        now(),
        &scratch.state_dir(),
        |document| document.install(&relay(KnownProvider::Codex)),
    );

    assert_eq!(outcome, Err(Trigger::ChangedUnderCorral));
    assert_eq!(
        std::fs::read_to_string(&before).expect("read"),
        A_USERS_CODEX_CONFIG,
        "the file the link no longer names was edited",
    );
    assert_eq!(
        std::fs::read_to_string(&after).expect("read"),
        "# the other profile\n",
        "the file the link now names was written without being read",
    );
    assert_eq!(std::fs::read_link(target.path()).expect("the link"), after);
}

/// A link to nothing is refused, not resolved. Where the user's configuration
/// should come to exist is their decision, and the link is left exactly as
/// it was so they can make it.
#[test]
fn a_link_to_nothing_refuses_the_write_and_is_left_as_it_is() {
    let scratch = Scratch::new("dangling-symlink");
    let target = scratch.target(KnownProvider::Claude);
    let nowhere = scratch.root.join("dotfiles").join("claude.json");
    std::fs::create_dir_all(target.path().parent().expect("a directory")).expect("the home");
    std::os::unix::fs::symlink(&nowhere, target.path()).expect("the user's link");

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
    assert!(is_symlink(target.path()), "the link was replaced by a file");
    assert_eq!(
        std::fs::read_link(target.path()).expect("the link"),
        nowhere
    );
    assert!(
        !nowhere.exists(),
        "corral chose where the configuration lives"
    );
}

fn is_symlink(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|data| data.file_type().is_symlink())
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
