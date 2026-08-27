#![forbid(unsafe_code)]

//! The `corral` command line: the first surface of the session-first control
//! center, and for now the one that proves the client → daemon path works.
//!
//! Every failure is reported as facts plus a direction. The command never
//! decides on the user's behalf to upgrade, downgrade, or stop a daemon.

use std::process::ExitCode;

use clap::{Parser, Subcommand};

mod relay;
use corral_client::{ActivationError, ClientActivationPolicy, Connection, RequestError, activate};
use corral_protocol::method::SessionListItem;
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
    ///
    /// Provider-first: `corral new claude` starts a managed Claude session,
    /// and `corral new -- bash` runs a raw command. An unknown first word is
    /// refused by name rather than guessed at, so the two namespaces stay
    /// distinct (grill Q6).
    New {
        /// The agent to start, or nothing when a command follows `--`.
        provider: Option<String>,
        /// Arguments after `--`: the agent's own, or the command to run.
        #[arg(last = true)]
        rest: Vec<String>,
    },
    /// Continue a session as a new run, and attach to it.
    ///
    /// "Continue", not "resume": the product verb a person reads is Continue
    /// in Corral (`PRODUCT.md` §5).
    Continue {
        /// The session's id, or enough of its start to be unambiguous.
        session: String,
    },
    /// Attach to a session corrald is already running.
    Attach {
        /// The session's id, or enough of its start to be unambiguous.
        session: String,
    },
    /// Open the session list.
    Tui,
    /// Deliver one provider hook event to corrald. Internal.
    ///
    /// Hidden because nobody invokes it: an injected provider configuration
    /// does, once per hook. It is a subcommand of this binary rather than a
    /// second artifact so that installation, versioning, and the path a
    /// settings file names stay one thing (ADR 0004 D1).
    #[command(hide = true)]
    HookRelay {
        #[arg(long)]
        provider: String,
        #[arg(long)]
        token: String,
    },
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    // Before the command line is even read: the relay's budget is the
    // interference one hook invocation costs the user's agent, and the parse
    // is part of the invocation (ADR 0004 D4).
    let started = std::time::Instant::now();
    let cli = Cli::parse();

    // Before activation, and that is the whole point: the relay never starts
    // `corrald` and never takes the rendezvous lock. A shim that could
    // activate the daemon would be a shim that can delay the user's agent by
    // however long a cold start takes (ADR 0004 D1).
    if let Command::HookRelay { provider, token } = &cli.command {
        return relay::deliver(token, provider, started);
    }

    let policy = ClientActivationPolicy::resolve();

    let mut connection = match activate(&policy).await {
        Ok(connection) => connection,
        Err(error) => return report_activation_failure(&error),
    };

    match cli.command {
        Command::Ping => ping(&mut connection).await,
        Command::List => list(&mut connection).await,
        Command::New { provider, rest } => new_session(&mut connection, provider, rest).await,
        Command::Continue { session } => continue_session(&mut connection, &session).await,
        Command::Attach { session } => attach(&mut connection, &session).await,
        // Answered above, before anything activated a daemon.
        Command::HookRelay { .. } => ExitCode::SUCCESS,
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
async fn new_session(
    connection: &mut Connection,
    provider: Option<String>,
    rest: Vec<String>,
) -> ExitCode {
    let requested = match provider {
        Some(name) => corral_tui::Requested::Provider {
            name: name.clone(),
            args: rest,
        },
        None if rest.is_empty() => {
            eprintln!("corral: new needs an agent or a command");
            eprintln!("  corral new claude");
            eprintln!("  corral new -- bash");
            return ExitCode::FAILURE;
        }
        None => corral_tui::Requested::Command(rest),
    };
    // Kept for the hint below: an unknown agent is refused by the daemon, and
    // the fix is a command-line form only this surface knows.
    let named = match &requested {
        corral_tui::Requested::Provider { name, .. } => Some(name.clone()),
        corral_tui::Requested::Command(_) => None,
    };

    let started = match corral_tui::start_session(connection, requested).await {
        Ok(started) => started,
        Err(error) => {
            let code = report_request_failure(&error);
            // The daemon names the agents it knows; the form for a plain
            // command is this surface's own syntax, so this surface is what
            // states it.
            if let Some(named) = named {
                eprintln!("For a plain command, use: corral new -- {named}");
            }
            return code;
        }
    };

    // Standard error, because standard output belongs to the session from the
    // moment the terminal opens.
    eprintln!("session {}", started.session_id);
    attach(connection, &started.session_id).await
}

/// Continue a session as a new run, and attach to it.
async fn continue_session(connection: &mut Connection, session: &str) -> ExitCode {
    let resolved = match resolve_session(connection, session).await {
        Ok(resolved) => resolved,
        Err(code) => return code,
    };
    let continued = match corral_tui::continue_session(connection, &resolved).await {
        Ok(continued) => continued,
        Err(error) => return report_request_failure(&error),
    };
    eprintln!("session {}", continued.session_id);
    attach(connection, &continued.session_id).await
}

/// Attach to a running session until the person detaches.
async fn attach(connection: &mut Connection, session: &str) -> ExitCode {
    let resolved = match resolve_session(connection, session).await {
        Ok(resolved) => resolved,
        Err(code) => return code,
    };

    // Raw before anything reads the terminal. `terminal_attach` and the
    // channel handshake are a wide enough window to type into, and a reader
    // parked on a terminal still in line discipline gets what the person typed
    // echoed over the session about to be painted — and not until they press
    // Enter. `Ok(None)` is a pipe on standard input, which `corral new` from a
    // script legitimately has: nothing to put in raw mode and nobody typing
    // into it.
    let raw = match corral_tui::RawMode::enter() {
        Ok(raw) => raw,
        Err(error) => {
            eprintln!("corral: {error}");
            return ExitCode::FAILURE;
        }
    };
    let Some(mut keys) = LocalKeys::start() else {
        eprintln!("corral: something is already reading this terminal");
        return ExitCode::FAILURE;
    };

    let detached = corral_tui::open(connection, &resolved, &mut keys).await;
    // Theirs again before anything else is written on it: a line ending in raw
    // mode moves down without returning to the first column, and what comes
    // after this is their shell prompt.
    drop(raw);

    match detached {
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
/// plus whatever secondary lines the projection allows. Every word of it comes from the
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
    // Indented under the state they qualify rather than beside the id, and in
    // the projection's order: what the two surfaces must agree on is the words
    // and where they sit relative to each other (grill Q2).
    rows.extend(
        presented
            .beneath()
            .into_iter()
            .map(|line| format!("{:ID_COLUMN$}{line}", "")),
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
