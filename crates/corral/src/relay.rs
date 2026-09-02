//! `corral hook-relay`: the poorest useful program in the repository, on
//! purpose.
//!
//! It reads one provider hook payload from wherever that provider delivers it,
//! frames it, delivers it to `corrald`'s hook endpoint, takes one receipt, and
//! exits 0. That is the whole contract, and every absence in it is deliberate
//! (ADR 0004 D1):
//!
//! - it never parses the payload, so payload drift cannot break it and
//!   semantic interpretation stays with the daemon's provider adapter;
//! - it never writes to standard output or standard error, and it always exits
//!   0 — Claude Code reads hook stdout and a nonzero exit as decisions, so a
//!   relay that can fail loudly is a relay that can steer the agent;
//! - it never takes the rendezvous lock, never spawns, and never activates:
//!   shims do not start `corrald` (`AGENTS.md`). An absent socket means the
//!   daemon is not running, which means fail open now.
//!
//! Where the payload comes from is the one thing a provider gets to differ
//! about, and it is told rather than guessed: standard input by default, and
//! the invocation's own last argument when the injected command line says so
//! (ADR 0009 D2). Everything after that point is byte-for-byte the same
//! program.
//!
//! And it is bounded end to end. One monotonic deadline covers every phase —
//! reading the payload, resolving the rendezvous, connecting, delivering, and
//! the acknowledgement — and each of the two that can block indefinitely runs on a
//! thread this one stops waiting for when the budget is out. No phase resets
//! the deadline, and a definite error fails open without waiting it out
//! (ADR 0004 D4).

use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::net::UnixStream;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use std::ffi::OsString;

use corral_protocol::hook::{
    HOOK_DELIVER, HookDelivery, MAX_HOOK_PAYLOAD_BYTES, RELAY_PAYLOAD_ARGV_FLAG,
    RELAY_PROVIDER_FLAG, RELAY_SUBCOMMAND, RELAY_TOKEN_FLAG,
};
use corral_protocol::{Frame, MAX_FRAME_BYTES, RequestId, encode_frame};
use corral_rendezvous::RendezvousPaths;

/// The maximum synchronous interference one hook relay invocation may cost the
/// user's agent.
///
/// One deadline for the whole invocation, not a budget per phase. The number is
/// per invocation deliberately: a provider that fires five hooks in one
/// operation can accumulate five of these, which is a composition of its
/// calling pattern rather than a budget this program was granted
/// (ADR 0004 D4).
pub const INTERFERENCE_BUDGET: Duration = Duration::from_millis(50);

/// What this invocation is, if it is a hook delivery at all.
///
/// Recognised without a parser, and that is the whole point. A command line
/// argument parser answers a command line it does not understand by writing
/// usage to standard error and exiting non-zero — and Claude Code reads a
/// non-zero hook exit as a blocking decision and hands the standard error to
/// the model. A relay that reached one could steer the agent by failing to
/// recognise itself.
///
/// Skew is the case this exists for, and ADR 0004 D3 says skew is normal: an
/// injected settings file names an absolute path and invokes whatever is
/// installed by the time an event fires, so a flag this build has no word for
/// is an ordinary thing to meet. Unknown arguments are ignored, a missing
/// value reads as absent, and the daemon is left to refuse what it cannot
/// place — which it does silently, as everything on this path does.
pub struct Invocation {
    pub provider: String,
    /// The launch this entry belongs to, and `None` for a globally installed
    /// entry that belongs to none (ADR 0014 D1).
    ///
    /// An empty `--token` reads as absent for the same reason a missing value
    /// does: this program's whole way of failing is silence, and the daemon
    /// is what refuses a delivery it cannot place.
    pub token: Option<String>,
    pub payload: PayloadSource,
}

/// Where this invocation's payload is.
///
/// Named rather than inferred from an empty argument. A relay that fell back
/// to standard input when the last argument did not look like a payload would
/// spend the whole interference budget waiting on a pipe the provider never
/// opened, once per event — and a relay that read standard input *as well*
/// would be a second, undeclared way for a payload to arrive.
pub enum PayloadSource {
    /// The provider writes it to standard input and closes.
    Stdin,
    /// The provider appended it as the invocation's final argument
    /// (ADR 0009 D2). Held verbatim: the bytes are the provider's, and the
    /// relay never parses them.
    Argument(Vec<u8>),
}

/// Read a hook delivery out of this process's own arguments, or `None` when
/// this invocation is something else entirely.
pub fn invocation(arguments: impl IntoIterator<Item = OsString>) -> Option<Invocation> {
    let mut arguments = arguments.into_iter().skip(1).peekable();
    if arguments.next()? != *RELAY_SUBCOMMAND {
        return None;
    }
    let mut provider = String::new();
    let mut token = String::new();
    let mut from_argument = false;
    // The invocation's own last word, whatever it turns out to be. Kept as it
    // goes past rather than sought afterwards, because the payload is the
    // final argument of the *invocation* — appended by the provider after
    // everything Corral wrote — and nothing but position identifies it.
    let mut last = None;
    while let Some(argument) = arguments.next() {
        // A flag whose value is missing leaves the field empty. The daemon
        // refuses what it cannot place, and silence is this program's whole
        // way of failing.
        let mut named = |value: &mut String| {
            // Peeked, not taken: a word that looks like a flag is not this
            // flag's value, and swallowing it would make one missing value into
            // two — the field holding the next flag's name, and that flag's own
            // value read as a stray word.
            let is_value = arguments
                .peek()
                .is_some_and(|next| !next.to_string_lossy().starts_with("--"));
            if is_value && let Some(next) = arguments.next() {
                *value = next.to_string_lossy().into_owned();
            }
        };
        match argument.to_string_lossy().as_ref() {
            RELAY_PROVIDER_FLAG => named(&mut provider),
            RELAY_TOKEN_FLAG => named(&mut token),
            RELAY_PAYLOAD_ARGV_FLAG => from_argument = true,
            _ => {}
        }
        last = Some(argument);
    }
    // The flag itself as the last word is an invocation that declared an argv
    // payload and carries none. There is nothing to deliver and nothing
    // truthful to say about it, and standard input is not a fallback: this is
    // the silent failure every path here ends in.
    let payload = match (from_argument, last) {
        (false, _) => PayloadSource::Stdin,
        (true, Some(last)) if last != *RELAY_PAYLOAD_ARGV_FLAG => {
            // Verbatim bytes, not a lossy string. A payload is the provider's
            // to write and the daemon's to read; substituting U+FFFD for a
            // byte on the way through would hand the daemon a payload no
            // provider produced.
            PayloadSource::Argument(last.as_bytes().to_vec())
        }
        (true, _) => PayloadSource::Argument(Vec::new()),
    };
    Some(Invocation {
        provider,
        token: (!token.is_empty()).then_some(token),
        payload,
    })
}

/// This process's parent, as the operating system reports it.
///
/// The provider process is up this chain, usually one shell away, and finding
/// which one is the daemon's work. What the relay contributes is the starting
/// point, which it can only read about itself.
fn parent_pid() -> u32 {
    std::os::unix::process::parent_id()
}

/// Deliver one hook event, and never do anything else.
///
/// `started` is taken by the caller, before the command line was even read, so
/// the budget covers the invocation rather than the part of it this function
/// can see.
pub fn deliver(invocation: &Invocation, started: Instant) -> ExitCode {
    // Every path below returns success. The value is built once, here, so that
    // a later edit cannot introduce a branch that reports failure to a
    // provider that would read it as a decision.
    let fail_open = ExitCode::SUCCESS;

    let payload = match &invocation.payload {
        // Already in hand, and read before this process was even started. The
        // budget is untouched by it — which is the whole of what an argv
        // payload changes.
        PayloadSource::Argument(payload) => {
            if payload.is_empty() {
                return fail_open;
            }
            payload.clone()
        }
        PayloadSource::Stdin => match read_payload(remaining(started)) {
            Some(payload) => payload,
            None => return fail_open,
        },
    };
    let Some(delivery) = HookDelivery::new(
        invocation.token.clone(),
        invocation.provider.clone(),
        env!("CARGO_PKG_VERSION").to_owned(),
        &payload,
    ) else {
        return fail_open;
    };
    // Two numbers the operating system already holds. The relay reports where
    // it stood and never walks anywhere: the walk is the daemon's, because
    // this process is short-lived by contract and has a budget to keep
    // (ADR 0014 D2).
    let delivery = delivery.observed_at(std::process::id(), parent_pid());
    // Two different failures, kept apart. A frame too large for the channel is
    // what the payload marker exists for; a delivery that will not encode at
    // all is a definite error to fail open on, and answering that one with the
    // oversize marker would state a reason that is not the reason
    // (`corral_protocol::hook`).
    let Some(frame) = encoded(&delivery) else {
        return fail_open;
    };
    let frame = if frame.len() <= MAX_FRAME_BYTES {
        frame
    } else {
        match encoded(&delivery.without_payload()) {
            Some(marked) if marked.len() <= MAX_FRAME_BYTES => marked,
            _ => return fail_open,
        }
    };

    // Everything left can block for as long as the machine wants to, so it
    // goes on a thread and the deadline is enforced here.
    //
    // Resolving the rendezvous reads the account database, which on a machine
    // whose directory service is remote or unreachable is a call with no bound
    // of its own; and a Unix-domain connect to a listener whose backlog is
    // full blocks on Linux rather than being refused, so a stalled daemon
    // would park every hook of every session in `connect`. Neither is
    // something a shim may make the user's agent wait for (`AGENTS.md`).
    deliver_within(remaining(started), move || {
        // Derived, never passed in: the relay and the daemon compute the same
        // rendezvous from the same rule, so a settings file cannot outlive a
        // daemon and point a later one somewhere else (ADR 0001 D1).
        let Ok(paths) = RendezvousPaths::canonical() else {
            return;
        };
        // A missing socket, a refused connection, a permission failure:
        // definite errors, answered now rather than by waiting out the budget.
        let Ok(stream) = UnixStream::connect(paths.hook_socket()) else {
            return;
        };
        let _ = send_and_settle(stream, &frame, started);
    });
    fail_open
}

/// Run the delivery, and stop waiting for it when the budget is out.
///
/// The thread is abandoned rather than joined, exactly as the stdin reader is:
/// this process is about to exit, and waiting for it would reintroduce the
/// wait that was just avoided. What it is still doing when that happens — a
/// name lookup, a connect, a write — is the daemon's problem or the machine's,
/// and by then it is no longer the agent's.
fn deliver_within(budget: Duration, delivery: impl FnOnce() + Send + 'static) {
    if budget.is_zero() {
        return;
    }
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    std::thread::spawn(move || {
        delivery();
        let _ = sender.send(());
    });
    let _ = receiver.recv_timeout(budget);
}

/// One delivery as bytes, or nothing when it will not encode at all.
///
/// Size is the caller's to judge, not this function's: encoding and being
/// carryable are two questions with two different answers, and `encode_frame`
/// enforces no length of its own — the limit is checked where a frame is
/// decoded.
fn encoded(delivery: &HookDelivery) -> Option<Vec<u8>> {
    let params = serde_json::to_value(delivery).ok()?;
    encode_frame(&Frame::request(RequestId(0), HOOK_DELIVER, Some(params))).ok()
}

/// Write the message and wait for the one receipt the protocol requires.
fn send_and_settle(mut stream: UnixStream, frame: &[u8], started: Instant) -> std::io::Result<()> {
    let left = remaining(started);
    if left.is_zero() {
        return Ok(());
    }
    stream.set_write_timeout(Some(left))?;
    stream.write_all(frame)?;
    stream.flush()?;

    let left = remaining(started);
    if left.is_zero() {
        return Ok(());
    }
    stream.set_read_timeout(Some(left))?;
    // Read, not interpreted. The ack carries receipt and never a decision, so
    // there is nothing in it to act on — what the wait is for is the protocol's
    // one round trip, and its content is not this program's business.
    let mut ack = String::new();
    BufReader::new(stream).read_line(&mut ack)?;
    Ok(())
}

/// The provider's hook stdin, bounded by size and by what is left of the
/// budget.
///
/// Read on a thread of its own so the deadline covers it. Standard input here
/// is a pipe the provider writes and closes, but "usually closes promptly" is
/// not a bound, and a relay parked on a pipe forever is exactly the
/// indefinite block the fail-open law forbids. The thread is abandoned rather
/// than joined: this process is about to exit, and waiting for it would
/// reintroduce the wait that was just avoided.
///
/// One byte past the cap is kept on purpose, so that oversize is *observed*
/// rather than inferred from a payload that happens to end at the limit.
///
/// What the cap is read is handed back **before** the rest is drained, and the
/// order matters both ways. Draining first would let a payload far past the
/// cap run out the budget and lose the oversize marker with it — a systematic
/// oversize has to be visible rather than silently missing (ADR 0004 D3).
/// Draining at all is what stops a relay that has read enough from handing the
/// provider a broken pipe while it is still writing, which is one of the few
/// ways a shim that never writes and always exits 0 could still perturb the
/// agent. So: answer, then keep reading for as long as this process lives.
fn read_payload(budget: Duration) -> Option<Vec<u8>> {
    if budget.is_zero() {
        return None;
    }
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let mut payload = Vec::new();
        let mut stdin = std::io::stdin().lock();
        let read = (&mut stdin)
            .take(MAX_HOOK_PAYLOAD_BYTES as u64 + 1)
            .read_to_end(&mut payload);
        let oversize = read.is_ok() && payload.len() > MAX_HOOK_PAYLOAD_BYTES;
        // The answer goes back first: everything after this is politeness to
        // the writer, and politeness may not cost the fact.
        let _ = sender.send(read.map(|_| payload));
        if oversize {
            let _ = std::io::copy(&mut stdin, &mut std::io::sink());
        }
    });
    receiver.recv_timeout(budget).ok()?.ok()
}

/// What is left of the invocation's budget.
fn remaining(started: Instant) -> Duration {
    INTERFERENCE_BUDGET.saturating_sub(started.elapsed())
}

#[cfg(test)]
#[path = "relay_tests.rs"]
mod tests;
