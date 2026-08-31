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
//! It behaves the way the payload fixtures say Claude Code behaves: it reads
//! the injected `--settings` file, finds the hook command Corral wrote into
//! it, and runs that command once per scripted event with the event's payload
//! on standard input. It never knows what a launch token is.
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

    if let Some(relay) = relay_command(&argv) {
        for payload in scripted_events() {
            fire(&relay, &payload);
        }
    }

    // A run that holds keeps its Session in the daemon's list as `running`;
    // one that does not exits cleanly, which is what makes its end
    // established and its continuation eligible.
    if std::env::var("CORRAL_MOCK_PROVIDER_HOLD").as_deref() == Ok("1") {
        let mut sink = Vec::new();
        let _ = std::io::stdin().lock().read_to_end(&mut sink);
    }
}

/// The hook command Corral wrote into the settings file this launch was given.
///
/// Read out of the file rather than passed in, because that is the coupling
/// under test: if Corral stops writing a command a provider can run, this
/// finds nothing and the events never arrive.
fn relay_command(argv: &[String]) -> Option<String> {
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

/// Run one hook, the way a provider does: through a shell, with the payload on
/// standard input, and with whatever it says on stdout ignored here.
fn fire(relay: &str, payload: &str) {
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
