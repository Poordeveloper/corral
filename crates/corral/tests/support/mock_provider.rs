#![forbid(unsafe_code)]

//! A scripted stand-in for a coding agent, for tests only.
//!
//! No test calls a real provider. What the end-to-end suite needs to prove is
//! Corral's half of the integration — that a launch carries hook injection,
//! that events reach the daemon over the real relay and the real endpoint, and
//! that identity is bound, confirmed, or contested from what arrives. A real
//! agent would prove the same thing while adding a network, a model, and an
//! account to every run.
//!
//! It behaves the way the payload fixtures say each provider behaves, and it
//! finds its own injection rather than being told where it is — that coupling
//! is the thing under test. As Claude Code it reads the injected `--settings`
//! file and runs the hook command inside it with the payload on standard
//! input; as Codex it reads the `-c notify=[…]` override off its own command
//! line and runs that program with the payload appended as one final argument
//! and nothing on standard input (ADR 0009 D2). It never knows what a launch
//! token is.
//!
//! Driven entirely by the environment, so one binary serves every scenario:
//!
//! ```text
//! CORRAL_MOCK_PROVIDER_EVENTS   a file of payloads, one JSON object per line
//! CORRAL_MOCK_PROVIDER_ARGV     a file the received argv is appended to
//! CORRAL_MOCK_PROVIDER_HOLD     "1" to stay alive until its terminal closes
//! ```

use std::io::{BufRead, Read, Write};
use std::process::{Command, Stdio};

fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    if let Ok(path) = std::env::var("CORRAL_MOCK_PROVIDER_ARGV") {
        record(&path, &argv.join(" "));
    }

    match injection(&argv) {
        Some(Injection::Settings(command)) => {
            for payload in scripted_events() {
                fire_through_a_shell(&command, &payload);
            }
        }
        Some(Injection::Notify(program)) => {
            for payload in scripted_events() {
                fire_with_the_payload_appended(&program, &payload);
            }
        }
        None => {}
    }

    // A run that holds keeps its Session in the daemon's list as `running`;
    // one that does not exits cleanly, which is what makes its end
    // established and its continuation eligible.
    if std::env::var("CORRAL_MOCK_PROVIDER_HOLD").as_deref() == Ok("1") {
        let mut sink = Vec::new();
        let _ = std::io::stdin().lock().read_to_end(&mut sink);
    }
}

/// How this launch was told to report, in the shape its provider uses.
enum Injection {
    /// A hook command line inside the settings file `--settings` names, run
    /// through a shell with the payload on standard input.
    Settings(String),
    /// A notify program named by the `-c notify=[…]` override, run with the
    /// payload appended as its final argument.
    Notify(Vec<String>),
}

/// Find this launch's injection on its own command line.
///
/// Read out of the argv (and, for Claude, out of the file the argv names)
/// rather than passed in, because that is the coupling under test: if Corral
/// stops writing something a provider can run, this finds nothing and the
/// events never arrive.
fn injection(argv: &[String]) -> Option<Injection> {
    settings_command(argv)
        .map(Injection::Settings)
        .or_else(|| notify_program(argv).map(Injection::Notify))
}

fn settings_command(argv: &[String]) -> Option<String> {
    // The *last* one, because that is what Claude Code does with a repeated
    // flag (matrix scenario 8) and it is the reason a caller's own `--settings`
    // is refused outright. A stand-in that took the first would keep passing a
    // launch where the real provider had loaded somebody else's file.
    let settings = argv
        .windows(2)
        .rfind(|pair| pair[0] == "--settings")
        .map(|pair| pair[1].clone())?;
    let document: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(settings).ok()?).ok()?;
    document
        .get("hooks")?
        .as_object()?
        .values()
        .next()?
        .get(0)?
        .get("hooks")?
        .get(0)?
        .get("command")?
        .as_str()
        .map(str::to_owned)
}

/// The notify program the `-c notify=[…]` override names.
///
/// The last override wins, the way the real CLI resolves a repeated flag
/// (spike scenario 5). The array is decoded as JSON: the escape vocabulary
/// Corral emits is the part TOML basic strings and JSON strings spell
/// identically, so this reads the value with a parser rather than with a
/// hand-written unescape. That the real TOML parser accepts it is the version
/// matrix's to prove, not a stand-in's.
fn notify_program(argv: &[String]) -> Option<Vec<String>> {
    let assignment = argv
        .windows(2)
        .rfind(|pair| pair[0] == "-c" && pair[1].starts_with("notify="))
        .map(|pair| pair[1].clone())?;
    let array = assignment.strip_prefix("notify=")?;
    let program: Vec<String> = serde_json::from_str(array).ok()?;
    (!program.is_empty()).then_some(program)
}

fn scripted_events() -> Vec<String> {
    let Ok(path) = std::env::var("CORRAL_MOCK_PROVIDER_EVENTS") else {
        return Vec::new();
    };
    let Ok(file) = std::fs::File::open(path) else {
        return Vec::new();
    };
    std::io::BufReader::new(file)
        .lines()
        .map_while(Result::ok)
        .filter(|line| !line.trim().is_empty())
        .collect()
}

/// Run one notify program the way Codex does: the configured words, then the
/// notification as exactly one more argument, and nothing on standard input.
fn fire_with_the_payload_appended(program: &[String], payload: &str) {
    let Some((command, configured)) = program.split_first() else {
        return;
    };
    let _ = Command::new(command)
        .args(configured)
        .arg(payload)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

/// Run one hook, the way Claude Code does: through a shell, with the payload on
/// standard input, and with whatever it says on stdout ignored here.
fn fire_through_a_shell(relay: &str, payload: &str) {
    let Ok(mut child) = Command::new("sh")
        .arg("-c")
        .arg(relay)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    else {
        return;
    };
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(payload.as_bytes());
    }
    let _ = child.wait();
}

fn record(path: &str, line: &str) {
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = writeln!(file, "{line}");
    }
}
