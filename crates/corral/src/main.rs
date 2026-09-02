#![forbid(unsafe_code)]

//! The `corral` command line: the first surface of the session-first control
//! center, and for now the one that proves the client → daemon path works.
//!
//! Every failure is reported as facts plus a direction. The command never
//! decides on the user's behalf to upgrade, downgrade, or stop a daemon.

use std::process::ExitCode;
use std::time::SystemTime;

use clap::{Parser, Subcommand};

mod relay;
use corral_client::{ActivationError, ClientActivationPolicy, Connection, RequestError, activate};
use corral_protocol::method::{self, SessionListItem};
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
    ///
    /// An agent's own arguments need the separator here, where the list's
    /// prompt takes them with or without it. The stricter shape is not a
    /// preference: the separator is the only thing that tells a provider from
    /// a raw command, and a parser that let it be optional after the provider
    /// would have to be given the separator to see — which clap never does, it
    /// consumes it. `corral new -- bash` and `corral new bash` would become
    /// the same words, and the two namespaces grill Q6 kept apart would
    /// collapse into whichever the daemon guessed. A person who types the
    /// shorter form is told the exact fix by the parser.
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
        /// Answer yes in advance to anything corrald must disclose before
        /// continuing, such as that it cannot tell whether a session found
        /// in an agent's history is still in use elsewhere.
        #[arg(long)]
        yes: bool,
    },
    /// Attach to a session corrald is already running.
    Attach {
        /// The session's id, or enough of its start to be unambiguous.
        session: String,
    },
    /// List the sessions that need you, and the ones ready for you.
    Needs,
    /// Acknowledge a session's current attention item.
    ///
    /// The daemon is told which item this command saw, never "whatever is
    /// current": a delayed acknowledgement must not clear the next real
    /// blocker (grill Q18).
    Ack {
        /// The session's id, or enough of its start to be unambiguous.
        session: String,
    },
    /// Read or annotate the attention journal — diagnostic evidence, never
    /// product state.
    Attention {
        #[command(subcommand)]
        action: AttentionAction,
    },
    /// Open the session list.
    Tui,
    /// See or change how Corral integrates with a provider.
    ///
    /// The daemon performs every operation: a client never writes a
    /// provider's configuration itself (ADR 0013 D1).
    Integration {
        #[command(subcommand)]
        action: IntegrationAction,
    },
}

#[derive(Debug, Subcommand)]
enum AttentionAction {
    /// Count the journal's transitions per day, naming incomplete days.
    Report {
        /// From this day, inclusive (YYYY-MM-DD).
        #[arg(long)]
        since: Option<String>,
    },
    /// Record that a session's current attention item was wrong.
    Dispute {
        /// The session's id, or enough of its start to be unambiguous.
        session: String,
    },
}

#[derive(Debug, Subcommand)]
enum IntegrationAction {
    /// Report what Corral's integration looks like, changing nothing.
    Status {
        /// The agent to ask about.
        provider: String,
    },
    /// Let Corral discover this agent's sessions, and install what that needs.
    Enable {
        /// The agent to integrate with.
        provider: String,
    },
    /// Stop Corral discovering this agent's sessions, and take its entries out.
    Disable {
        /// The agent to stop integrating with.
        provider: String,
    },
}

/// Synchronous, and that is for the relay's sake.
///
/// `#[tokio::main]` would build a reactor before this body ran and tear one
/// down after it returned — on every hook invocation, several per agent turn,
/// for a program that is entirely synchronous and uses none of it. Worse, the
/// construction would sit *outside* the interference budget below while the
/// user's agent waited for it, so the number would understate what a hook
/// actually costs (ADR 0004 D4). Everything that does need a reactor gets one
/// after the relay has been answered.
fn main() -> ExitCode {
    // Before the command line is even read: the relay's budget is the
    // interference one hook invocation costs the user's agent, and reading the
    // arguments is part of the invocation (ADR 0004 D4).
    let started = std::time::Instant::now();

    // Before the parser, before activation, and before a reactor exists.
    //
    // Before the parser because a parser answers a command line it does not
    // understand by writing usage to standard error and exiting non-zero,
    // which Claude Code reads as a blocking hook decision — so an injected
    // settings file naming a flag this build does not know would let the shim
    // steer the agent by failing to recognise itself. Skew is normal: that
    // file is written at launch and invokes whatever is installed when an
    // event fires (ADR 0004 D3).
    //
    // Before activation because shims never start `corrald`: one that could
    // would delay the user's agent by however long a cold start takes
    // (ADR 0004 D1).
    if let Some(relay) = relay::invocation(std::env::args_os()) {
        return relay::deliver(&relay, started);
    }

    let cli = Cli::parse();
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(source) => {
            eprintln!("corral: this surface could not start: {source}");
            return ExitCode::FAILURE;
        }
    };
    runtime.block_on(serve(cli))
}

async fn serve(cli: Cli) -> ExitCode {
    let policy = ClientActivationPolicy::resolve();

    let mut connection = match activate(&policy).await {
        Ok(connection) => connection,
        Err(error) => return report_activation_failure(&error),
    };

    match cli.command {
        Command::Ping => ping(&mut connection).await,
        Command::List => list(&mut connection).await,
        Command::New { provider, rest } => new_session(&mut connection, provider, rest).await,
        Command::Continue { session, yes } => {
            continue_session(&mut connection, &session, yes).await
        }
        Command::Attach { session } => attach(&mut connection, &session).await,
        Command::Needs => needs(&mut connection).await,
        Command::Ack { session } => acknowledge(&mut connection, &session).await,
        Command::Attention { action } => attention(&mut connection, action).await,
        Command::Tui => session_list(&policy, connection).await,
        Command::Integration { action } => integration(&mut connection, action).await,
    }
}

/// The rows that need the person, then the ones ready for them, as the
/// daemon claims them. Nothing is derived here: a row is in this list
/// because the daemon's attention field says so (ADR 0015 D1).
async fn needs(connection: &mut Connection) -> ExitCode {
    let sessions = match connection.session_list().await {
        Ok(sessions) => sessions,
        Err(error) => return report_request_failure(&error),
    };
    let now = SystemTime::now();
    let mut needing = Vec::new();
    let mut ready = Vec::new();
    for session in &sessions.sessions {
        let Ok(item) = serde_json::from_value::<SessionListItem>(session.clone()) else {
            continue;
        };
        match item.attention.as_ref().map(|facts| &facts.state) {
            Some(corral_protocol::method::AttentionWireState::NeedsYou) => needing.push(item),
            Some(corral_protocol::method::AttentionWireState::Ready) => ready.push(item),
            _ => {}
        }
    }
    if needing.is_empty() && ready.is_empty() {
        println!("Nothing needs you.");
        return ExitCode::SUCCESS;
    }
    for item in needing.iter().chain(ready.iter()) {
        for row in session_rows(item, now) {
            println!("{row}");
        }
    }
    ExitCode::SUCCESS
}

/// The session's current unacknowledged item, by the id this command saw.
async fn current_item(
    connection: &mut Connection,
    session: &str,
) -> Result<Option<String>, ExitCode> {
    let listed = match connection.session_list().await {
        Ok(listed) => listed,
        Err(error) => return Err(report_request_failure(&error)),
    };
    Ok(listed
        .sessions
        .iter()
        .filter_map(|value| serde_json::from_value::<SessionListItem>(value.clone()).ok())
        .find(|item| item.session_id == session)
        .and_then(|item| {
            corral_tui::present_at(&item, SystemTime::now())
                .acknowledgeable()
                .map(str::to_owned)
        }))
}

async fn acknowledge(connection: &mut Connection, session: &str) -> ExitCode {
    let resolved = match resolve_session(connection, session).await {
        Ok(resolved) => resolved,
        Err(code) => return code,
    };
    let Some(item) = (match current_item(connection, &resolved).await {
        Ok(item) => item,
        Err(code) => return code,
    }) else {
        println!("Nothing to acknowledge.");
        return ExitCode::SUCCESS;
    };
    match connection.attention_acknowledge(&resolved, &item).await {
        Ok(()) => {
            println!("Acknowledged.");
            ExitCode::SUCCESS
        }
        Err(error) => report_request_failure(&error),
    }
}

async fn attention(connection: &mut Connection, action: AttentionAction) -> ExitCode {
    match action {
        AttentionAction::Report { since } => {
            let report = match connection.attention_report(since.as_deref()).await {
                Ok(report) => report,
                Err(error) => return report_request_failure(&error),
            };
            if report.days.is_empty() {
                println!("No attention journal days.");
                return ExitCode::SUCCESS;
            }
            println!(
                "{:<12}{:>12}{:>12}{:>8}{:>10}",
                "day", "transitions", "needs you", "ready", "disputes"
            );
            for day in &report.days {
                println!(
                    "{:<12}{:>12}{:>12}{:>8}{:>10}  {}",
                    day.date,
                    day.transitions,
                    day.into_needs_you,
                    day.into_ready,
                    day.disputes,
                    if day.incomplete { "INCOMPLETE" } else { "" }
                );
            }
            ExitCode::SUCCESS
        }
        AttentionAction::Dispute { session } => {
            let resolved = match resolve_session(connection, &session).await {
                Ok(resolved) => resolved,
                Err(code) => return code,
            };
            let item = match current_item(connection, &resolved).await {
                Ok(item) => item,
                Err(code) => return code,
            };
            match connection
                .attention_dispute(&resolved, item.as_deref())
                .await
            {
                Ok(answer) if answer.stale => {
                    println!(
                        "Recorded; that item was already gone by the time the dispute arrived."
                    );
                    ExitCode::SUCCESS
                }
                Ok(_) => {
                    println!("Recorded.");
                    ExitCode::SUCCESS
                }
                Err(error) => report_request_failure(&error),
            }
        }
    }
}

/// Ask the daemon about a provider's integration, or ask it to change one.
///
/// Every answer says the same three things: what the standing is, whether
/// Corral can expect this agent's sessions to report, and — when there is
/// something to explain — what Corral found and did not do.
async fn integration(connection: &mut Connection, action: IntegrationAction) -> ExitCode {
    let (method, provider) = match &action {
        IntegrationAction::Status { provider } => (method::INTEGRATION_STATUS, provider),
        IntegrationAction::Enable { provider } => (method::INTEGRATION_ENABLE, provider),
        IntegrationAction::Disable { provider } => (method::INTEGRATION_DISABLE, provider),
    };

    let answer = match connection.integration(method, provider).await {
        Ok(answer) => answer,
        Err(error) => return report_request_failure(&error),
    };

    println!("{} · {}", answer.provider, describe(&answer.standing));
    if let Some(path) = &answer.path {
        println!("  configuration {path}");
    }
    if let Some(detail) = &answer.detail {
        println!("  {detail}");
    }
    if !answer.claims_delivery {
        // The one sentence a person acts on. Limited awareness is a product
        // state, not an error, so this is printed and the command succeeds
        // (`PRODUCT.md` §6).
        println!("  Sessions from this agent show Limited awareness until this is resolved.");
    }
    ExitCode::SUCCESS
}

/// A standing in the words a person reads.
///
/// A value this build has no word for is rendered as it arrived rather than
/// refused: a newer daemon may name a standing this client predates, and
/// showing the raw word beats showing nothing (`AGENTS.md` §Protocol).
fn describe(standing: &str) -> String {
    match standing {
        method::STANDING_INSTALLED => "integrated".to_owned(),
        method::STANDING_NOT_INSTALLED => "not integrated".to_owned(),
        method::STANDING_DRIFTED => "integrated, but not by this version of Corral".to_owned(),
        method::STANDING_REFUSED => {
            "not integrated · Corral did not change your configuration".to_owned()
        }
        method::STANDING_REPAIR_WITHHELD => {
            "not integrated · Corral stopped repairing it".to_owned()
        }
        other => other.to_owned(),
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
            // states it — and only for the refusal it answers. Printed after
            // any other failure it would send a person to start an *unmanaged*
            // session, which is the opposite of what they asked for.
            if let (Some(named), true) = (named, unknown_agent(&error)) {
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

/// Whether the daemon refused because it does not integrate that agent.
///
/// Read off the code rather than the sentence: behaviour hangs off the code
/// alone, and a hint matched against prose would drift the first time the
/// prose did.
fn unknown_agent(error: &RequestError) -> bool {
    matches!(
        error,
        RequestError::Refused(refusal) if refusal.code == corral_protocol::ErrorCode::UnknownProvider
    )
}

/// Continue a session as a new run, and attach to it.
async fn continue_session(connection: &mut Connection, session: &str, yes: bool) -> ExitCode {
    let resolved = match resolve_session(connection, session).await {
        Ok(resolved) => resolved,
        Err(code) => return code,
    };
    let shown = if yes {
        corral_tui::Shown::Accepted
    } else {
        corral_tui::Shown::NotYet
    };
    let continued = match corral_tui::continue_session(connection, &resolved, shown).await {
        Ok(corral_tui::Continued::Started { started, disclosed }) => {
            if let Some(disclosed) = disclosed {
                eprintln!("{disclosed}");
            }
            started
        }
        Ok(corral_tui::Continued::NeedsDisclosure { text, .. }) => {
            eprintln!("{text}");
            eprintln!("To continue anyway: corral continue --yes {resolved}");
            return ExitCode::FAILURE;
        }
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

    // One instant for the whole listing. A row that read the clock for itself
    // would let two facts observed at the same moment print two different
    // ages, which is a listing disagreeing with itself.
    let now = SystemTime::now();
    let mut unrenderable = 0;
    for session in &sessions.sessions {
        match serde_json::from_value::<SessionListItem>(session.clone()) {
            Ok(item) => {
                for row in session_rows(&item, now) {
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
fn session_rows(item: &SessionListItem, now: SystemTime) -> Vec<String> {
    let presented = corral_tui::present_at(item, now);
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
