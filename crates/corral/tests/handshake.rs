//! The bootstrap handshake and the served method surface, exercised
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

/// A connection that ends gives its slot back.
///
/// The accept loop serves a bounded number of connections at once, and the
/// bound is held by a permit the serving task owns. A permit that outlived its
/// connection would not fail anything at first: the daemon would answer
/// normally until it had accepted the bound, and then stop accepting forever,
/// with no error anywhere. So what is asserted is that many connections'
/// worth of slots come back.
#[test]
fn slots_come_back_when_connections_end() {
    let account = TestAccount::new("accept-slots");
    let _daemon = account.start_daemon();

    // Past the daemon's bound, sequentially, each closed before the next
    // opens. The number is written out rather than read from
    // `corrald::policy::CONCURRENT_CONNECTIONS`: a client crate depending on
    // the daemon would hand every surface a path to `corral-core`, which is
    // the one edge `check-dependency-direction` exists to refuse. The
    // constant's own doc names this test, so the two move together.
    for _ in 0..200 {
        let mut client = RawClient::connect(&account.socket());
        client.establish();
    }

    let mut client = RawClient::connect(&account.socket());
    client.establish();
    let answered = client.request(1, "session.list", None);

    assert!(
        answered.is_some(),
        "the daemon stopped accepting after its own bound",
    );
}

#[test]
fn a_hello_missing_its_identity_is_malformed_rather_than_old() {
    let account = TestAccount::new("hello-malformed");
    let _daemon = account.start_daemon();
    let mut client = RawClient::connect(&account.socket());

    let response = client
        .request(
            1,
            "hello",
            Some(json!({"protocol_version": corral_protocol::PROTOCOL_VERSION})),
        )
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
    assert_eq!(hello["protocol_version"], corral_protocol::PROTOCOL_VERSION);
    assert_eq!(
        hello["min_compatible_peer_version"],
        corral_protocol::MIN_COMPATIBLE_PEER_VERSION
    );
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

    let response = client.say_hello(
        corral_protocol::PROTOCOL_VERSION,
        corral_protocol::MIN_COMPATIBLE_PEER_VERSION,
    );

    assert_eq!(error_code(&response), Some("protocol_violation"));
    assert!(client.receive().is_none());
}

/// The daemon names the feature contracts it serves, so a client can ask
/// before it offers a person an action this daemon may be too old for.
///
/// Additive methods and fields say nothing through the protocol version, which
/// is the whole reason the field exists — an unadvertised contract would leave
/// a new client reporting `method_not_found` as though the person had asked
/// for something wrong.
#[test]
fn the_daemon_advertises_the_contracts_it_serves() {
    let account = TestAccount::new("hello-capabilities");
    let _daemon = account.start_daemon();
    let mut client = RawClient::connect(&account.socket());

    let response = client.establish();

    assert_eq!(
        response["outcome"]["result"]["capabilities"],
        json!(["managed-sessions"]),
    );
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
            "protocol_version": corral_protocol::PROTOCOL_VERSION,
            "min_compatible_peer_version": corral_protocol::MIN_COMPATIBLE_PEER_VERSION,
            "nickname": "future",
        },
    }));

    let response = client.receive().expect("the daemon answered");

    assert_eq!(
        response["outcome"]["result"]["compatibility_result"],
        "compatible"
    );
}

/// A daemon answers; it never originates. A response frame therefore
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

/// The reason protocol 2 exists.
///
/// A peer built before `session.new` required a `command_id` still declares
/// the older version. It must be refused where Corral says what it can talk
/// to — in the hello, with both version pairs stated — and never reach a
/// request, where the same disagreement would surface as a decoder error after
/// the handshake had already told it everything was fine
/// (`docs/decisions/2026-08-25-protocol-2-acceptance.md`).
#[test]
fn a_protocol_1_peer_is_refused_in_the_handshake_rather_than_at_its_first_request() {
    let account = TestAccount::new("hello-protocol-1");
    let _daemon = account.start_daemon();
    let mut client = RawClient::connect(&account.socket());

    let response = client.say_hello(1, 1);

    let hello = &response["outcome"]["result"];
    assert_eq!(
        hello["compatibility_result"], "incompatible",
        "a peer that cannot be served is told so by the hello: {response}"
    );
    assert_eq!(hello["protocol_version"], corral_protocol::PROTOCOL_VERSION);
    assert_eq!(
        hello["min_compatible_peer_version"],
        corral_protocol::MIN_COMPATIBLE_PEER_VERSION,
        "the client is told what it would have to speak"
    );
    // A verdict, not a refusal of the frame: a decoder error here would mean
    // the handshake had let it through and something further down caught it.
    assert_eq!(error_code(&response), None, "{response}");
    assert!(
        client.receive().is_none(),
        "an incompatible peer never establishes, so no request follows"
    );
}
