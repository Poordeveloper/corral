#![forbid(unsafe_code)]

//! The `corral` command line: the first surface of the session-first control
//! center, and for now the one that proves the client → daemon path works.
//!
//! Every failure is reported as facts plus a direction. The command never
//! decides on the user's behalf to upgrade, downgrade, or stop a daemon.

use std::process::ExitCode;

use clap::{Parser, Subcommand};
use corral_client::{ActivationError, ClientActivationPolicy, Connection, RequestError, activate};
use corral_protocol::method::{SessionListItem, SessionNewParams};
use corral_tui::LocalKeys;

#[derive(Debug, Parser)]
#[command(
    name = "corral",
    version,
    about = "See every session. Know what needs you. Take control."
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Check that this account's corrald is reachable and compatible.
    Ping,
    /// List the sessions corrald knows about.
    List,
    /// Start a session and attach to it.
    New {
        /// The command to run. Everything after `--` is the command's own.
        #[arg(last = true, required = true)]
        argv: Vec<String>,
    },
    /// Attach to a session corrald is already running.
    Attach {
        /// The session's id, or enough of its start to be unambiguous.
        session: String,
    },
    /// Open the session list.
    Tui,
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    let policy = ClientActivationPolicy::resolve();

    let mut connection = match activate(&policy).await {
        Ok(connection) => connection,
        Err(error) => return report_activation_failure(&error),
    };

    match cli.command {
        Command::Ping => ping(&mut connection).await,
        Command::List => list(&mut connection).await,
        Command::New { argv } => new_session(&mut connection, argv).await,
        Command::Attach { session } => attach(&mut connection, &session).await,
        Command::Tui => session_list(&policy, connection).await,
    }
}

/// Hand this terminal to the session list.
///
/// The list needs the activation policy as well as the connection: a daemon
/// that goes away while a person is watching the list is something it asks for
/// again, on exactly the terms every other surface activates under (ADR 0001).
async fn session_list(policy: &ClientActivationPolicy, connection: Connection) -> ExitCode {
    match corral_tui::run(policy, connection).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("corral: {error}");
            ExitCode::FAILURE
        }
    }
}

/// Start a session and attach to it.
async fn new_session(connection: &mut Connection, argv: Vec<String>) -> ExitCode {
    let stdin = std::io::stdin();
    let geometry = corral_tui::Geometry::of(&stdin);
    let cwd = std::env::current_dir()
        .ok()
        .map(|path| path.to_string_lossy().into_owned());

    // Minted per invocation, and the same id is what a retry would carry: it
    // is what stops a lost response from starting a second agent. This surface
    // does not retry yet, so nothing here re-sends it — the id is the daemon's
    // protection against a client that does (ADR 0002, Q13).
    let command_id = uuid::Uuid::new_v4().as_hyphenated().to_string();

    let started = match connection
        .session_new(SessionNewParams {
            command_id,
            argv,
            cwd,
            rows: geometry.map(|geometry| geometry.rows),
            cols: geometry.map(|geometry| geometry.cols),
        })
        .await
    {
        Ok(started) => started,
        Err(error) => return report_request_failure(&error),
    };

    // Standard error, because standard output belongs to the session from the
    // moment the terminal opens.
    eprintln!("session {}", started.session_id);
    attach(connection, &started.session_id).await
}

/// Attach to a running session until the person detaches.
async fn attach(connection: &mut Connection, session: &str) -> ExitCode {
    let resolved = match resolve_session(connection, session).await {
        Ok(resolved) => resolved,
        Err(code) => return code,
    };

    let Some(mut keys) = LocalKeys::start() else {
        eprintln!("corral: something is already reading this terminal");
        return ExitCode::FAILURE;
    };

    match corral_tui::open(connection, &resolved, &mut keys).await {
        Ok(()) => {
            eprintln!("detached from {resolved}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("corral: {error}");
            ExitCode::FAILURE
        }
    }
}

/// Turn what a person typed into the session id the daemon knows.
///
/// A prefix is a convenience, never an identity: an ambiguous one is refused
/// rather than resolved to whichever session sorted first.
async fn resolve_session(connection: &mut Connection, typed: &str) -> Result<String, ExitCode> {
    let listed = match connection.session_list().await {
        Ok(listed) => listed,
        Err(error) => return Err(report_request_failure(&error)),
    };

    let ids: Vec<String> = listed
        .sessions
        .iter()
        .filter_map(|session| {
            session
                .get("session_id")
                .and_then(|id| id.as_str())
                .map(str::to_owned)
        })
        .collect();

    let matching: Vec<&String> = ids.iter().filter(|id| id.starts_with(typed)).collect();
    match matching.as_slice() {
        [only] => Ok((*only).clone()),
        [] => {
            eprintln!("corral: no session starts with {typed}");
            Err(ExitCode::FAILURE)
        }
        several => {
            eprintln!("corral: {} sessions start with {typed}:", several.len());
            for id in several {
                eprintln!("  {id}");
            }
            Err(ExitCode::FAILURE)
        }
    }
}

async fn ping(connection: &mut Connection) -> ExitCode {
    let started = std::time::Instant::now();
    if let Err(error) = connection.ping().await {
        return report_request_failure(&error);
    }
    let elapsed = started.elapsed();

    let ours = connection.local_versions();
    let peer = connection.peer();
    println!("corrald at {}", connection.endpoint().display());
    println!(
        "  protocol   this build {} (needs at least {}) · daemon {} (needs at least {})",
        ours.protocol_version,
        ours.min_compatible_peer_version,
        peer.protocol_version,
        peer.min_compatible_peer_version,
    );
    println!("  negotiated {}", render_capabilities(peer));
    println!("  round trip {:.2} ms", elapsed.as_secs_f64() * 1_000.0);
    ExitCode::SUCCESS
}

async fn list(connection: &mut Connection) -> ExitCode {
    let sessions = match connection.session_list().await {
        Ok(sessions) => sessions,
        Err(error) => return report_request_failure(&error),
    };

    if sessions.sessions.is_empty() {
        // An empty list is a fact the daemon reported, not a missing answer.
        println!("No sessions.");
        return ExitCode::SUCCESS;
    }

    let mut unrenderable = 0;
    for session in &sessions.sessions {
        match serde_json::from_value::<SessionListItem>(session.clone()) {
            Ok(item) => {
                for row in session_rows(&item) {
                    println!("{row}");
                }
            }
            // A daemon newer than this build may describe a session in a shape
            // this build cannot read. Counting those is better than dropping
            // them silently or guessing at their fields.
            Err(_) => unrenderable += 1,
        }
    }
    if unrenderable > 0 {
        println!("{unrenderable} session(s) this build cannot render yet.");
    }
    ExitCode::SUCCESS
}

/// How wide the id column is: the first group of a session id, and the space
/// that separates it from what follows.
const ID_COLUMN: usize = 10;

/// How wide the state column is before the title starts.
///
/// Wider than the longest state text this surface produces, so the titles line
/// up instead of stepping in and out with the state beside them.
const STATE_COLUMN: usize = 36;

/// What `corral list` prints for one session.
///
/// One line, because a list read at a glance should stay one line per session,
/// plus the capability line when there is one. Every word of it comes from the
/// shared projection: this surface and the session list say the same thing
/// about the same session or one of them is lying (grill Q2).
fn session_rows(item: &SessionListItem) -> Vec<String> {
    let presented = corral_tui::present(item);
    let mut rows = vec![format!(
        "{:<ID_COLUMN$}{:<STATE_COLUMN$}{}",
        corral_tui::short_id(&item.session_id),
        presented.state_line(),
        item.title
    )];
    // Indented under the state it qualifies rather than beside the id.
    rows.extend(
        presented
            .screen
            .map(|screen| format!("{:ID_COLUMN$}{screen}", "")),
    );
    rows
}

fn render_capabilities(peer: &corral_protocol::ServerHello) -> String {
    if peer.capabilities.is_empty() {
        "no optional capabilities".to_owned()
    } else {
        peer.capabilities
            .iter()
            .cloned()
            .collect::<Vec<_>>()
            .join(", ")
    }
}

#[cfg(test)]
#[path = "list_tests.rs"]
mod tests;

/// Exit statuses are not a stable contract yet: this surface reports the
/// failure in words, and a taxonomy arrives with the M1 release.
fn report_activation_failure(error: &ActivationError) -> ExitCode {
    eprintln!("corral: {error}");
    ExitCode::FAILURE
}

fn report_request_failure(error: &RequestError) -> ExitCode {
    eprintln!("corral: {error}");
    ExitCode::FAILURE
}
