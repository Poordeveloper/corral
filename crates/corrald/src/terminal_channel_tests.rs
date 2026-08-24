use super::*;

#[test]
fn a_geometry_survives_its_wire_form() {
    let geometry = PtyGeometry {
        rows: 40,
        cols: 132,
    };

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

#[test]
fn a_geometry_carries_its_full_range() {
    let geometry = PtyGeometry {
        rows: u16::MAX,
        cols: u16::MAX,
    };

    assert_eq!(
        decode_geometry(&encode_geometry(geometry)),
        Some(geometry),
        "a large geometry did not survive the round trip"
    );
}
