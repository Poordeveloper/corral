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

/// The field is `compatibility_result`, as ADR 0001 and S3(a) name it. A peer
/// spelling it otherwise states no verdict, and a verdict may not be assumed
/// from an absent field.
#[test]
fn a_server_hello_without_a_verdict_does_not_decode() {
    let error = serde_json::from_str::<ServerHello>(
        r#"{"protocol_version":1,"min_compatible_peer_version":1,"compatibility":"compatible"}"#,
    )
    .expect_err("required field");

    assert!(error.to_string().contains("compatibility_result"));
}

/// A hello with no role means what it always meant: an ordinary semantic
/// connection. An older client's hello cannot become something else because a
/// field was added after it.
#[test]
fn a_hello_without_a_role_still_decodes() {
    let decoded: ClientHello =
        serde_json::from_str(r#"{"protocol_version": 1, "min_compatible_peer_version": 1}"#)
            .expect("decode");

    assert!(decoded.role.is_none());
}

/// A role kind this build does not serve must not make the whole hello
/// malformed: the client stated a version this daemon can compare and asked
/// for something it does not have. Those are different answers.
#[test]
fn a_role_kind_this_build_does_not_know_decodes_as_unknown() {
    let decoded: ClientHello = serde_json::from_str(
        r#"{"protocol_version": 1, "min_compatible_peer_version": 1,
            "role": {"kind": "terminal_control", "some_future_field": 7}}"#,
    )
    .expect("an unknown role kind is not a malformed hello");

    assert_eq!(
        decoded.role,
        Some(ConnectionRole::Unknown {
            kind: "terminal_control".to_owned()
        })
    );
}

/// The terminal-data role is its token. A claim without one is not that role,
/// and inventing an empty token would open a channel on nothing.
#[test]
fn a_terminal_data_role_without_a_token_is_not_that_role() {
    let decoded: ClientHello = serde_json::from_str(
        r#"{"protocol_version": 1, "min_compatible_peer_version": 1,
            "role": {"kind": "terminal_data"}}"#,
    )
    .expect("decode");

    assert_eq!(
        decoded.role,
        Some(ConnectionRole::Unknown {
            kind: "terminal_data".to_owned()
        })
    );
}

/// The published constant and what serde emits are the same string, so a
/// non-Rust peer that trusts the constant is understood.
#[test]
fn the_role_encodes_under_the_name_it_publishes() {
    let role = ConnectionRole::TerminalData {
        attach_token: "abc".to_owned(),
    };

    let encoded = serde_json::to_value(&role).expect("encode");

    assert_eq!(
        encoded.get("kind").and_then(|kind| kind.as_str()),
        Some(ConnectionRole::TERMINAL_DATA)
    );
    assert_eq!(
        serde_json::from_value::<ConnectionRole>(encoded).expect("decode"),
        role
    );
}
