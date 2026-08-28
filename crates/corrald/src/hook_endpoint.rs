//! The hook endpoint: a second local socket that can only take evidence.
//!
//! Beside the canonical rendezvous rather than multiplexed onto it. Hook
//! versioning would otherwise ride the client hello, and "evidence-only" would
//! be an ACL promise instead of a structural fact: no session method and no
//! control surface is reachable from here, because this dispatcher serves one
//! method and knows no others (ADR 0004 D2).
//!
//! The daemon alone creates and removes the socket. The relay never creates
//! it: an absent socket means `corrald` is not running, which means fail open
//! now.
//!
//! The fence around it is the run directory, which is `0700` and checked on
//! every start. The socket's own `0600` is set after the bind and is belt and
//! braces: `bind` creates the node under the process umask, so there is a
//! window, and closing it would mean tightening a process-wide umask around a
//! call other threads share. The canonical socket is bound the same way for
//! the same reason (ADR 0001: the modes are a transport fence, not a security
//! boundary). A Corral-owned *file* is a different case and is created `0600`
//! outright, because a file is opened rather than bound and the mode is free
//! at creation.
//!
//! Receipt is acknowledged before the event is interpreted. The ack carries
//! receipt and never a decision, so there is nothing for a shim to wait on
//! beyond arrival — and the store's latency is kept off the path that can
//! delay a user's agent (ADR 0004 D4).

use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::time::{Duration, Instant, SystemTime};

use corral_protocol::hook::{HOOK_DELIVER, HOOK_PROTOCOL_VERSION, HookAck, HookDelivery};
use corral_protocol::{Frame, FrameReader, FrameWriter};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::watch;
use tracing::{debug, error, info, warn};

use crate::hook_evidence::{Delivered, Deliveries};
use crate::provider::LaunchToken;

/// How long one hook connection may take to state its business.
///
/// A shim connects, writes one framed message, and waits. Anything slower is
/// not a shim inside its own budget, and holding the connection open would let
/// a stuck writer accumulate against a daemon that has agents to watch.
const DELIVERY_DEADLINE: Duration = Duration::from_secs(2);

/// Keeps a failing accept from spinning the CPU while the cause persists.
const ACCEPT_BACKOFF: Duration = Duration::from_millis(50);

/// Claim the hook endpoint's pathname, before anything can ask for a session
/// that would report through it.
///
/// Separate from serving because the answer has to be known synchronously. A
/// managed launch injects hooks that deliver here; if this fails, that session
/// looks managed and can never report, which is the state
/// `provider::launch::usable_relay` already refuses a launch over. A bind
/// discovered inside a spawned task would be discovered after the first client
/// had already asked.
pub fn bind(socket: &Path) -> io::Result<UnixListener> {
    // Only this daemon owns the pathname. A leftover from a process that died
    // without unlinking would otherwise make the bind fail, and the singleton
    // claim is already what proves no other daemon is serving.
    let _ = std::fs::remove_file(socket);
    let listener = UnixListener::bind(socket)?;
    // The run directory is already user-private; the socket says so too rather
    // than inheriting whatever the umask happened to be. A failure here takes
    // the pathname with it: a node nothing will ever serve reads to anything
    // watching the path as a daemon that is present.
    if let Err(source) = std::fs::set_permissions(socket, PermissionsExt::from_mode(0o600)) {
        drop(listener);
        let _ = std::fs::remove_file(socket);
        return Err(source);
    }
    info!(endpoint = %socket.display(), "the hook endpoint is serving");
    Ok(listener)
}

/// Serve a bound hook endpoint until shutdown is committed.
pub async fn serve(
    socket: &Path,
    listener: UnixListener,
    deliveries: Deliveries,
    mut shutdown: watch::Receiver<bool>,
) {
    loop {
        tokio::select! {
            _ = shutdown.changed() => break,
            accepted = listener.accept() => match accepted {
                Ok((stream, _address)) => {
                    let deliveries = deliveries.clone();
                    tokio::spawn(async move { take_one(stream, deliveries).await });
                }
                Err(source) => {
                    error!(%source, "the hook endpoint could not accept");
                    tokio::time::sleep(ACCEPT_BACKOFF).await;
                }
            },
        }
    }

    // Reached only when the loop leaves on its own. A shutdown that drops the
    // runtime cancels this task wherever it was parked, so the departing
    // daemon unlinks both pathnames itself; this is what makes the ordinary
    // exit tidy rather than what makes it correct.
    drop(listener);
    let _ = std::fs::remove_file(socket);
}

/// One connection carries one message and one ack, and then it is over.
async fn take_one(stream: UnixStream, deliveries: Deliveries) {
    let (read_half, write_half) = stream.into_split();
    let mut reader = FrameReader::new(read_half);
    let mut writer = FrameWriter::new(write_half);

    let framed = tokio::time::timeout(DELIVERY_DEADLINE, reader.read_frame()).await;
    let request = match framed {
        Ok(Ok(Some(Frame::Request(request)))) => request,
        // Everything else is silence. This endpoint answers deliveries; it does
        // not teach a caller what it would have accepted, and a shim has
        // nothing to do with the answer either way.
        Ok(Ok(_)) => return,
        Ok(Err(fault)) => {
            debug!(%fault, "a hook delivery was not framed");
            return;
        }
        Err(_) => {
            debug!("a hook connection did not deliver inside its deadline");
            return;
        }
    };

    if request.method != HOOK_DELIVER {
        debug!(method = %request.method, "the hook endpoint serves no such method");
        return;
    }

    // Stamped here, at arrival, and carried with the delivery: whatever the
    // queue does afterwards, the observation happened now. Both clocks,
    // because they answer different questions — what a surface renders an age
    // from, and what orders this daemon's own observations against each other
    // when the first one steps.
    let observed_at = SystemTime::now();
    let arrived = Instant::now();
    let accepted = read_delivery(request.params);

    // The receipt goes back whether or not the payload was usable. It says
    // "received" and nothing more, and a shim that waited for a verdict would
    // be a shim whose provider waits for one.
    if let Err(source) = writer
        .write_frame(&Frame::result(request.id, HookAck::wire_value()))
        .await
    {
        debug!(%source, "a hook delivery could not be acknowledged");
    }

    if let Some(delivered) = accepted.map(|delivery| Delivered {
        token: delivery.token,
        provider: delivery.provider,
        payload: delivery.payload,
        payload_omitted: delivery.payload_omitted,
        observed_at,
        arrived,
    }) {
        deliveries.offer(delivered);
    }
}

/// A delivery this build can act on, or nothing.
struct Accepted {
    token: LaunchToken,
    provider: String,
    payload: Option<String>,
    payload_omitted: Option<String>,
}

fn read_delivery(params: Option<serde_json::Value>) -> Option<Accepted> {
    let params = params?;
    let delivery: HookDelivery = serde_json::from_value(params)
        .inspect_err(|source| debug!(%source, "a hook delivery did not decode"))
        .ok()?;

    // A version this build does not speak is dropped with diagnostics. The
    // relay exits 0 regardless, because fail-open is not conditional on being
    // understood (ADR 0004 D3).
    if delivery.hook_protocol_version != HOOK_PROTOCOL_VERSION {
        warn!(
            claimed = delivery.hook_protocol_version,
            speaks = HOOK_PROTOCOL_VERSION,
            "a hook delivery stated a contract this build does not speak",
        );
        return None;
    }

    let token = LaunchToken::from_wire(&delivery.launch_token).or_else(|| {
        debug!("a hook delivery carried no usable launch token");
        None
    })?;

    Some(Accepted {
        token,
        provider: delivery.provider,
        payload: delivery.payload,
        payload_omitted: delivery.payload_omitted,
    })
}
