use super::*;

/// A daemon that refused answered. Saying it could not be reached would claim
/// something about one that is demonstrably there — an older daemon that does
/// not implement `session.list` is exactly this (`AGENTS.md` §Protocol,
/// §Runtime truth).
///
/// Three claims, and only one of them says nothing is there.
#[test]
fn what_a_failed_request_says_about_the_daemon_behind_it() {
    let endpoint = std::path::PathBuf::from("/nowhere");
    let cases = [
        (
            about(&RequestError::Refused(corral_protocol::ProtocolError {
                code: corral_protocol::ErrorCode::MethodNotFound,
                message: "no such method".to_owned(),
            })),
            "would not list",
        ),
        (
            about(&RequestError::Protocol {
                detail: "a response for request 2 arrived while 3 was outstanding".to_owned(),
            }),
            "cannot read",
        ),
        (
            about(&RequestError::DaemonConnectionLost { endpoint }),
            "did not answer",
        ),
    ];

    for (unanswered, expected) in cases {
        let said = unanswered.line();

        assert!(said.contains(expected), "{said}");
    }
}

/// A refusal leaves a connection that can be asked again; the other two do
/// not — one has nobody on it, and the other is at a place this client cannot
/// find.
#[test]
fn only_a_refusal_leaves_a_connection_worth_keeping() {
    let refused = RequestError::Refused(corral_protocol::ProtocolError {
        code: corral_protocol::ErrorCode::MethodNotFound,
        message: "no such method".to_owned(),
    });

    assert!(matches!(about(&refused), Unanswered::Refused(_)));
    assert!(matches!(
        about(&RequestError::Protocol {
            detail: "nonsense".to_owned()
        }),
        Unanswered::Unreadable(_)
    ));
    assert!(matches!(
        about(&RequestError::DaemonConnectionLost {
            endpoint: std::path::PathBuf::from("/nowhere")
        }),
        Unanswered::Silent(_)
    ));
}

/// A corrald that dies on startup leaves no owner behind, so a poll that
/// activated every second would start one every second. The wait between
/// attempts grows, and stops growing.
#[test]
fn activation_waits_longer_each_time_it_fails_and_stops_at_a_ceiling() {
    let first = Backoff::after(0);
    assert_eq!(first.failures, 1);
    assert!(
        first
            .waiting()
            .is_some_and(|waiting| waiting <= Duration::from_secs(1))
    );

    let later = Backoff::after(20);
    assert!(
        later
            .waiting()
            .is_some_and(|waiting| waiting <= Backoff::CEILING),
        "the wait grew past its ceiling"
    );
}
