//! `corral hook-relay`: the poorest useful program in the repository, on
//! purpose.
//!
//! It reads one provider hook payload from standard input, frames it, delivers
//! it to `corrald`'s hook endpoint, takes one receipt, and exits 0. That is
//! the whole contract, and every absence in it is deliberate (ADR 0004 D1):
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
//! And it is bounded end to end. One monotonic deadline covers reading stdin,
//! connecting, delivering, and the acknowledgement; no phase resets it, and a
//! definite error fails open without waiting the budget out (ADR 0004 D4).

use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use corral_protocol::hook::{HOOK_DELIVER, HookDelivery, MAX_HOOK_PAYLOAD_BYTES};
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

/// Deliver one hook event, and never do anything else.
///
/// `started` is taken by the caller, before the command line was even parsed,
/// so the budget covers the invocation rather than the part of it this
/// function can see.
pub fn deliver(token: &str, provider: &str, started: Instant) -> ExitCode {
    // Every path below returns success. The value is built once, here, so that
    // a later edit cannot introduce a branch that reports failure to a
    // provider that would read it as a decision.
    let fail_open = ExitCode::SUCCESS;

    let Some(payload) = read_payload(remaining(started)) else {
        return fail_open;
    };
    let Some(delivery) = HookDelivery::new(
        token.to_owned(),
        provider.to_owned(),
        env!("CARGO_PKG_VERSION").to_owned(),
        &payload,
    ) else {
        return fail_open;
    };
    let Some(frame) = framed(&delivery).or_else(|| framed(&delivery.without_payload())) else {
        return fail_open;
    };

    // Derived, never passed in: the relay and the daemon compute the same
    // rendezvous from the same rule, so a settings file cannot outlive a
    // daemon and point a later one somewhere else (ADR 0001 D1).
    let Ok(paths) = RendezvousPaths::canonical() else {
        return fail_open;
    };
    // A missing socket, a refused connection, a permission failure: definite
    // errors, answered now rather than by waiting out the budget.
    let Ok(stream) = UnixStream::connect(paths.hook_socket()) else {
        return fail_open;
    };

    let _ = send_and_settle(stream, &frame, started);
    fail_open
}

/// One delivery as a frame this channel can carry, or nothing.
///
/// `None` when the encoded message is past the framing limit, which is the
/// caller's cue to try the same delivery with its payload marked instead of
/// carried: the endpoint would refuse an oversized frame as a framing fault
/// and the event would vanish with no record of why.
fn framed(delivery: &HookDelivery) -> Option<Vec<u8>> {
    let params = serde_json::to_value(delivery).ok()?;
    let frame = encode_frame(&Frame::request(RequestId(0), HOOK_DELIVER, Some(params))).ok()?;
    (frame.len() <= MAX_FRAME_BYTES).then_some(frame)
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
/// rather than inferred from a payload that happens to end at the limit. The
/// rest is read and discarded rather than left unread: a relay that stops
/// reading while the provider is still writing hands it a broken pipe, which
/// is a way a shim that never writes and always exits 0 could still perturb
/// the agent. Discarding is bounded by the budget like everything else — a
/// write that cannot finish inside the interference ceiling is the one case
/// where returning control wins over reading politely (ADR 0004 D4).
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
        if read.is_ok() && payload.len() > MAX_HOOK_PAYLOAD_BYTES {
            let _ = std::io::copy(&mut stdin, &mut std::io::sink());
        }
        let _ = sender.send(read.map(|_| payload));
    });
    receiver.recv_timeout(budget).ok()?.ok()
}

/// What is left of the invocation's budget.
fn remaining(started: Instant) -> Duration {
    INTERFERENCE_BUDGET.saturating_sub(started.elapsed())
}
