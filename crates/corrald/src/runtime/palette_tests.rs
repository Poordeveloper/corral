use super::*;
use crate::runtime::{AuthoritativeTerminal, PtyGeometry};

const GEOMETRY: PtyGeometry = PtyGeometry::expect_valid(24, 80);

fn terminal_after(bytes: &[u8]) -> AuthoritativeTerminal {
    let mut terminal = AuthoritativeTerminal::new(GEOMETRY);
    let _ = terminal.consume(bytes);
    terminal
}

fn checkpoint(terminal: &AuthoritativeTerminal) -> PaletteCheckpoint {
    PaletteCheckpoint::of(terminal.terminal().expect("not poisoned"))
}

const CUSTOM_A: &[u8] = b"\x1b]4;1;rgb:12/34/56\x07\x1b]10;rgb:aa/bb/cc\x07";
const CUSTOM_B: &[u8] = b"\x1b]4;1;rgb:65/43/21\x07\x1b]4;7;rgb:01/02/03\x07";
const RESET: &[u8] = b"\x1b]104\x07\x1b]110\x07\x1b]111\x07";

#[test]
fn a_session_that_never_touched_its_palette_needs_no_checkpoint() {
    let current = checkpoint(&terminal_after(b"plain text"));

    assert_eq!(current, PaletteCheckpoint::BASELINE);
    assert!(current.frame_from(&PaletteCheckpoint::BASELINE).is_none());
}

#[test]
fn a_custom_palette_is_a_checkpoint_a_fresh_replica_reaches() {
    let daemon = checkpoint(&terminal_after(CUSTOM_A));
    let frame = daemon
        .frame_from(&PaletteCheckpoint::BASELINE)
        .expect("a custom palette differs from the baseline");

    let replica = checkpoint(&terminal_after(&frame));

    assert_eq!(replica, daemon);
    assert_ne!(daemon, PaletteCheckpoint::BASELINE);
}

#[test]
fn a_checkpoint_already_held_is_not_sent_again() {
    let daemon = checkpoint(&terminal_after(CUSTOM_A));

    assert!(daemon.frame_from(&daemon).is_none());
}

#[test]
fn one_custom_palette_to_another_is_a_checkpoint() {
    let held = checkpoint(&terminal_after(CUSTOM_A));
    let daemon = checkpoint(&terminal_after(CUSTOM_B));
    let frame = daemon.frame_from(&held).expect("the palettes differ");

    // The replica held A and is brought to B, entry 1 and entry 7 included,
    // with A's dynamic foreground gone.
    let mut replica = terminal_after(CUSTOM_A);
    let _ = replica.consume(&frame);

    assert_eq!(checkpoint(&replica), daemon);
}

/// The case the grill closed (2026-09-05-adr-0017-grill.md Q3): a replica
/// that once received a custom palette, then missed the reset delta, is
/// brought back to the default by an explicit checkpoint — never left with
/// stale colours because "the palette is default now".
#[test]
fn a_return_to_the_default_is_an_explicit_checkpoint() {
    let mut daemon_terminal = terminal_after(CUSTOM_A);
    let _ = daemon_terminal.consume(RESET);
    let daemon = checkpoint(&daemon_terminal);
    assert_eq!(
        daemon,
        PaletteCheckpoint::BASELINE,
        "the reset returned the daemon to the default"
    );

    // The replica never saw the reset.
    let mut replica = terminal_after(CUSTOM_A);
    let held = checkpoint(&replica);
    let frame = daemon
        .frame_from(&held)
        .expect("a default palette differs from the custom one the connection holds");
    let _ = replica.consume(&frame);

    assert_eq!(checkpoint(&replica), PaletteCheckpoint::BASELINE);
}
