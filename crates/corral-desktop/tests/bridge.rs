//! The bridge against a real `corrald`: the hello, an attachment, the
//! snapshot prefix, resync, resize, detach, and a daemon that goes away.
//!
//! The daemon is a staged, validated test-support build under a private
//! account (`corral-e2e`), reached through an explicit endpoint: the bridge
//! connects and never activates, so a wrong binary is a test failure and
//! never a production daemon (PR9 plan, round 2 Q12). Nothing corrald's own
//! fidelity suite proves is repeated here; what is crossed is the public
//! client contract.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::time::{Duration, SystemTime};

use corral_client::launch::{LaunchSite, Requested};
use corral_client::{ClientActivationPolicy, EndpointSelection};
use corral_desktop::bridge::{Attached, Bridge, Unanswered};
use corral_desktop::quit::{self, Continuing, Gate};
use corral_desktop::replica::{Geometry, Replica};
use corral_desktop::sessions::SessionList;
use corral_e2e::TestAccount;
use corral_protocol::terminal::{FrameKind, Sequence, TerminalFrame};
use futures::StreamExt;
use futures::channel::oneshot;

/// How long a test waits for the daemon.
const WITHIN: Duration = Duration::from_secs(10);

fn bridge_for(account: &TestAccount) -> Bridge {
    Bridge::start(
        ClientActivationPolicy::default(),
        EndpointSelection::Explicit(account.socket()),
    )
}

async fn answered<T>(reply: oneshot::Receiver<T>) -> T {
    tokio::time::timeout(WITHIN, reply)
        .await
        .expect("the bridge answered in time")
        .expect("the bridge is alive")
}

async fn next_frame(attached: &mut Attached) -> TerminalFrame {
    tokio::time::timeout(WITHIN, attached.inbound.next())
        .await
        .expect("a frame in time")
        .expect("the channel is open")
        .frame
}

/// Frames up to and including the next snapshot.
async fn prefix(attached: &mut Attached) -> Vec<TerminalFrame> {
    let mut frames = Vec::new();
    loop {
        let frame = next_frame(attached).await;
        let done = frame.kind == FrameKind::Snapshot;
        frames.push(frame);
        if done {
            return frames;
        }
    }
}

fn kinds(frames: &[TerminalFrame]) -> Vec<FrameKind> {
    frames.iter().map(|frame| frame.kind).collect()
}

fn first_row(replica: &Replica) -> String {
    let window = replica.window().expect("a screen");
    window.window[0]
        .cells
        .iter()
        .filter(|cell| !cell.is_spacer())
        .map(|cell| cell.ch)
        .collect::<String>()
        .trim_end()
        .to_owned()
}

async fn start_shell(bridge: &Bridge, account: &TestAccount, script: &str) -> String {
    let started = answered(bridge.start_session(
        // The raw harness PR3 left for tests: no provider is launched here,
        // and the Desktop's own form can only ever ask for one.
        Requested::Command(vec!["sh".to_owned(), "-c".to_owned(), script.to_owned()]),
        LaunchSite {
            working_directory: Some(account.scratch().to_path_buf()),
            rows: Some(10),
            cols: Some(40),
        },
    ))
    .await
    .expect("the session started");
    started.session_id
}

#[tokio::test]
async fn the_hello_reaches_the_list_and_a_channel_opens_with_the_prefix() {
    let account = TestAccount::new("desktop-prefix");
    let _daemon = account.start_daemon();
    let bridge = bridge_for(&account);

    let polled = answered(bridge.poll()).await.expect("a poll");
    assert!(polled.capabilities.managed_sessions);
    assert!(polled.capabilities.geometry, "terminal.geometry.v1");
    assert!(polled.capabilities.palette, "terminal.palette.v1");
    assert!(polled.listing.items.is_empty());

    let session_id = start_shell(&bridge, &account, "printf 'hello\\n'; sleep 30").await;
    let mut attached = answered(bridge.attach(session_id.clone()))
        .await
        .expect("attached");
    assert_eq!(attached.geometry, Geometry { rows: 10, cols: 40 });
    assert!(attached.promised.geometry && attached.promised.palette);

    // ADR 0017 D4: Geometry, then the Snapshot, stamped alike; no Palette,
    // because this session never touched it.
    let frames = prefix(&mut attached).await;
    assert_eq!(kinds(&frames), [FrameKind::Geometry, FrameKind::Snapshot]);
    assert_eq!(frames[0].epoch, frames[1].epoch);
    assert_eq!(frames[0].sequence, frames[1].sequence);
    assert_eq!(
        Geometry::decode(&frames[0].payload),
        Some(Geometry { rows: 10, cols: 40 })
    );

    // Through the replica, the shell's output arrives — in the snapshot or
    // in a delta after it.
    let mut replica = Replica::new(attached.promised);
    for frame in &frames {
        replica.apply(frame);
    }
    let deadline = tokio::time::Instant::now() + WITHIN;
    while first_row(&replica) != "hello" {
        assert!(
            tokio::time::Instant::now() < deadline,
            "hello never arrived"
        );
        let frame = next_frame(&mut attached).await;
        assert!(!replica.apply(&frame).resync, "the stream desynchronised");
    }
    let epoch = replica.epoch();

    // A resync is answered by the full prefix, inside the same epoch.
    attached.outbound.send(TerminalFrame {
        kind: FrameKind::ResyncRequest,
        epoch,
        sequence: Sequence(0),
        payload: Vec::new(),
    });
    let frames = prefix(&mut attached).await;
    assert_eq!(kinds(&frames), [FrameKind::Geometry, FrameKind::Snapshot]);
    assert_eq!(frames[1].epoch, epoch);
    let mut fresh = Replica::new(attached.promised);
    for frame in &frames {
        fresh.apply(frame);
    }
    assert_eq!(first_row(&fresh), "hello");

    // A resize opens a new epoch whose prefix carries the new size.
    attached.outbound.send(TerminalFrame {
        kind: FrameKind::Resize,
        epoch,
        sequence: Sequence(0),
        payload: Geometry { rows: 12, cols: 50 }.encode(),
    });
    let frames = prefix(&mut attached).await;
    assert_eq!(kinds(&frames), [FrameKind::Geometry, FrameKind::Snapshot]);
    assert!(frames[1].epoch.0 > epoch.0, "a resize is a new epoch");
    assert_eq!(
        Geometry::decode(&frames[0].payload),
        Some(Geometry { rows: 12, cols: 50 })
    );

    // Detaching closes the channel; the run lives on.
    drop(attached.outbound);
    let ended = tokio::time::timeout(WITHIN, async {
        while attached.inbound.next().await.is_some() {}
    })
    .await;
    assert!(ended.is_ok(), "the channel did not end after detaching");
    let polled = answered(bridge.poll()).await.expect("a poll");
    let row = polled
        .listing
        .items
        .iter()
        .find(|item| item.session_id == session_id)
        .expect("the session is still listed");
    assert_eq!(row.execution_state, "running");
}

/// A daemon that goes away is reported as silent, never as anything else,
/// and one that comes back answers again without the Desktop restarting.
/// Through an explicit endpoint nothing is ever started by this client.
/// The Quit gate counts what the daemon says it started (tray grill Q11): a
/// session launched through it is `managed` and `running` in its own words,
/// so one is R = 1 and Quit warns.
#[tokio::test]
async fn a_session_the_daemon_started_counts_as_continuing() {
    let account = TestAccount::new("desktop-quit-gate");
    let _daemon = account.start_daemon();
    let bridge = bridge_for(&account);

    let session_id = start_shell(&bridge, &account, "sleep 30").await;
    let polled = answered(bridge.poll()).await.expect("a poll");
    let mut list = SessionList::default();
    list.take(Ok(polled), SystemTime::now());

    assert!(list.is_current());
    assert!(list.rows().iter().any(|row| row.session_id == session_id));
    assert_eq!(
        quit::continuing(list.rows()),
        Continuing {
            running: 1,
            unverified: 0
        }
    );
    let Gate::Warn(warning) = quit::gate(&list) else {
        panic!("a running session Corral started warns");
    };
    assert_eq!(warning.message, "1 session will continue running.");
}

#[tokio::test]
async fn a_lost_daemon_is_reported_and_a_restarted_one_answers_again() {
    let account = TestAccount::new("desktop-lost");
    let daemon = account.start_daemon();
    let bridge = bridge_for(&account);
    answered(bridge.poll()).await.expect("a poll");

    drop(daemon);
    let lost = answered(bridge.poll())
        .await
        .expect_err("the daemon is gone");
    assert!(matches!(lost, Unanswered::Silent(_)), "{lost:?}");
    assert!(
        !account.socket().exists() || !corral_e2e::lock_is_held(&account.lock()),
        "nothing was started by the client"
    );

    let _daemon = account.start_daemon();
    // The activation backoff holds for a second after a failure; the poll
    // keeps its cadence and the next one after it answers.
    let deadline = tokio::time::Instant::now() + WITHIN;
    loop {
        match answered(bridge.poll()).await {
            Ok(_) => break,
            Err(unanswered) => {
                assert!(
                    matches!(unanswered, Unanswered::Silent(_)),
                    "{unanswered:?}"
                );
                assert!(tokio::time::Instant::now() < deadline, "never reconnected");
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
        }
    }
}
