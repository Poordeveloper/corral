use super::*;

fn home() -> PathBuf {
    PathBuf::from("/Users/someone")
}

#[test]
fn the_macos_bundle_is_an_installation() {
    let home = home();
    let installation = Installation::recognize(
        &home.join("Applications/Corral.app/Contents/MacOS/corral"),
        &home,
    )
    .expect("the bundle shape");
    assert_eq!(installation.root(), home.join("Applications/Corral.app"));
    assert_eq!(
        installation.corral(),
        home.join("Applications/Corral.app/Contents/MacOS/corral")
    );
    assert_eq!(installation.symlink(), home.join(".local/bin/corral"));
}

#[test]
fn the_linux_tree_is_an_installation() {
    let home = home();
    let installation = Installation::recognize(&home.join(".local/share/corral/bin/corral"), &home)
        .expect("the tree shape");
    assert_eq!(installation.root(), home.join(".local/share/corral"));
    assert_eq!(installation.symlink(), home.join(".local/bin/corral"));
}

#[test]
fn a_build_directory_is_not_an_installation() {
    let home = home();
    let executable = home.join("src/corral/target/debug/corral");
    let refused = Installation::recognize(&executable, &home).expect_err("not a shape");
    let words = refused.to_string();
    assert!(words.contains(&executable.display().to_string()), "{words}");
    assert!(
        words.contains("Corral.app/Contents/MacOS/corral"),
        "{words}"
    );
    assert!(words.contains(".local/share/corral/bin/corral"), "{words}");
}

/// The daemon lives in the same directory, and running `uninstall` from it
/// is not something an installation ever does.
#[test]
fn only_the_client_is_recognized() {
    let home = home();
    assert!(
        Installation::recognize(
            &home.join("Applications/Corral.app/Contents/MacOS/corrald"),
            &home
        )
        .is_err()
    );
}

/// A shape under somebody else's home is somebody else's installation.
#[test]
fn a_shape_under_another_home_is_not_this_accounts_installation() {
    let theirs = PathBuf::from("/Users/other/Applications/Corral.app/Contents/MacOS/corral");
    assert!(Installation::recognize(&theirs, &home()).is_err());
}
