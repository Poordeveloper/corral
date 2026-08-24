use super::*;

#[test]
fn a_geometry_survives_its_wire_form() {
    let geometry = PtyGeometry::expect_valid(40, 132);

    let decoded = decode_geometry(&encode_geometry(geometry)).expect("a well-formed payload");

    assert_eq!(decoded, geometry);
}

/// A short resize payload is ignored rather than guessed at: a geometry
/// invented from missing bytes would reflow a real screen.
#[test]
fn a_truncated_geometry_is_not_a_geometry() {
    assert_eq!(decode_geometry(&[]), None);
    assert_eq!(decode_geometry(&[0, 24, 0]), None);
}

/// Four bytes from an attached client must not be able to ask the daemon for
/// a 65535x65535 active area — billions of cells, allocated in full, and not
/// covered by the scrollback budget.
#[test]
fn a_geometry_past_what_corral_will_build_is_refused() {
    let absurd = [0xFF, 0xFF, 0xFF, 0xFF];

    assert_eq!(decode_geometry(&absurd), None);
}

/// A terminal of zero rows or columns has no cells to hold state in, and the
/// emulator only checks that with a `debug_assert` — a release daemon would
/// dereference a null page.
#[test]
fn an_empty_geometry_is_refused() {
    assert_eq!(decode_geometry(&[0, 0, 0, 0]), None);
    assert_eq!(decode_geometry(&[0, 0, 0, 80]), None);
    assert_eq!(decode_geometry(&[0, 24, 0, 0]), None);
}

#[test]
fn the_largest_geometry_corral_builds_survives_the_round_trip() {
    let largest = PtyGeometry::expect_valid(
        crate::runtime::MAX_TERMINAL_ROWS,
        crate::runtime::MAX_TERMINAL_COLS,
    );

    assert_eq!(decode_geometry(&encode_geometry(largest)), Some(largest));
}
