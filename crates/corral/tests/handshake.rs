//! The bootstrap handshake and the protocol 1 served surface, exercised
//! against a real daemon with hand-built frames.

// The repository allows unwrap/expect in tests; that setting does not reach
// helpers that sit outside a `#[test]` function in an integration-test crate.
#![allow(clippy::expect_used, clippy::unwrap_used)]

mod support;

use serde_json::json;
use support::TestAccount;
use support::wire::{RawClient, error_code};

#[test]
fn a_hello_is_the_only_legal_first_message() {
    let account = TestAccount::new("hello-first");
    let _daemon = account.start_daemon();
    let mut client = RawClient::connect(&account.socket());

    let response = client
        .request(1, "session.list", None)
        .expect("a typed refusal");

    assert_eq!(error_code(&response), Some("protocol_violation"));
    assert!(
        client.receive().is_none(),
        "the connection must close after a bootstrap violation"
    );
}

#[test]
fn a_hello_missing_its_identity_is_malformed_rather_than_old() {
    let account = TestAccount::new("hello-malformed");
    let _daemon = account.start_daemon();
    let mut client = RawClient::connect(&account.socket());

    let response = client
        .request(1, "hello", Some(json!({"protocol_version": 1})))
        .expect("a typed refusal");

    assert_eq!(error_code(&response), Some("malformed_hello"));
    assert!(client.receive().is_none());
}

#[test]
fn an_incompatible_peer_is_told_both_sides_and_closed() {
    let account = TestAccount::new("hello-incompatible");
    let _daemon = account.start_daemon();
    let mut client = RawClient::connect(&account.socket());

    let response = client.say_hello(99, 99);

    let hello = &response["outcome"]["result"];
    assert_eq!(hello["compatibility_result"], "incompatible");
    assert_eq!(hello["protocol_version"], 1);
    assert_eq!(hello["min_compatible_peer_version"], 1);
    assert!(
        client.receive().is_none(),
        "an incompatible peer never establishes"
    );
}

#[test]
fn a_repeated_hello_is_a_protocol_violation() {
    let account = TestAccount::new("hello-repeated");
    let _daemon = account.start_daemon();
    let mut client = RawClient::connect(&account.socket());
    client.establish();

    let response = client.say_hello(1, 1);

    assert_eq!(error_code(&response), Some("protocol_violation"));
    assert!(client.receive().is_none());
}

#[test]
fn the_negotiated_capability_set_is_empty() {
    let account = TestAccount::new("hello-capabilities");
    let _daemon = account.start_daemon();
    let mut client = RawClient::connect(&account.socket());

    let response = client.establish();

    assert_eq!(response["outcome"]["result"]["capabilities"], json!([]));
}

#[test]
fn an_unknown_method_leaves_the_connection_usable() {
    let account = TestAccount::new("unknown-method");
    let _daemon = account.start_daemon();
    let mut client = RawClient::connect(&account.socket());
    client.establish();

    let refused = client
        .request(1, "session.attach", None)
        .expect("a typed refusal");
    let served = client.request(2, "ping", None).expect("still usable");

    assert_eq!(error_code(&refused), Some("method_not_found"));
    assert_eq!(served["outcome"]["result"], json!({}));
}

#[test]
fn parameters_a_baseline_method_cannot_honour_are_refused_not_ignored() {
    let account = TestAccount::new("invalid-params");
    let _daemon = account.start_daemon();
    let mut client = RawClient::connect(&account.socket());
    client.establish();

    let refused = client
        .request(1, "session.list", Some(json!({"workspace": "corral"})))
        .expect("a typed refusal");
    let served = client
        .request(2, "session.list", None)
        .expect("still usable");

    assert_eq!(error_code(&refused), Some("invalid_params"));
    assert_eq!(served["outcome"]["result"], json!({"sessions": []}));
}

#[test]
fn an_unknown_notification_is_ignored() {
    let account = TestAccount::new("unknown-notification");
    let _daemon = account.start_daemon();
    let mut client = RawClient::connect(&account.socket());
    client.establish();

    client.send(&json!({"type": "notification", "method": "session.watch"}));
    let served = client.request(1, "ping", None).expect("still usable");

    assert_eq!(served["outcome"]["result"], json!({}));
}

#[test]
fn unknown_additive_fields_are_tolerated() {
    let account = TestAccount::new("unknown-fields");
    let _daemon = account.start_daemon();
    let mut client = RawClient::connect(&account.socket());
    client.send(&json!({
        "type": "request",
        "id": 0,
        "method": "hello",
        "trace": "from-a-newer-client",
        "params": {
            "protocol_version": 1,
            "min_compatible_peer_version": 1,
            "nickname": "future",
        },
    }));

    let response = client.receive().expect("the daemon answered");

    assert_eq!(
        response["outcome"]["result"]["compatibility_result"],
        "compatible"
    );
}

/// Protocol 1 daemons answer; they never originate. A response frame therefore
/// cannot be an answer to anything, and the connection is not trustworthy.
#[test]
fn an_unsolicited_response_closes_the_connection() {
    let account = TestAccount::new("unsolicited-response");
    let _daemon = account.start_daemon();
    let mut client = RawClient::connect(&account.socket());
    client.establish();

    client.send(&json!({"type": "response", "id": 7, "outcome": {"result": {}}}));

    assert!(client.receive().is_none());
}

#[test]
fn a_frame_that_is_not_an_envelope_closes_the_connection() {
    let account = TestAccount::new("undecodable");
    let _daemon = account.start_daemon();
    let mut client = RawClient::connect(&account.socket());

    client.send_raw(b"{not json at all}\n");

    assert!(client.receive().is_none());
}

/// An unknown frame kind is a contract this version never negotiated. Ignoring
/// it would present as a hang to whoever expected an answer.
#[test]
fn an_unknown_frame_kind_closes_the_connection() {
    let account = TestAccount::new("unknown-kind");
    let _daemon = account.start_daemon();
    let mut client = RawClient::connect(&account.socket());

    client.send(&json!({"type": "subscribe", "id": 1, "topic": "sessions"}));

    assert!(client.receive().is_none());
}

/// A pending connection has a bounded life of its own; the daemon closes it at
/// the deadline rather than holding a transport peer forever.
#[test]
fn a_pending_connection_is_closed_at_the_deadline() {
    let account = TestAccount::new("pre-hello")
        .with_pre_hello_deadline(std::time::Duration::from_millis(300))
        .with_idle_grace(std::time::Duration::from_secs(30));
    let _daemon = account.start_daemon();
    let mut client = RawClient::connect(&account.socket());

    // Say nothing at all, and wait.
    assert!(
        client.receive().is_none(),
        "the daemon must close a connection that never says hello"
    );
}

/// The frame-size limit is a safety limit, and reaching it means the byte
/// stream is no longer trustworthy: no typed reply, just a close.
#[test]
fn an_oversize_frame_closes_the_connection() {
    let account = TestAccount::new("oversize");
    let _daemon = account.start_daemon();
    let mut client = RawClient::connect(&account.socket());

    let oversize = vec![b'x'; corral_protocol::MAX_FRAME_BYTES + 4096];

    // The daemon stops reading the moment its buffer crosses the limit, so
    // the tail of this write may find a closed peer. Either way the answer
    // under test is the same: the connection is gone.
    if client.send_raw_tolerating_close(&oversize) {
        assert!(client.receive().is_none());
    }
}
