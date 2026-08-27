use std::os::unix::fs::PermissionsExt;

use super::*;

/// A private scratch directory that cleans up after itself.
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("corral-launch-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("a scratch directory");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn scope() -> LaunchScope {
    LaunchScope {
        session: CorralSessionId::mint(),
        run: RunId::mint(),
        provider: KnownProvider::Claude,
    }
}

#[test]
fn a_minted_token_resolves_to_the_launch_it_names() {
    let mut tokens = LaunchTokens::new();
    let first = scope();
    let second = scope();
    let one = tokens.mint(first).expect("a token");
    let two = tokens.mint(second).expect("a token");

    assert_eq!(tokens.resolve(&one), Some(first));
    assert_eq!(tokens.resolve(&two), Some(second));
    assert_ne!(one, two);
}

/// Resolution is a read. A launch fires hooks for as long as it runs, and
/// every one of them has to reach the same Session.
#[test]
fn resolving_a_token_does_not_consume_it() {
    let mut tokens = LaunchTokens::new();
    let scope = scope();
    let token = tokens.mint(scope).expect("a token");
    for _ in 0..3 {
        assert_eq!(tokens.resolve(&token), Some(scope));
    }
}

/// An event with no token, an unknown token, or another daemon's token
/// resolves to nothing. It is never correlated by cwd or time.
#[test]
fn a_token_this_daemon_did_not_mint_resolves_to_nothing() {
    let mut tokens = LaunchTokens::new();
    tokens.mint(scope()).expect("a token");
    let stranger = LaunchTokens::new().mint(scope()).expect("a token");
    assert_eq!(tokens.resolve(&stranger), None);
}

#[test]
fn a_forgotten_launch_resolves_to_nothing() {
    let mut tokens = LaunchTokens::new();
    let token = tokens.mint(scope()).expect("a token");
    tokens.forget(token);
    assert_eq!(tokens.resolve(&token), None);
    assert_eq!(tokens.outstanding(), 0);
}

#[test]
fn a_token_survives_its_own_wire_form() {
    let mut tokens = LaunchTokens::new();
    let token = tokens.mint(scope()).expect("a token");
    let wire = token.to_wire();
    assert_eq!(wire.len(), 32);
    assert_eq!(LaunchToken::from_wire(&wire), Some(token));
    for malformed in ["", "abc", &"z".repeat(32), &wire[..31]] {
        assert_eq!(LaunchToken::from_wire(malformed), None, "{malformed}");
    }
}

/// A capability in a log outlives the launch it names.
#[test]
fn a_token_never_prints_itself() {
    let token = LaunchTokens::new().mint(scope()).expect("a token");
    assert_eq!(format!("{token:?}"), "LaunchToken(redacted)");
    assert!(!format!("{token:?}").contains(&token.to_wire()));
}

#[test]
fn an_injected_file_is_private_from_creation_and_holds_the_relay_command() {
    let scratch = Scratch::new("write");
    let run = RunId::mint();
    let settings = InjectedSettings::write(
        scratch.path(),
        run,
        KnownProvider::Claude,
        "'/opt/corral' relay",
    )
    .expect("the settings are written");

    let mode = std::fs::metadata(settings.path())
        .expect("the file exists")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o600);
    let document = std::fs::read_to_string(settings.path()).expect("readable");
    assert!(document.contains("'/opt/corral' relay"), "{document}");
    // Published whole: no partial is left where a provider could read one.
    assert!(
        !scratch
            .path()
            .join(format!("corral-launch-{run}.json.partial"))
            .exists()
    );
}

/// Provenance in the name, so a sweep can never confuse one of Corral's files
/// with anything else in the directory.
#[test]
fn an_injected_files_name_says_which_run_owns_it() {
    let run = RunId::mint();
    let path = InjectedSettings::path_for(Path::new("/state/launch"), run);
    let name = path
        .file_name()
        .expect("a name")
        .to_string_lossy()
        .into_owned();
    assert_eq!(name, format!("corral-launch-{run}.json"));
}

#[test]
fn a_finished_runs_file_is_removed_and_removing_a_missing_one_is_quiet() {
    let scratch = Scratch::new("remove");
    let run = RunId::mint();
    InjectedSettings::write(scratch.path(), run, KnownProvider::Claude, "relay").expect("written");
    InjectedSettings::remove_for(scratch.path(), run);
    assert!(!InjectedSettings::path_for(scratch.path(), run).exists());
    // Idempotent: a Run whose ending is observed twice must not be a warning
    // storm, and a file already gone is the outcome the call wanted.
    InjectedSettings::remove_for(scratch.path(), run);
}

/// The three classes the founder ruled removable, and everything else
/// retained. An Unverifiable owner is the one this exists to protect: losing
/// Corral's ownership is not proof the provider process is dead (grill Q10).
#[test]
fn only_the_three_evidenced_classes_are_swept() {
    let exited = RunId::mint();
    let unverified = RunId::mint();
    let uncommitted = RunId::mint();
    let partial = RunId::mint();
    let known = move |run: RunId| {
        if run == exited {
            Some(true)
        } else if run == unverified {
            Some(false)
        } else {
            None
        }
    };

    assert_eq!(
        classify(&format!("corral-launch-{exited}.json"), &known),
        SweepVerdict::OwnerExited,
    );
    assert_eq!(
        classify(&format!("corral-launch-{unverified}.json"), &known),
        SweepVerdict::OwnerUnverified,
    );
    assert_eq!(
        classify(&format!("corral-launch-{uncommitted}.json"), &known),
        SweepVerdict::NoLaunchCommitted,
    );
    assert_eq!(
        classify(&format!("corral-launch-{partial}.json.partial"), &known),
        SweepVerdict::NeverPublished,
    );
    for stranger in ["registry.sqlite3", "notes.json", ".DS_Store", "launch.json"] {
        assert_eq!(
            classify(stranger, &known),
            SweepVerdict::NotOurs,
            "{stranger}"
        );
    }
    // Corral's prefix over a name that says nothing about an owner. There is
    // no evidence to destroy it on, so it stays.
    assert_eq!(
        classify("corral-launch-not-a-run.json", &known),
        SweepVerdict::MalformedName,
    );
}

#[test]
fn a_sweep_removes_the_evidenced_files_and_leaves_the_rest() {
    let scratch = Scratch::new("sweep");
    let exited = RunId::mint();
    let unverified = RunId::mint();
    let uncommitted = RunId::mint();
    for run in [exited, unverified, uncommitted] {
        InjectedSettings::write(scratch.path(), run, KnownProvider::Claude, "relay")
            .expect("written");
    }
    let partial = scratch
        .path()
        .join("corral-launch-".to_owned() + &RunId::mint().to_string() + ".json.partial");
    std::fs::write(&partial, "half").expect("a remnant");
    let stranger = scratch.path().join("someone-elses.json");
    std::fs::write(&stranger, "{}").expect("a stranger's file");

    sweep_launch_dir(scratch.path(), move |run| {
        if run == exited {
            Some(true)
        } else if run == unverified {
            Some(false)
        } else {
            None
        }
    });

    assert!(!InjectedSettings::path_for(scratch.path(), exited).exists());
    assert!(!InjectedSettings::path_for(scratch.path(), uncommitted).exists());
    assert!(!partial.exists());
    assert!(
        InjectedSettings::path_for(scratch.path(), unverified).exists(),
        "an unverifiable owner keeps its file",
    );
    assert!(
        stranger.exists(),
        "a sweep touches nothing it did not write"
    );
}

/// A directory that is not there is a daemon that has launched nothing yet.
#[test]
fn sweeping_a_directory_that_does_not_exist_is_quiet() {
    sweep_launch_dir(Path::new("/nonexistent/corral/launch"), |_| None);
}

/// The hook command line is handed to a shell by the provider, so a path with
/// a space, a quote, or a `$` has to survive it unchanged.
#[test]
fn a_relay_path_survives_the_shell_it_is_handed_to() {
    assert_eq!(shell_word("/opt/corral"), "'/opt/corral'");
    assert_eq!(
        shell_word("/Users/a b/Application Support/corral"),
        "'/Users/a b/Application Support/corral'",
    );
    assert_eq!(
        shell_word("/opt/$(rm -rf ~)/corral"),
        "'/opt/$(rm -rf ~)/corral'"
    );
    assert_eq!(shell_word("/opt/it's/corral"), r"'/opt/it'\''s/corral'");
}

/// A token outlives its Run only as a way to be wrong. Once the Run is over,
/// evidence arriving under its token is late evidence about a dead Run — and
/// after a continuation has replaced the Session's runtime, that token would
/// let a process which outlived its episode contest the identity of the one
/// that replaced it (ADR 0004 D5).
#[test]
fn a_token_is_retired_with_the_run_it_names() {
    let mut tokens = LaunchTokens::new();
    let session = CorralSessionId::mint();
    let first = LaunchScope {
        session,
        run: RunId::mint(),
        provider: KnownProvider::Claude,
    };
    let second = LaunchScope {
        session,
        run: RunId::mint(),
        provider: KnownProvider::Claude,
    };
    let ended = tokens.mint(first).expect("a token");
    let live = tokens.mint(second).expect("a token");

    tokens.forget_run(first.run);

    assert_eq!(
        tokens.resolve(&ended),
        None,
        "the ended run's token is retired"
    );
    assert_eq!(
        tokens.resolve(&live),
        Some(second),
        "the run that replaced it keeps its own",
    );
    assert_eq!(tokens.outstanding(), 1, "the map does not grow forever");
}

/// A Run nothing minted a token for is not an error to retire.
#[test]
fn retiring_a_run_with_no_token_is_quiet() {
    let mut tokens = LaunchTokens::new();
    let scope = scope();
    tokens.mint(scope).expect("a token");

    tokens.forget_run(RunId::mint());

    assert_eq!(tokens.outstanding(), 1);
}
