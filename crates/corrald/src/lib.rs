#![forbid(unsafe_code)]

//! `corrald`: the one primary daemon of an OS account.
//!
//! Startup is ordered so that no two daemons can ever serve the same
//! rendezvous: claim the singleton lock first, clean only what the claim
//! proves is stale, and bind last. A daemon that loses the claim exits
//! cleanly, having touched nothing (ADR 0001 D2, D4).
//!
//! The registry store is opened and validated before the endpoint is bound, so
//! a store the daemon cannot vouch for is a startup failure rather than
//! something a client discovers after its hello succeeded (ADR 0002, Q14). The
//! lock and socket pathnames are rendezvous artifacts, not semantic state: a
//! new `corrald` reconstructs nothing from its predecessor's runtime.

mod connection;
mod lifecycle;
mod platform;
mod policy;
/// The managed runtime. Public because the lifecycle scenarios this crate owes
/// — detach, disconnect, restart, crash, unverifiable exit — are integration
/// tests, and an integration test reaches only what the library exposes.
pub mod runtime;
mod server;
mod state;
mod terminal_channel;

use std::fmt;
use std::process::ExitCode;
use std::sync::Arc;

use clap::Parser;
use corral_rendezvous::{RendezvousError, RendezvousPaths, SingletonClaim, remove_stale_socket};
use corral_state::StateError;
use tracing::{error, info};

use crate::policy::{DaemonPolicy, SINGLETON_CLAIM_WAIT};
use crate::state::DaemonState;

/// `corrald` is started by Corral surfaces, not by people. Run directly it is
/// an ordinary foreground process logging to standard error.
#[derive(Debug, Parser)]
#[command(name = "corrald", version, about = "The Corral session daemon")]
struct Arguments {
    /// Marks a daemon started by auto-activation rather than by a person.
    /// Internal, and not a stable command-line contract.
    #[arg(long = "internal-auto-start", hide = true)]
    auto_start: bool,
}

/// Every way the daemon can fail before it serves anything.
#[derive(Debug)]
enum StartupError {
    Rendezvous(RendezvousError),
    State(StateError),
    Runtime(std::io::Error),
    Serve(std::io::Error),
}

pub fn run() -> ExitCode {
    let arguments = Arguments::parse();
    init_tracing();

    if arguments.auto_start {
        platform::detach_session();
    }

    match start() {
        Ok(code) => code,
        Err(error) => {
            error!(%error, "corrald could not start");
            ExitCode::FAILURE
        }
    }
}

fn start() -> Result<ExitCode, StartupError> {
    let paths = RendezvousPaths::canonical().map_err(StartupError::Rendezvous)?;
    paths.ensure_run_dir().map_err(StartupError::Rendezvous)?;
    let policy = DaemonPolicy::resolve();

    let Some(claim) = SingletonClaim::acquire(paths.lock(), SINGLETON_CLAIM_WAIT)
        .map_err(StartupError::Rendezvous)?
    else {
        // Losing the race is the ordinary outcome when several clients cold
        // start at once, not a failure worth an error status.
        info!(
            lock = %paths.lock().display(),
            "another corrald holds the singleton claim; exiting"
        );
        return Ok(ExitCode::SUCCESS);
    };

    // Only the claim holder may clean, and only a confirmed socket artifact.
    remove_stale_socket(&claim, paths.socket()).map_err(StartupError::Rendezvous)?;

    // Before the endpoint exists, not after: a daemon that answered a hello and
    // then found its registry unusable would already have told a client it can
    // be relied on.
    paths.ensure_state_dir().map_err(StartupError::Rendezvous)?;
    let state = Arc::new(DaemonState::open(paths.registry()).map_err(StartupError::State)?);

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(StartupError::Runtime)?;
    runtime
        .block_on(server::serve(paths.socket(), policy, Arc::clone(&state)))
        .map_err(StartupError::Serve)?;

    // Order matters and is stated rather than left to drop order: the runtime
    // and every connection it owns go first, then the rendezvous pathname,
    // and the claim last. While the claim is held a probing client sees an
    // owner and refuses to start a replacement, so nothing can bind into a
    // half-dismantled rendezvous (ADR 0001 D6).
    drop(runtime);
    // Best effort: the next claim winner owns whatever an abrupt death leaves
    // behind, so failing to unlink here costs nothing.
    let _ = std::fs::remove_file(paths.socket());
    drop(claim);

    // A daemon that stopped because it could not trust its own durable state
    // says so in its exit status, whatever else committed the shutdown first
    // and whichever task reached the conclusion. The store is what remembers
    // it. The next activation retries initialization; if the cause persists it
    // still cannot become ready.
    if state.stopped_vouching() {
        return Ok(ExitCode::FAILURE);
    }
    Ok(ExitCode::SUCCESS)
}

/// Diagnostics go to standard error. An auto-started daemon has had its
/// standard error pointed at the daemon log by whoever spawned it, so there is
/// one place logs go and the daemon never decides it.
fn init_tracing() {
    let _ = tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_max_level(tracing::Level::INFO)
        .try_init();
}

impl fmt::Display for StartupError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Rendezvous(error) => write!(f, "{error}"),
            Self::State(error) => write!(f, "{error}"),
            Self::Runtime(source) => write!(f, "the async runtime could not start: {source}"),
            Self::Serve(source) => write!(f, "the endpoint could not be served: {source}"),
        }
    }
}
