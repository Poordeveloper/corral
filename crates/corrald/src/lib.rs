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

/// Walking from a relay to the provider process that ran it (ADR 0014 D2).
mod ancestry;
mod attention;
mod connection;
/// Sessions Corral found rather than started (ADR 0014).
mod external_session;
mod hook_endpoint;
mod hook_evidence;
mod in_flight;
/// The one mutator of a user's own provider configuration (ADR 0013).
mod integration;
mod lifecycle;
mod managed_launch;
mod platform;
mod policy;
/// Coding-agent knowledge: launch and resume composition, hook ingress
/// interpretation, and the artifacts a managed launch leaves behind. Public
/// for the same reason `runtime` is — the scenarios this crate owes are
/// integration tests, and an integration test reaches only what the library
/// exposes.
pub mod provider;
mod run_lifecycle;
/// The managed runtime. Public because the lifecycle scenarios this crate owes
/// — detach, disconnect, restart, crash, unverifiable exit — are integration
/// tests, and an integration test reaches only what the library exposes.
pub mod runtime;
mod server;
mod state;
/// Finding provider runtimes that never sent Corral anything (ADR 0014 D2).
mod sweep;
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
    // One call: the launch directory sits inside the durable-state tree, and
    // ensuring it ensures the tree. Asking for both walked and re-checked the
    // same two directories twice, and gave the ownership question two callers
    // to drift about.
    paths
        .ensure_launch_dir()
        .map_err(StartupError::Rendezvous)?;
    let state = Arc::new(
        DaemonState::open(paths.registry(), paths.launch_dir(), paths.state_dir())
            .map_err(StartupError::State)?,
    );

    // Before the endpoint is bound, and only by the daemon holding the claim.
    // Every managed episode still open belongs to a daemon that is gone, and a
    // managed runtime does not survive its owning daemon (ADR 0007 L6) — so
    // these are closed as unverifiable, which is what Corral can say rather
    // than a claim that any process exited (grill Q5).
    for run in state
        .reconcile_managed_runs()
        .map_err(StartupError::State)?
    {
        info!(%run, "a managed run from a previous corrald is recorded as unverifiable");
    }

    // After reconciliation, because reconciliation is what turns a departed
    // daemon's open episodes into recorded endings — and only a recorded
    // ending is evidence strong enough to destroy the artifact that named it.
    // Nothing here removes a file whose Run's fate is unestablished: losing
    // Corral's ownership is not proof the provider process is dead (grill Q10).
    let swept = Arc::clone(&state);
    provider::sweep_launch_dir(paths.launch_dir(), move |run| swept.exit_established(run));

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(StartupError::Runtime)?;
    runtime
        .block_on(server::serve(
            paths.socket(),
            paths.hook_socket(),
            policy,
            Arc::clone(&state),
        ))
        .map_err(StartupError::Serve)?;

    // Order matters and is stated rather than left to drop order: the runtime
    // and every connection it owns go first, then the rendezvous pathname,
    // and the claim last. While the claim is held a probing client sees an
    // owner and refuses to start a replacement, so nothing can bind into a
    // half-dismantled rendezvous (ADR 0001 D6).
    drop(runtime);
    // Every connection is closed by now, so nothing new will be observed, and
    // whatever is still queued is the last of it. Waiting is the difference
    // between a fact recorded late and a fact nobody ever writes — and a wait
    // that runs out is itself reported, in the exit status below.
    if state.settle_observations() == runtime::Integrity::Lost {
        error!("this daemon could not record everything it observed about its runs");
    }
    // Best effort: the next claim winner owns whatever an abrupt death leaves
    // behind, so failing to unlink here costs nothing.
    //
    // Both pathnames, here, because dropping the runtime cancels the hook
    // endpoint's task wherever it was parked — its own cleanup runs only when
    // the loop exits on its own, which a cancelled task never does. A daemon
    // that left its hook socket behind would be a departed daemon that still
    // looks present to anything reading the path.
    let _ = std::fs::remove_file(paths.socket());
    let _ = std::fs::remove_file(paths.hook_socket());
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
