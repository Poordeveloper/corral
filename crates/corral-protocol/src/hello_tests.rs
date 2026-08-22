use super::*;

fn peer(protocol_version: u32, min_compatible_peer_version: u32) -> PeerVersions {
    PeerVersions {
        protocol_version,
        min_compatible_peer_version,
    }
}

#[test]
fn both_peers_always_reach_the_same_verdict() {
    for a_version in 1..8 {
        for a_min in 1..=a_version {
            for b_version in 1..8 {
                for b_min in 1..=b_version {
                    let a = peer(a_version, a_min);
                    let b = peer(b_version, b_min);
                    assert_eq!(
                        compatible(a, b),
                        compatible(b, a),
                        "asymmetric verdict for {a:?} and {b:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn a_peer_below_the_local_floor_is_incompatible() {
    assert!(!compatible(peer(3, 2), peer(1, 1)));
}

#[test]
fn a_peer_whose_floor_is_above_us_is_incompatible() {
    assert!(!compatible(peer(1, 1), peer(3, 2)));
}

#[test]
fn overlapping_ranges_are_compatible() {
    assert!(compatible(peer(3, 1), peer(1, 1)));
}

#[test]
fn a_hello_without_capabilities_means_the_empty_set() {
    let hello: ClientHello =
        serde_json::from_str(r#"{"protocol_version":1,"min_compatible_peer_version":1}"#)
            .expect("decode");

    assert!(hello.capabilities.is_empty());
}

#[test]
fn a_hello_missing_a_required_version_does_not_decode() {
    let error = serde_json::from_str::<ClientHello>(r#"{"protocol_version":1}"#)
        .expect_err("required field");

    assert!(error.to_string().contains("min_compatible_peer_version"));
}

#[test]
fn unknown_hello_fields_are_ignored() {
    let hello: ClientHello = serde_json::from_str(
        r#"{"protocol_version":1,"min_compatible_peer_version":1,"nickname":"future"}"#,
    )
    .expect("decode");

    assert_eq!(hello.protocol_version, 1);
}
