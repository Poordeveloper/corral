use serde_json::json;

use super::*;

fn delivery(payload: &[u8]) -> HookDelivery {
    HookDelivery::new(
        "0123456789abcdef0123456789abcdef".to_owned(),
        "claude".to_owned(),
        "0.0.0".to_owned(),
        payload,
    )
    .expect("a carryable payload")
}

#[test]
fn a_payload_travels_verbatim() {
    let raw = br#"{"hook_event_name":"Stop","session_id":"abc"}"#;
    let carried = delivery(raw);
    assert_eq!(
        carried.payload.as_deref(),
        Some(std::str::from_utf8(raw).expect("utf-8"))
    );
    assert_eq!(carried.payload_omitted, None);
    assert_eq!(carried.hook_protocol_version, HOOK_PROTOCOL_VERSION);
}

/// Dropped whole and marked, never truncated: a truncated fact would be a
/// fabricated one, and a systematic oversize must be visible rather than
/// silently missing (ADR 0004 D3).
#[test]
fn an_oversize_payload_is_dropped_whole_and_marked() {
    let carried = delivery(&vec![b'x'; MAX_HOOK_PAYLOAD_BYTES + 1]);
    assert_eq!(carried.payload, None);
    assert_eq!(
        carried.payload_omitted.as_deref(),
        Some(PAYLOAD_OMITTED_OVERSIZE)
    );
}

#[test]
fn a_payload_exactly_at_the_cap_is_carried() {
    let carried = delivery(&vec![b'x'; MAX_HOOK_PAYLOAD_BYTES]);
    assert_eq!(
        carried.payload.map(|payload| payload.len()),
        Some(MAX_HOOK_PAYLOAD_BYTES)
    );
}

/// A byte sequence this channel cannot carry is a definite error the relay
/// fails open on — never an oversize marker, which would state a reason that
/// is not the reason.
#[test]
fn a_payload_that_is_not_text_is_not_reported_as_oversize() {
    assert!(
        HookDelivery::new(
            "t".to_owned(),
            "claude".to_owned(),
            "0.0.0".to_owned(),
            &[0xff, 0xfe]
        )
        .is_none()
    );
}

/// Future input: an envelope gaining fields must stay decodable by a build
/// that has no word for them (`AGENTS.md` §Protocol).
#[test]
fn unknown_envelope_fields_are_ignored() {
    let wire = json!({
        "hook_protocol_version": 1,
        "launch_token": "abc",
        "provider": "claude",
        "shim_version": "9.9.9",
        "payload": "{}",
        "a_field_from_later": {"nested": true},
    });
    let decoded: HookDelivery = serde_json::from_value(wire).expect("decodable");
    assert_eq!(decoded.launch_token, "abc");
    assert_eq!(decoded.payload.as_deref(), Some("{}"));
}

/// Absence means the payload is what it says it is, never that one was dropped
/// for a reason this build had no word for.
#[test]
fn an_absent_payload_marker_is_not_a_dropped_payload() {
    let wire = json!({
        "hook_protocol_version": 1,
        "launch_token": "abc",
        "provider": "claude",
        "shim_version": "0.0.0",
        "payload": "{}",
    });
    let decoded: HookDelivery = serde_json::from_value(wire).expect("decodable");
    assert_eq!(decoded.payload_omitted, None);
}

/// A marker this build does not know is still a payload that is not here.
/// Decoding must not fail over the reason.
#[test]
fn an_unknown_omission_reason_still_decodes() {
    let wire = json!({
        "hook_protocol_version": 1,
        "launch_token": "abc",
        "provider": "claude",
        "shim_version": "0.0.0",
        "payload_omitted": "a-reason-from-later",
    });
    let decoded: HookDelivery = serde_json::from_value(wire).expect("decodable");
    assert_eq!(decoded.payload, None);
    assert_eq!(
        decoded.payload_omitted.as_deref(),
        Some("a-reason-from-later")
    );
}

/// A version is what governs breaking change, so it is carried and compared
/// rather than assumed.
#[test]
fn a_delivery_states_the_contract_it_speaks() {
    let wire = serde_json::to_value(delivery(b"{}")).expect("encodable");
    assert_eq!(wire["hook_protocol_version"], HOOK_PROTOCOL_VERSION);
}

/// Receipt only. An ack that could carry a decision is a channel that can
/// steer the user's agent (ADR 0004 D4).
#[test]
fn the_acknowledgement_carries_nothing() {
    assert_eq!(HookAck::wire_value(), json!({}));
    assert_eq!(
        serde_json::to_value(HookAck::default()).expect("encodable"),
        json!({}),
    );
    // A newer peer's ack with fields still decodes, and still means receipt.
    let _: HookAck = serde_json::from_value(json!({"decision": "block"})).expect("decodable");
}

/// The whole delivery has to fit the framing this channel rides on, cap
/// included, or the largest legitimate payload would be unsendable.
#[test]
fn the_largest_carryable_delivery_fits_one_frame() {
    let carried = delivery(&vec![b'x'; MAX_HOOK_PAYLOAD_BYTES]);
    let params = serde_json::to_value(&carried).expect("encodable");
    let frame = crate::encode_frame(&crate::Frame::request(
        crate::RequestId(0),
        HOOK_DELIVER,
        Some(params),
    ))
    .expect("framable");
    assert!(
        frame.len() < crate::MAX_FRAME_BYTES,
        "{} bytes",
        frame.len()
    );
}
