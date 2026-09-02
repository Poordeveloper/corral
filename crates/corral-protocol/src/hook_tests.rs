use serde_json::json;

use super::*;

fn delivery(payload: &[u8]) -> HookDelivery {
    HookDelivery::new(
        Some("0123456789abcdef0123456789abcdef".to_owned()),
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
            Some("t".to_owned()),
            "claude".to_owned(),
            "0.0.0".to_owned(),
            &[0xff, 0xfe]
        )
        .is_none()
    );
}

/// Oversize is decided by length, before validity — and it has to be. A
/// payload read up to the cap ends wherever the cap falls, which for anything
/// but ASCII usually lands mid-character; asking "is this text?" first would
/// turn the ordinary oversize case into a silent drop, exactly where a
/// systematic oversize is supposed to become visible.
#[test]
fn an_oversize_payload_cut_mid_character_is_still_marked() {
    // Every one of these ends with a truncated multi-byte character, which is
    // what a byte-bounded read of real text produces.
    for tail in [&[0xe2_u8][..], &[0xe2, 0x82][..], &[0xf0, 0x9f, 0x92][..]] {
        let mut payload = vec![b'x'; MAX_HOOK_PAYLOAD_BYTES + 1 - tail.len()];
        payload.extend_from_slice(tail);
        assert!(payload.len() > MAX_HOOK_PAYLOAD_BYTES);

        let carried = delivery(&payload);

        assert_eq!(carried.payload, None, "{} bytes", payload.len());
        assert_eq!(
            carried.payload_omitted.as_deref(),
            Some(PAYLOAD_OMITTED_OVERSIZE),
        );
    }
}

/// The cap bounds the payload; the framing limit bounds the message. JSON
/// escaping expands a control character six-fold, so a payload well under the
/// cap can encode past what the channel carries — and the delivery marked
/// instead is what keeps the event from vanishing without a record.
#[test]
fn a_payload_that_escapes_past_the_frame_has_a_marked_form_that_fits() {
    let escaping = vec![0x1b_u8; MAX_HOOK_PAYLOAD_BYTES];
    let carried = delivery(&escaping);
    assert!(carried.payload.is_some(), "it is under the cap");

    let framed = |delivery: &HookDelivery| {
        let params = serde_json::to_value(delivery).expect("encodable");
        crate::encode_frame(&crate::Frame::request(
            crate::RequestId(0),
            HOOK_DELIVER,
            Some(params),
        ))
        .expect("framable")
        .len()
    };

    assert!(
        framed(&carried) > crate::MAX_FRAME_BYTES,
        "the worst case has to be a real one for this to be worth handling",
    );
    let marked = carried.without_payload();
    assert!(framed(&marked) < crate::MAX_FRAME_BYTES);
    assert_eq!(marked.payload, None);
    assert_eq!(
        marked.payload_omitted.as_deref(),
        Some(PAYLOAD_OMITTED_OVERSIZE),
    );
    // Everything a delivery is placed by survives the drop.
    assert_eq!(marked.launch_token, carried.launch_token);
    assert_eq!(marked.provider, carried.provider);
    assert_eq!(marked.hook_protocol_version, carried.hook_protocol_version);
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
    assert_eq!(decoded.launch_token.as_deref(), Some("abc"));
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

// The token-less scope, and the skew it has to survive in both directions
// (ADR 0014 D1).

/// Absence is the global scope, not a token that went missing.
#[test]
fn a_delivery_from_a_globally_installed_entry_carries_no_token() {
    let carried = HookDelivery::new(
        None,
        "claude".to_owned(),
        "0.0.0".to_owned(),
        br#"{"hook_event_name":"Stop"}"#,
    )
    .expect("a carryable payload");

    assert!(carried.launch_token.is_none());
    let wire = serde_json::to_value(&carried).expect("encodable");
    assert!(
        wire.get("launch_token").is_none(),
        "an absent token is absent on the wire, never an empty string",
    );
}

/// A daemon that predates the token-less scope requires the field, so the
/// delivery does not decode there and is dropped with diagnostics. Degraded
/// awareness on a mixed pair, never interference — and the relay exits 0
/// either way.
#[test]
fn an_older_daemon_cannot_decode_a_token_less_delivery() {
    /// The shape `HookDelivery` had before the token became optional.
    #[derive(serde::Deserialize)]
    #[allow(dead_code)]
    struct BeforeGlobalScope {
        hook_protocol_version: u32,
        launch_token: String,
        provider: String,
        shim_version: String,
    }

    let wire = json!({
        "hook_protocol_version": 1,
        "provider": "claude",
        "shim_version": "9.9.9",
        "payload": "{}",
    });

    assert!(serde_json::from_value::<BeforeGlobalScope>(wire.clone()).is_err());
    // The same bytes this build reads as the global scope.
    let decoded: HookDelivery = serde_json::from_value(wire).expect("decodable here");
    assert!(decoded.launch_token.is_none());
}

/// A relay that predates the self-observation fields sends none, and absence
/// means unreported rather than a process with no parent.
#[test]
fn an_older_relays_delivery_reports_no_observation() {
    let wire = json!({
        "hook_protocol_version": 1,
        "launch_token": "abc",
        "provider": "claude",
        "shim_version": "0.0.1",
        "payload": "{}",
    });

    let decoded: HookDelivery = serde_json::from_value(wire).expect("decodable");
    assert_eq!(decoded.relay_pid, None);
    assert_eq!(decoded.relay_parent_pid, None);
}

#[test]
fn where_the_relay_stood_survives_the_wire() {
    let carried = delivery(b"{}").observed_at(4321, 4320);

    let wire = serde_json::to_value(&carried).expect("encodable");
    let decoded: HookDelivery = serde_json::from_value(wire).expect("decodable");

    assert_eq!(decoded.relay_pid, Some(4321));
    assert_eq!(decoded.relay_parent_pid, Some(4320));
}

/// The oversize marker is about the payload and says nothing about scope or
/// about where the relay stood.
#[test]
fn dropping_an_oversize_payload_keeps_the_scope_and_the_observation() {
    let carried = HookDelivery::new(None, "codex".to_owned(), "0.0.0".to_owned(), b"{}")
        .expect("a carryable payload")
        .observed_at(11, 10);

    let marked = carried.without_payload();

    assert!(marked.launch_token.is_none());
    assert_eq!(marked.relay_pid, Some(11));
    assert_eq!(marked.relay_parent_pid, Some(10));
}
