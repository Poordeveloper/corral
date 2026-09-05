use super::*;

const PROMISED_BOTH: Promised = Promised {
    geometry: true,
    palette: true,
};
const PROMISED_GEOMETRY: Promised = Promised {
    geometry: true,
    palette: false,
};
const LEGACY: Promised = Promised {
    geometry: false,
    palette: false,
};

fn frame(kind: FrameKind, epoch: u64, sequence: u64, payload: &[u8]) -> TerminalFrame {
    TerminalFrame {
        kind,
        epoch: Epoch(epoch),
        sequence: Sequence(sequence),
        payload: payload.to_vec(),
    }
}

fn geometry(epoch: u64, sequence: u64, rows: u16, cols: u16) -> TerminalFrame {
    frame(
        FrameKind::Geometry,
        epoch,
        sequence,
        &Geometry { rows, cols }.encode(),
    )
}

fn snapshot(epoch: u64, sequence: u64, text: &str) -> TerminalFrame {
    frame(FrameKind::Snapshot, epoch, sequence, text.as_bytes())
}

fn delta(epoch: u64, sequence: u64, text: &str) -> TerminalFrame {
    frame(FrameKind::Delta, epoch, sequence, text.as_bytes())
}

fn palette(epoch: u64, sequence: u64, osc: &str) -> TerminalFrame {
    frame(FrameKind::Palette, epoch, sequence, osc.as_bytes())
}

fn resync() -> Applied {
    Applied {
        resync: true,
        ..Applied::default()
    }
}

fn redraw() -> Applied {
    Applied {
        redraw: true,
        ..Applied::default()
    }
}

/// A resync asked for while a screen was on display: the screen goes, so
/// the display changes too.
fn resync_hiding() -> Applied {
    Applied {
        redraw: true,
        resync: true,
        ..Applied::default()
    }
}

/// The first row of the screen, trimmed.
fn first_row(replica: &Replica) -> String {
    let window = replica.window().expect("a screen");
    window.window[0]
        .cells
        .iter()
        .filter(|cell| !matches!(cell.width, CellWidth::Spacer))
        .map(|cell| cell.ch)
        .collect::<String>()
        .trim_end()
        .to_owned()
}

use qwertty_term_vt::snapshot::CellWidth;

#[test]
fn a_snapshot_installs_at_the_geometry_it_was_sent_with() {
    let mut replica = Replica::new(PROMISED_GEOMETRY);

    assert_eq!(replica.apply(&geometry(0, 0, 10, 40)), Applied::default());
    assert_eq!(replica.apply(&snapshot(0, 0, "hello")), redraw());

    assert_eq!(replica.geometry(), Some(Geometry { rows: 10, cols: 40 }));
    assert_eq!(replica.epoch(), Epoch(0));
    assert_eq!(first_row(&replica), "hello");
}

/// ADR 0017 D1: under the capability, a snapshot without its geometry is a
/// desync — not installed, and a fresh screen asked for. Once per episode:
/// a daemon that keeps sending bare snapshots does not get a resync per
/// snapshot forever (spike grill Q3's bounded loop).
#[test]
fn a_snapshot_without_its_geometry_is_a_desync_that_asks_once() {
    let mut replica = Replica::new(PROMISED_GEOMETRY);

    assert_eq!(replica.apply(&snapshot(0, 0, "unsized")), resync());
    assert_eq!(replica.window().err(), Some(Absence::AwaitingSnapshot));

    assert_eq!(replica.apply(&snapshot(0, 1, "still unsized")), redraw());
    assert_eq!(replica.window().err(), Some(Absence::Unavailable));
}

/// ADR 0017 D4: prefix members are one state point with the snapshot they
/// precede. Stamped for another, they are not combined with it.
#[test]
fn prefix_members_stamped_for_another_point_are_not_combined() {
    let mut replica = Replica::new(PROMISED_GEOMETRY);
    replica.apply(&geometry(0, 0, 10, 40));

    assert_eq!(replica.apply(&snapshot(0, 1, "later")), resync());
    assert_eq!(replica.geometry(), None);
}

#[test]
fn a_stale_epochs_prefix_and_snapshot_are_discarded() {
    let mut replica = Replica::new(PROMISED_GEOMETRY);
    replica.apply(&geometry(1, 0, 10, 40));
    replica.apply(&snapshot(1, 0, "current"));

    assert_eq!(replica.apply(&geometry(0, 7, 5, 5)), Applied::default());
    assert_eq!(replica.apply(&snapshot(0, 7, "old")), Applied::default());

    assert_eq!(replica.geometry(), Some(Geometry { rows: 10, cols: 40 }));
    assert_eq!(first_row(&replica), "current");
}

/// ADR 0017 D3: the checkpoint is applied before the snapshot, so colours
/// the snapshot's cells refer to by index resolve as the daemon resolves
/// them.
#[test]
fn the_palette_checkpoint_is_applied_before_the_snapshot() {
    let mut replica = Replica::new(PROMISED_BOTH);
    replica.apply(&geometry(0, 0, 4, 20));
    replica.apply(&palette(0, 0, "\x1b]4;1;#112233\x07"));

    assert_eq!(replica.apply(&snapshot(0, 0, "\x1b[31mred")), redraw());

    let window = replica.window().expect("a screen");
    let entry = window.palette[1];
    assert_eq!((entry.r, entry.g, entry.b), (0x11, 0x22, 0x33));
    assert_eq!(first_row(&replica), "red");
}

#[test]
fn a_palette_stamped_apart_from_its_snapshot_is_not_installed() {
    let mut replica = Replica::new(PROMISED_BOTH);
    replica.apply(&palette(0, 0, "\x1b]4;1;#112233\x07"));
    replica.apply(&geometry(0, 1, 4, 20));

    assert_eq!(replica.apply(&snapshot(0, 1, "x")), resync());
    assert_eq!(replica.geometry(), None);
}

#[test]
fn deltas_apply_to_the_installed_epoch_and_a_newer_one_is_a_missed_prefix() {
    let mut replica = Replica::new(PROMISED_GEOMETRY);
    replica.apply(&geometry(2, 0, 4, 20));
    replica.apply(&snapshot(2, 0, "ab"));

    assert_eq!(replica.apply(&delta(2, 1, "c")), redraw());
    assert_eq!(first_row(&replica), "abc");

    // Older: stale, nothing. Newer: this epoch's prefix never arrived.
    assert_eq!(replica.apply(&delta(1, 9, "old")), Applied::default());
    assert_eq!(first_row(&replica), "abc");
    assert_eq!(replica.apply(&delta(3, 0, "new")), resync_hiding());
}

/// Round 1, #4: one automatic resync per failure episode, re-armed only by
/// a daemon-produced epoch or by opening again — never by an epoch this
/// client's own resize produced.
#[test]
fn a_daemon_produced_epoch_re_arms_recovery_and_a_requested_one_does_not() {
    let mut replica = Replica::new(PROMISED_GEOMETRY);

    // Spend the episode.
    assert_eq!(replica.apply(&snapshot(0, 0, "bare")), resync());
    // The daemon reshapes on its own: a new episode.
    replica.apply(&geometry(1, 0, 4, 20));
    assert_eq!(replica.apply(&snapshot(1, 0, "fresh")), redraw());
    assert_eq!(
        replica.apply(&snapshot(1, 3, "bare again")),
        resync_hiding()
    );

    // Spent again. This client asks for a reshape; the epoch that answers it
    // is its own doing and re-arms nothing.
    replica.requested(Geometry { rows: 6, cols: 30 });
    replica.apply(&geometry(2, 0, 6, 30));
    assert_eq!(replica.apply(&snapshot(2, 0, "resized")), redraw());
    assert_eq!(replica.apply(&snapshot(2, 4, "bare once more")), redraw());
    assert_eq!(replica.window().err(), Some(Absence::Unavailable));
}

/// Round 2, Q13: under a daemon that sends no `Geometry`, nothing is built
/// at a guessed size. The first snapshot waits for a real local grid, which
/// is sent, and a fresh snapshot asked for.
#[test]
fn under_a_legacy_daemon_no_screen_exists_before_a_grid_does() {
    let mut replica = Replica::new(LEGACY);

    assert_eq!(
        replica.apply(&snapshot(0, 0, "for nobody")),
        Applied::default()
    );
    assert_eq!(replica.window().err(), Some(Absence::AwaitingGrid));

    assert_eq!(replica.requested(Geometry { rows: 24, cols: 80 }), resync());
    assert_eq!(replica.apply(&snapshot(0, 1, "sized")), redraw());
    assert_eq!(replica.geometry(), Some(Geometry { rows: 24, cols: 80 }));

    // A later grid is a resize, not a resync.
    assert_eq!(
        replica.requested(Geometry {
            rows: 30,
            cols: 100
        }),
        Applied::default()
    );
}

/// A frame the daemon did not promise says nothing: a legacy replica keeps
/// the size it asked for.
#[test]
fn a_geometry_frame_a_daemon_did_not_promise_is_ignored() {
    let mut replica = Replica::new(LEGACY);
    replica.requested(Geometry { rows: 10, cols: 20 });
    replica.apply(&geometry(0, 0, 50, 50));

    assert_eq!(replica.apply(&snapshot(0, 0, "x")), redraw());
    assert_eq!(replica.geometry(), Some(Geometry { rows: 10, cols: 20 }));
}

#[test]
fn a_channel_error_is_reported_in_the_daemons_words() {
    let mut replica = Replica::new(PROMISED_GEOMETRY);

    let applied = replica.apply(&frame(FrameKind::ChannelError, 0, 0, b"no such run"));

    assert_eq!(applied.refusal.as_deref(), Some("no such run"));
    assert!(!applied.resync);
}

/// The known qwertty-term-vt 0.4.0 defect (docs/evidence/pr3-terminal-fuzz-
/// 2026-08-24.md): a title whose 1024th byte falls inside a multi-byte
/// character panics the parser. The replica is destroyed, never the process;
/// one resync is asked for; the screen it brings is installed; a second
/// failure in the same episode stops automatic retry (spike grill Q3).
#[test]
fn a_parser_panic_destroys_the_replica_and_recovers_once() {
    let mut poison = b"\x1b]2;".to_vec();
    poison.extend(std::iter::repeat_n(b'a', 1022));
    poison.extend("€".as_bytes());
    poison.push(0x07);

    let mut replica = Replica::new(PROMISED_GEOMETRY);
    replica.apply(&geometry(0, 0, 4, 20));
    replica.apply(&snapshot(0, 0, "fine"));

    let applied = replica.apply(&frame(FrameKind::Delta, 0, 1, &poison));
    assert_eq!(
        applied,
        Applied {
            redraw: true,
            resync: true,
            refusal: None
        }
    );
    assert_eq!(replica.window().err(), Some(Absence::Unavailable));

    // The resync's screen, same epoch.
    replica.apply(&geometry(0, 2, 4, 20));
    assert_eq!(replica.apply(&snapshot(0, 2, "rebuilt")), redraw());
    assert_eq!(first_row(&replica), "rebuilt");

    // Poisoned again in the same episode: no third attempt.
    let applied = replica.apply(&frame(FrameKind::Delta, 0, 3, &poison));
    assert_eq!(applied, redraw());
    assert_eq!(replica.window().err(), Some(Absence::Unavailable));
}

#[test]
fn the_modes_keys_are_encoded_under_come_from_the_replica() {
    let mut replica = Replica::new(PROMISED_GEOMETRY);
    assert_eq!(replica.modes(), Modes::default());

    replica.apply(&geometry(0, 0, 4, 20));
    replica.apply(&snapshot(0, 0, "\x1b[?1h\x1b[?2004h"));

    assert_eq!(
        replica.modes(),
        Modes {
            cursor_keys: true,
            bracketed_paste: true
        }
    );
}

#[test]
fn geometry_round_trips_through_the_four_wire_bytes() {
    let geometry = Geometry {
        rows: 0x0102,
        cols: 0x0304,
    };
    assert_eq!(geometry.encode(), vec![1, 2, 3, 4]);
    assert_eq!(Geometry::decode(&[1, 2, 3, 4]), Some(geometry));
    assert_eq!(Geometry::decode(&[1, 2, 3]), None);
}

/// ADR 0017 D3: a checkpoint is omitted when the connection already holds
/// it, so a screen rebuilt later on the same connection keeps the palette
/// this connection last received. `None` means unchanged, not default.
#[test]
fn a_rebuilt_screen_keeps_the_palette_the_connection_already_received() {
    let mut replica = Replica::new(PROMISED_BOTH);
    replica.apply(&geometry(0, 0, 4, 20));
    replica.apply(&palette(0, 0, "\x1b]4;1;#112233\x07"));
    replica.apply(&snapshot(0, 0, "red"));

    // Another viewer resized: a new epoch's prefix, without a `Palette`
    // because this connection already holds the checkpoint.
    replica.apply(&geometry(1, 0, 4, 30));
    assert_eq!(replica.apply(&snapshot(1, 0, "still red")), redraw());

    let entry = replica.window().expect("a screen").palette[1];
    assert_eq!((entry.r, entry.g, entry.b), (0x11, 0x22, 0x33));
}

/// A desync means the installed screen belongs to an epoch the daemon has
/// left. It is not shown as current while the fresh one is on its way.
#[test]
fn a_desync_hides_the_screen_it_no_longer_trusts_until_the_fresh_one_arrives() {
    let mut replica = Replica::new(PROMISED_GEOMETRY);
    replica.apply(&geometry(2, 0, 4, 20));
    replica.apply(&snapshot(2, 0, "ab"));

    let applied = replica.apply(&delta(3, 0, "new"));
    assert!(applied.resync);
    assert!(applied.redraw);
    assert_eq!(replica.window().err(), Some(Absence::AwaitingSnapshot));

    // The fresh screen, when it comes, is the one on display.
    replica.apply(&geometry(3, 0, 4, 20));
    assert_eq!(replica.apply(&snapshot(3, 0, "new")), redraw());
    assert_eq!(first_row(&replica), "new");
}
