#![forbid(unsafe_code)]

//! The `corral` command line: the first surface of the session-first control
//! center, and for now the one that proves the client → daemon path works.
//!
//! Every failure is reported as facts plus a direction. The command never
//! decides on the user's behalf to upgrade, downgrade, or stop a daemon.

use std::process::ExitCode;

use clap::{Parser, Subcommand};
use corral_client::{ActivationError, ClientActivationPolicy, Connection, RequestError, activate};

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
    } else {
        // A daemon newer than this build may know about sessions whose shape
        // this build cannot render; saying so is better than pretending.
        println!(
            "{} session(s) reported by a daemon this build cannot render yet.",
            sessions.sessions.len()
        );
    }
    ExitCode::SUCCESS
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
