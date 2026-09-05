use super::*;
use crate::runtime::spawn::PtyGeometry;

const GEOMETRY: PtyGeometry = PtyGeometry::expect_valid(24, 80);

fn terminal_with(lines: usize) -> AuthoritativeTerminal {
    let mut terminal = AuthoritativeTerminal::new(GEOMETRY);
    for line in 0..lines {
        let _ = terminal.consume(format!("line {line}\r\n").as_bytes());
    }
    terminal
}

#[test]
fn a_snapshot_of_a_fresh_terminal_carries_no_history_and_says_so() {
    let mut terminal = AuthoritativeTerminal::new(GEOMETRY);

    let snapshot = encode(&mut terminal).expect("a fresh screen encodes");

    assert_eq!(snapshot.included_scrollback_rows(), 0);
    assert!(!snapshot.history_truncated_before());
}

/// Everything retained fits well under the target here, so the snapshot
/// carries all of it and truthfully reports nothing was left out.
#[test]
fn a_snapshot_carries_the_history_it_has_when_it_fits() {
    let mut terminal = terminal_with(100);

    let snapshot = encode(&mut terminal).expect("the screen encodes");

    assert!(
        snapshot.included_scrollback_rows() > 0,
        "100 lines through a 24-row screen produced no history"
    );
    assert!(!snapshot.history_truncated_before());
    assert!(snapshot.encoded_bytes() <= SNAPSHOT_TARGET_BYTES);
}

/// The row count is an experience target, not a promise to ship everything:
/// beyond it the snapshot stops and says history existed before what it
/// carries (ADR 0003 D6, D7).
#[test]
fn history_beyond_the_row_target_is_omitted_and_declared() {
    let mut terminal = terminal_with(SNAPSHOT_SCROLLBACK_ROWS + 500);

    let snapshot = encode(&mut terminal).expect("the screen encodes");

    assert_eq!(
        snapshot.included_scrollback_rows(),
        SNAPSHOT_SCROLLBACK_ROWS
    );
    assert!(
        snapshot.history_truncated_before(),
        "history was dropped and the snapshot did not admit it"
    );
}

/// A client replaying the snapshot must arrive at the screen the daemon holds,
/// and the title is part of that screen — the one field the emulator tracks
/// but its serializer omits (ADR 0003 D3).
#[test]
fn the_title_the_serializer_omits_is_emitted_by_corral() {
    let mut terminal = AuthoritativeTerminal::new(GEOMETRY);
    let _ = terminal.consume(b"\x1b]2;deploying\x07working\r\n");

    let snapshot = encode(&mut terminal).expect("the screen encodes");

    let payload = snapshot.payload();
    assert!(
        payload
            .windows(b"\x1b]2;deploying\x07".len())
            .any(|window| window == b"\x1b]2;deploying\x07"),
        "the snapshot does not set the title the daemon holds"
    );
}

/// Resync is the recovery path, so an unchanging 5 KB palette would be paid
/// again at precisely the worst moment. It rides the subscription instead
/// (ADR 0003 D4).
#[test]
fn a_snapshot_does_not_carry_the_palette() {
    let mut terminal = terminal_with(10);

    let snapshot = encode(&mut terminal).expect("the screen encodes");

    assert!(
        !snapshot
            .payload()
            .windows(4)
            .any(|window| window == b"\x1b]4;"),
        "the snapshot carries palette entries that belong to the subscription"
    );
}

/// The approved representative extreme (grill Q8): a very large, heavily
/// styled viewport. Evidence that a realistic extreme sits far below the
/// ceiling — not a proof that every legitimate viewport does.
#[test]
fn an_approved_large_geometry_extreme_stays_far_below_the_ceiling() {
    let geometry = PtyGeometry::expect_valid(140, 500);
    let mut terminal = AuthoritativeTerminal::new(geometry);

    for row in 0..geometry.rows() {
        for col in 0..geometry.cols() {
            let red = (col % 256) as u8;
            let green = (row % 256) as u8;
            let blue = ((col + row) % 256) as u8;
            let _ = terminal.consume(
                format!(
                    "\x1b[38;2;{red};{green};{blue}m\x1b[48;2;{blue};{red};{green}m\x1b[1m{}",
                    if col % 7 == 0 { '漢' } else { 'W' }
                )
                .as_bytes(),
            );
        }
        let _ = terminal.consume(b"\r\n");
    }

    let snapshot = encode(&mut terminal).expect("a legal extreme still encodes");

    // Measured at 1,578,123 bytes when this landed — a tenth of the ceiling,
    // and half what the ADR's sizing rationale estimated. The assertion is
    // deliberately loose: it guards the headroom, not the exact figure, so an
    // encoder change that costs a few percent is not a failing test while one
    // that costs an order of magnitude is.
    let half_the_ceiling = SNAPSHOT_CEILING_BYTES / 2;
    assert!(
        snapshot.encoded_bytes() < half_the_ceiling,
        "the approved extreme encoded to {} bytes, past half the {SNAPSHOT_CEILING_BYTES}-byte ceiling",
        snapshot.encoded_bytes()
    );
}

/// The ceiling's own job, tested separately from the healthy extreme: a
/// viewport that cannot fit is refused with a typed failure. No partial
/// viewport is ever shipped as a successful snapshot (ADR 0003 D8).
///
/// The budget is shrunk rather than the screen grown: what is under test is
/// the algorithm's refusal, and minting a genuinely 16 MiB viewport would
/// spend seconds and hundreds of megabytes to exercise the same branch.
#[test]
fn a_viewport_past_the_ceiling_is_refused_rather_than_truncated() {
    let mut terminal = terminal_with(50);
    let viewport_only = encode_within(&mut terminal, SnapshotBudget::of(0, usize::MAX))
        .expect("the viewport alone encodes");
    let impossible = SnapshotBudget::of(0, viewport_only.encoded_bytes() - 1);

    let error = encode_within(&mut terminal, impossible).expect_err("the viewport cannot fit");

    assert!(matches!(
        error,
        SnapshotError::ViewportExceedsCeiling { .. }
    ));
    assert!(
        error.to_string().contains("past the"),
        "the failure does not say what it exceeded: {error}"
    );
}

/// Over the target, the oldest scrollback goes first and the viewport stays
/// whole — the degradation order ADR 0003 D8 fixes.
#[test]
fn the_oldest_scrollback_is_sacrificed_before_the_viewport() {
    let mut terminal = terminal_with(500);
    let generous = encode(&mut terminal).expect("the screen encodes");
    assert!(generous.included_scrollback_rows() > 0);

    let viewport_only = encode_within(&mut terminal, SnapshotBudget::of(0, usize::MAX))
        .expect("the viewport alone encodes");
    // A budget that cannot hold the full history but comfortably holds the
    // viewport: trimming must land between the two, never refuse.
    let tight = SnapshotBudget::of(viewport_only.encoded_bytes() + 64, usize::MAX);

    let trimmed = encode_within(&mut terminal, tight).expect("trimming succeeds");

    assert!(
        trimmed.included_scrollback_rows() < generous.included_scrollback_rows(),
        "a tighter budget carried as much history as a generous one"
    );
    assert!(
        trimmed.history_truncated_before(),
        "history was trimmed and the snapshot did not admit it"
    );
    assert!(
        trimmed.encoded_bytes() >= viewport_only.encoded_bytes(),
        "the viewport itself was traded away to meet the target"
    );
}

/// A refusal names the ceiling that refused it, not whichever constant the
/// module happens to hold: the budget is a parameter, and a message quoting
/// the default would state a limit the encoder did not apply.
#[test]
fn a_ceiling_refusal_names_the_ceiling_that_applied() {
    let mut terminal = AuthoritativeTerminal::new(PtyGeometry::expect_valid(24, 80));
    let _ = terminal.consume(b"a screen with something on it\r\n");
    let tiny = SnapshotBudget::of(1, 8);

    let refusal = encode_within(&mut terminal, tiny).expect_err("a viewport past the ceiling");

    assert!(
        refusal.to_string().contains("8-byte ceiling"),
        "the refusal named a limit it did not apply: {refusal}"
    );
}
