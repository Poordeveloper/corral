use super::*;

fn args(values: &[&str]) -> Vec<OsString> {
    values.iter().map(OsString::from).collect()
}

/// The backend substitutes `$HOME` for a working directory that is not a
/// directory, so a request that would silently run somewhere else must be
/// refused before it reaches the backend at all (grill Q1).
#[test]
fn spawn_invalid_cwd_is_rejected_before_pty_spawn() {
    let missing = std::env::temp_dir().join("corral-no-such-directory-ever");
    let _ = std::fs::remove_dir_all(&missing);

    let rejection = LaunchRequest::new("/bin/sh", args(&[]), &missing)
        .expect_err("a missing working directory is refused");

    assert_eq!(rejection, LaunchRejection::WorkingDirectoryMissing(missing));
}

/// A path that exists but is a file is a different mistake from one that does
/// not exist, and the backend would paper over both identically.
#[test]
fn a_file_as_working_directory_is_rejected_as_its_own_mistake() {
    let file = std::env::temp_dir().join(format!("corral-cwd-file-{}", std::process::id()));
    std::fs::write(&file, b"not a directory").expect("write the scratch file");

    let rejection = LaunchRequest::new("/bin/sh", args(&[]), &file)
        .expect_err("a file is refused as a working directory");

    let _ = std::fs::remove_file(&file);
    assert_eq!(
        rejection,
        LaunchRejection::WorkingDirectoryNotADirectory(file)
    );
}

#[test]
fn an_empty_program_is_refused() {
    let rejection = LaunchRequest::new("", args(&[]), std::env::temp_dir())
        .expect_err("an empty program is refused");

    assert_eq!(rejection, LaunchRejection::EmptyProgram);
}

/// argv carries tokens, passwords, URLs, and customer identifiers; the label a
/// list shows is the basename alone (grill Q3).
#[test]
fn the_display_title_is_the_basename_and_never_the_arguments() {
    let request = LaunchRequest::new(
        "/usr/local/bin/claude",
        args(&["--token", "sk-secret-value"]),
        std::env::temp_dir(),
    )
    .expect("a valid request");

    assert_eq!(request.display_title(), "claude");
}

#[test]
fn a_valid_request_keeps_the_directory_it_was_given() {
    let directory = std::env::temp_dir();

    let request =
        LaunchRequest::new("/bin/sh", args(&["-c", "true"]), &directory).expect("a valid request");

    assert_eq!(request.working_directory(), directory.as_path());
    assert_eq!(request.args(), args(&["-c", "true"]).as_slice());
}
