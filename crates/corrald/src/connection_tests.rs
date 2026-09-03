use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use super::*;
use corral_protocol::Outcome;
use serde_json::json;

static COUNTER: AtomicU32 = AtomicU32::new(0);

/// A registry store on a real file, because the dispatch paths under test read
/// one and a stand-in would only prove the stand-in.
struct Registry {
    state: Arc<DaemonState>,
    directory: PathBuf,
}

impl Registry {
    fn new(name: &str) -> Self {
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let directory =
            std::env::temp_dir().join(format!("corrald-{}-{unique}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).expect("create the scratch directory");
        Self {
            state: Arc::new(
                DaemonState::open(
                    &directory.join("registry.sqlite3"),
                    &directory.join("launch"),
                    &directory,
                )
                .expect("open"),
            ),
            directory,
        }
    }
}

impl Drop for Registry {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}

fn request(method: &str, params: Option<serde_json::Value>) -> Request {
    Request {
        id: RequestId(9),
        method: method.to_owned(),
        params,
    }
}

fn error_code(dispatch: Dispatch) -> (ErrorCode, bool) {
    let (frame, close) = match dispatch {
        Dispatch::Reply(frame) => (frame, false),
        Dispatch::ReplyThenClose(frame) => (frame, true),
        Dispatch::FailClosed(error) => panic!("expected an answer, got {error}"),
    };
    match frame {
        Frame::Response(response) => match response.outcome {
            Outcome::Error(error) => (error.code, close),
            Outcome::Result(value) => panic!("expected an error, got {value}"),
        },
        other => panic!("expected a response, got {other:?}"),
    }
}

#[tokio::test]
async fn an_unknown_method_leaves_the_connection_usable() {
    let registry = Registry::new("unknown-method");

    let (code, close) =
        error_code(dispatch(&request("session.attach", None), &registry.state).await);

    assert_eq!(code, ErrorCode::MethodNotFound);
    assert!(!close);
}

#[tokio::test]
async fn a_repeated_hello_is_a_protocol_violation() {
    let registry = Registry::new("repeated-hello");

    let (code, close) = error_code(dispatch(&request(method::HELLO, None), &registry.state).await);

    assert_eq!(code, ErrorCode::ProtocolViolation);
    assert!(close, "the bootstrap transition happens once");
}

#[tokio::test]
async fn parameters_a_baseline_method_cannot_honour_are_refused() {
    let registry = Registry::new("bad-params");

    let (code, close) = error_code(
        dispatch(
            &request(method::SESSION_LIST, Some(json!({"workspace": "corral"}))),
            &registry.state,
        )
        .await,
    );

    assert_eq!(code, ErrorCode::InvalidParams);
    assert!(!close);
}

#[tokio::test]
async fn the_session_list_is_empty_and_says_so() {
    let registry = Registry::new("session-list");

    let dispatched = dispatch(&request(method::SESSION_LIST, None), &registry.state).await;

    let Dispatch::Reply(Frame::Response(response)) = dispatched else {
        panic!("expected a plain reply");
    };
    match response.outcome {
        Outcome::Result(value) => assert_eq!(value, json!({"sessions": []})),
        Outcome::Error(error) => panic!("expected a result, got {error}"),
    }
}

/// A runtime outside Corral is one row, whatever is known about it. Seen by
/// the sweep alone it is listed under a minted identity with no provider
/// identity — absent is unknown. Once discovery names the Session it is
/// carrying, the same row is listed under that Session with its identity,
/// and nothing is listed under the minted one any more.
#[tokio::test]
async fn an_external_runtime_is_listed_under_the_session_discovery_names() {
    let registry = Registry::new("external-row");
    let process = crate::platform::process::ProcessIdentity {
        pid: 4321,
        parent: 1,
        group: 4321,
        started: std::time::UNIX_EPOCH + std::time::Duration::from_secs(500),
        executable: PathBuf::from("/usr/local/bin/claude"),
    };
    let provisional = crate::sweep::RuntimeCandidate::recognized(
        crate::provider::KnownProvider::Claude,
        process.clone(),
    );
    let minted = provisional.provisional_id().to_string();
    registry
        .state
        .seen_runtimes()
        .absorb(crate::sweep::Pass::Read {
            found: vec![provisional],
            uninspected: Default::default(),
        });

    let rows = session_rows(&registry.state).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["session_id"], minted);
    assert_eq!(rows[0]["origin"], method::ORIGIN_DISCOVERED);
    assert_eq!(rows[0]["provider"], json!({"name": "claude"}));

    let session = corral_core::CorralSessionId::mint();
    registry.state.seen_runtimes().identify(
        crate::provider::KnownProvider::Claude,
        &process,
        crate::sweep::Identified {
            session,
            external_id: corral_core::ExternalId::new("session-abc").expect("an identity"),
            run: corral_core::RunId::mint(),
        },
    );

    let rows = session_rows(&registry.state).await;
    assert_eq!(
        rows.len(),
        1,
        "the provisional row was joined, not replaced"
    );
    assert_eq!(rows[0]["session_id"], session.to_string());
    assert_eq!(rows[0]["origin"], method::ORIGIN_DISCOVERED);
    assert_eq!(
        rows[0]["provider"],
        json!({"name": "claude", "external_id": "session-abc"})
    );
    assert_eq!(rows[0]["terminal_access"], "unavailable");
}

/// A session Corral launched is one row, whatever the sweep meets. Its
/// process is on the same table as every provider outside Corral, and a pass
/// that found it — as a pass on Linux does — must not list the session a
/// second time as a runtime outside Corral, while a runtime in a group that
/// is not Corral's is listed as before.
#[tokio::test]
async fn a_managed_runtime_the_sweep_finds_is_not_listed_a_second_time() {
    let registry = Registry::new("owned-runtime");
    let session = new_raw_session(&registry.state, &["/bin/sh", "-c", "sleep 30"]).await;
    let owned = registry
        .state
        .with_runtime(|runtime| runtime.owned.groups())
        .expect("the runtime");
    assert_eq!(owned.len(), 1, "the launch registered its child");
    let child = *owned.iter().next().expect("the child's group");

    let as_the_sweep_sees_it = |pid: u32, group: u32| {
        crate::sweep::RuntimeCandidate::recognized(
            crate::provider::KnownProvider::Claude,
            crate::platform::process::ProcessIdentity {
                pid,
                parent: std::process::id(),
                group,
                started: std::time::UNIX_EPOCH + std::time::Duration::from_secs(500),
                executable: PathBuf::from("/usr/local/bin/claude"),
            },
        )
    };
    let changes = crate::sweep::settle(&registry.state, async {
        crate::sweep::Pass::Read {
            found: vec![as_the_sweep_sees_it(child, child)],
            uninspected: Default::default(),
        }
    })
    .await;

    assert_eq!(changes, crate::sweep::Changes::default());
    let rows = session_rows(&registry.state).await;
    assert_eq!(
        rows.len(),
        1,
        "the managed session was listed again: {rows:?}"
    );
    assert_eq!(rows[0]["session_id"], session);
    assert_eq!(rows[0]["origin"], method::ORIGIN_MANAGED);

    let changes = crate::sweep::settle(&registry.state, async {
        crate::sweep::Pass::Read {
            found: vec![
                as_the_sweep_sees_it(child, child),
                as_the_sweep_sees_it(child + 1, 1),
            ],
            uninspected: Default::default(),
        }
    })
    .await;

    assert_eq!(changes.appeared.len(), 1);
    let rows = session_rows(&registry.state).await;
    assert_eq!(rows.len(), 2, "a runtime outside Corral is still listed");

    registry
        .state
        .with_runtime(|runtime| runtime.sessions.get(session.parse().expect("an id")))
        .flatten()
        .expect("the session")
        .shut_down();
}

/// `session.new` for a plain command, answered with the Session it started.
async fn new_raw_session(state: &Arc<DaemonState>, argv: &[&str]) -> String {
    let Dispatch::Reply(Frame::Response(response)) = dispatch(
        &request(
            method::SESSION_NEW,
            Some(json!({
                "command_id": corral_core::CorralSessionId::mint().to_string(),
                "argv": argv,
            })),
        ),
        state,
    )
    .await
    else {
        panic!("expected a plain reply");
    };
    match response.outcome {
        Outcome::Result(value) => value["session_id"]
            .as_str()
            .expect("a session id")
            .to_owned(),
        Outcome::Error(error) => panic!("expected a started session, got {error}"),
    }
}

async fn session_rows(state: &Arc<DaemonState>) -> Vec<serde_json::Value> {
    let Dispatch::Reply(Frame::Response(response)) =
        dispatch(&request(method::SESSION_LIST, None), state).await
    else {
        panic!("expected a plain reply");
    };
    match response.outcome {
        Outcome::Result(mut value) => match value["sessions"].take() {
            serde_json::Value::Array(rows) => rows,
            other => panic!("expected rows, got {other}"),
        },
        Outcome::Error(error) => panic!("expected a result, got {error}"),
    }
}

/// The same law, applied to the refusal a person meets first.
///
/// Every client renders this string as it stands — including the session list,
/// which cannot append its own hint the way the command line does — so the
/// daemon's sentence may name neither Corral's machinery nor any one surface's
/// syntax (`PRODUCT.md` §8).
#[test]
fn the_unknown_agent_refusal_exposes_no_machinery_and_no_surface() {
    let said = unknown_provider("bash").to_lowercase();

    for jargon in ["daemon", "argv", "provider", "binding", "token", "runtime"] {
        assert!(!said.contains(jargon), "{said:?} exposes {jargon}");
    }
    assert!(
        said.contains("bash"),
        "{said:?} does not name what was asked for"
    );
    for known in crate::provider::KnownProvider::ALL {
        assert!(
            said.contains(known.as_str()),
            "{said:?} does not name {known}, which Corral does know",
        );
    }
}

/// The name came off the wire and every client renders this sentence as it
/// stands — the list writes it into a full-screen frame it does not re-escape.
/// A name that is not one is answered without being repeated back.
#[test]
fn an_unusable_agent_name_is_not_echoed_into_the_refusal() {
    for hostile in [
        "\u{202e}drowssap",
        "clau\u{1b}[2Jde",
        &"x".repeat(ProviderId::LIMIT + 1),
    ] {
        let said = unknown_provider(hostile);

        assert!(!said.contains(hostile), "{said:?} repeated {hostile:?}");
        assert!(
            said.contains("claude"),
            "{said:?} still names what it knows"
        );
    }

    // An ordinary name is still named: the point is what may be shown, not
    // that a person should be told less.
    assert!(unknown_provider("cursor").contains("cursor"));
}

// ---------------------------------------------------------------- attention

fn result_value(dispatch: Dispatch) -> serde_json::Value {
    match dispatch {
        Dispatch::Reply(Frame::Response(response)) => match response.outcome {
            Outcome::Result(value) => value,
            Outcome::Error(error) => panic!("expected a result, got {error:?}"),
        },
        Dispatch::Reply(other) => panic!("expected a response, got {other:?}"),
        Dispatch::ReplyThenClose(_) | Dispatch::FailClosed(_) => panic!("expected a reply"),
    }
}

fn sealed_hook(asserts: corral_core::SemanticState) -> corral_core::Claim {
    corral_core::Claim {
        source: corral_core::EvidenceSource::ProviderHook,
        association: corral_core::Assurance::Deterministic,
        channel: corral_core::Channel::CorralOwnedPty,
        sealing: corral_core::Sealing::Sealed,
        asserts,
    }
}

/// The summary is the daemon's projection of its current items.
#[tokio::test]
async fn attention_summary_counts_the_ledgers_items() {
    let registry = Registry::new("attention-summary");
    let session = corral_core::CorralSessionId::mint();
    let now = crate::clock::Reading::now();
    registry.state.with_runtime(|runtime| {
        runtime.attention.observe(
            session,
            sealed_hook(corral_core::SemanticState::NeedsYou),
            now,
        );
        runtime
            .attention
            .tick(now, |_| crate::runtime::ExecutionState::Running);
    });

    let value =
        result_value(dispatch(&request(method::ATTENTION_SUMMARY, None), &registry.state).await);
    assert_eq!(value["needs_you"]["total"], 1);
    assert_eq!(value["needs_you"]["unacknowledged"], 1);
    assert_eq!(value["ready"]["total"], 0);
}

/// A stale id is refused with its own code and acknowledges nothing.
#[tokio::test]
async fn acknowledging_a_stale_item_is_refused_and_changes_nothing() {
    let registry = Registry::new("attention-ack-stale");
    let session = corral_core::CorralSessionId::mint();
    let now = crate::clock::Reading::now();
    registry.state.with_runtime(|runtime| {
        runtime.attention.observe(
            session,
            sealed_hook(corral_core::SemanticState::NeedsYou),
            now,
        );
        runtime
            .attention
            .tick(now, |_| crate::runtime::ExecutionState::Running);
    });

    let stale = corral_core::AttentionItemId::mint();
    let (code, close) = error_code(
        dispatch(
            &request(
                method::ATTENTION_ACKNOWLEDGE,
                Some(json!({"session_id": session.to_string(), "attention_item_id": stale.to_string()})),
            ),
            &registry.state,
        )
        .await,
    );
    assert_eq!(code, ErrorCode::StaleAttentionItem);
    assert!(!close);
    let value =
        result_value(dispatch(&request(method::ATTENTION_SUMMARY, None), &registry.state).await);
    assert_eq!(value["needs_you"]["unacknowledged"], 1);
}

#[tokio::test]
async fn acknowledging_the_current_item_clears_it_from_the_badge() {
    let registry = Registry::new("attention-ack");
    let session = corral_core::CorralSessionId::mint();
    let now = crate::clock::Reading::now();
    let item = registry
        .state
        .with_runtime(|runtime| {
            runtime
                .attention
                .observe(session, sealed_hook(corral_core::SemanticState::Ready), now);
            runtime
                .attention
                .tick(now, |_| crate::runtime::ExecutionState::Running);
            runtime
                .attention
                .state(session)
                .and_then(|(_, item)| item)
                .map(|item| item.id())
        })
        .flatten()
        .expect("an item");

    let value = result_value(
        dispatch(
            &request(
                method::ATTENTION_ACKNOWLEDGE,
                Some(json!({"session_id": session.to_string(), "attention_item_id": item.to_string()})),
            ),
            &registry.state,
        )
        .await,
    );
    assert_eq!(value, json!({}));
    let summary =
        result_value(dispatch(&request(method::ATTENTION_SUMMARY, None), &registry.state).await);
    assert_eq!(summary["ready"]["total"], 1);
    assert_eq!(summary["ready"]["unacknowledged"], 0);
}

/// A dispute appends to the journal and says whether the item it named was
/// already stale; the report reads the day back.
#[tokio::test]
async fn a_dispute_is_journaled_and_reported() {
    let registry = Registry::new("attention-dispute");
    let diagnostics = registry.directory.join("diagnostics");
    let now = crate::clock::Reading::now();
    registry.state.attach_journal(
        crate::attention::Journal::open(
            &diagnostics,
            crate::attention::Budget::default(),
            now.wall,
        )
        .expect("journal"),
    );
    let session = corral_core::CorralSessionId::mint();
    let item = registry
        .state
        .with_runtime(|runtime| {
            runtime.attention.observe(
                session,
                sealed_hook(corral_core::SemanticState::NeedsYou),
                now,
            );
            runtime
                .attention
                .tick(now, |_| crate::runtime::ExecutionState::Running);
            runtime
                .attention
                .state(session)
                .and_then(|(_, item)| item)
                .map(|item| item.id())
        })
        .flatten()
        .expect("an item");

    let current = result_value(
        dispatch(
            &request(
                method::ATTENTION_DISPUTE,
                Some(json!({"session_id": session.to_string(), "attention_item_id": item.to_string()})),
            ),
            &registry.state,
        )
        .await,
    );
    assert_eq!(current["stale"], false);
    let stale = result_value(
        dispatch(
            &request(
                method::ATTENTION_DISPUTE,
                Some(json!({"session_id": session.to_string(), "attention_item_id": corral_core::AttentionItemId::mint().to_string()})),
            ),
            &registry.state,
        )
        .await,
    );
    assert_eq!(stale["stale"], true);

    let report = result_value(
        dispatch(
            &request(method::ATTENTION_REPORT, Some(json!({}))),
            &registry.state,
        )
        .await,
    );
    let today = &report["days"][0];
    assert_eq!(today["disputes"], 2);
    assert_eq!(today["incomplete"], false);
}

/// A discovered runtime a delivery has identified is a Session like any
/// other: the daemon derives its attention from the same ledger and counts
/// it in the same summary, so the row it is shown on carries that state.
/// Counting a session in the heading while its row says nothing is one
/// surface contradicting the other over the same fact.
#[tokio::test]
async fn an_identified_external_row_carries_the_state_the_daemon_derived() {
    let registry = Registry::new("external-attention");
    let process = crate::platform::process::ProcessIdentity {
        pid: 5150,
        parent: 1,
        group: 5150,
        started: std::time::UNIX_EPOCH + std::time::Duration::from_secs(500),
        executable: PathBuf::from("/usr/local/bin/claude"),
    };
    let provisional = crate::sweep::RuntimeCandidate::recognized(
        crate::provider::KnownProvider::Claude,
        process.clone(),
    );
    registry
        .state
        .seen_runtimes()
        .absorb(crate::sweep::Pass::Read {
            found: vec![provisional],
            uninspected: Default::default(),
        });
    let session = corral_core::CorralSessionId::mint();
    registry.state.seen_runtimes().identify(
        crate::provider::KnownProvider::Claude,
        &process,
        crate::sweep::Identified {
            session,
            external_id: corral_core::ExternalId::new("session-abc").expect("an identity"),
            run: corral_core::RunId::mint(),
        },
    );

    let now = crate::clock::Reading::now();
    registry.state.with_runtime(|runtime| {
        runtime.attention.observe(
            session,
            corral_core::Claim {
                channel: corral_core::Channel::ExternalRuntime,
                association: corral_core::Assurance::Attested,
                ..sealed_hook(corral_core::SemanticState::NeedsYou)
            },
            now,
        );
        runtime
            .attention
            .tick(now, |_| crate::runtime::ExecutionState::Running);
    });

    let summary =
        result_value(dispatch(&request(method::ATTENTION_SUMMARY, None), &registry.state).await);
    assert_eq!(summary["needs_you"]["total"], 1, "the daemon counts it");

    let rows = session_rows(&registry.state).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["session_id"], session.to_string());
    assert_eq!(
        rows[0]["attention"]["state"], "needs_you",
        "the row the count is about says what it is"
    );
}

/// A runtime that cannot be consulted is not a runtime with nothing to say.
/// `attention.summary` answering zeroes would be Corral telling a person that
/// nothing needs them on the strength of state nobody finished writing, which
/// is the one direction an attention surface must never fail in.
#[tokio::test]
async fn attention_verbs_refuse_a_runtime_that_cannot_be_consulted() {
    let registry = Registry::new("attention-poisoned");
    let session = corral_core::CorralSessionId::mint();
    let poisoning = Arc::clone(&registry.state);
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        poisoning.with_runtime(|_| panic!("poison the runtime lock"));
    }));

    assert_eq!(
        error_code(dispatch(&request(method::ATTENTION_SUMMARY, None), &registry.state).await).0,
        ErrorCode::Busy
    );
    assert_eq!(
        error_code(
            dispatch(
                &request(
                    method::ATTENTION_DISPUTE,
                    Some(serde_json::json!({ "session_id": session.to_string() })),
                ),
                &registry.state,
            )
            .await
        )
        .0,
        ErrorCode::Busy
    );
    let journalled = registry
        .state
        .journal_dir()
        .and_then(|dir| crate::attention::report(&dir).ok())
        .map(|report| report.days.iter().map(|day| day.disputes).sum::<u64>())
        .unwrap_or(0);
    assert_eq!(journalled, 0, "a refused dispute records nothing");
}

/// `since` is documented as a day and compared against the journal's own day
/// names. A value in another shape would order against something it does not
/// mean and answer a report that looks valid — on the surface this PR offers
/// as its dogfood evidence.
#[tokio::test]
async fn attention_report_refuses_a_since_that_is_not_a_day() {
    let registry = Registry::new("attention-report-since");
    for bad in ["not-a-date", "2026-99-99", "2026-9-9", "2026-09"] {
        assert_eq!(
            error_code(
                dispatch(
                    &request(
                        method::ATTENTION_REPORT,
                        Some(serde_json::json!({ "since": bad })),
                    ),
                    &registry.state,
                )
                .await
            )
            .0,
            ErrorCode::InvalidParams,
            "{bad} is not a day the journal names"
        );
    }
    let value = result_value(
        dispatch(
            &request(
                method::ATTENTION_REPORT,
                Some(serde_json::json!({ "since": "2026-09-02" })),
            ),
            &registry.state,
        )
        .await,
    );
    assert!(value["days"].as_array().expect("days").is_empty());
}

/// A record the journal could not write is evidence that is gone, and the
/// marker is the only thing that can say so. When that cannot be written
/// either, no report of this journal may claim to be complete again: the
/// smaller count it would answer is the silent incompleteness D8 forbids.
#[tokio::test]
async fn a_journal_that_lost_a_record_it_could_not_mark_refuses_to_report() {
    use std::os::unix::fs::PermissionsExt;

    let registry = Registry::new("journal-unreportable");
    let diagnostics = registry.directory.join("diagnostics");
    let opened = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_788_350_400);
    registry.state.attach_journal(
        crate::attention::Journal::open(&diagnostics, crate::attention::Budget::default(), opened)
            .expect("journal"),
    );
    assert!(!registry.state.journal_unreportable());

    // Nothing can be created here any more: the next day's file cannot be
    // opened, and neither can its marker.
    std::fs::set_permissions(&diagnostics, PermissionsExt::from_mode(0o500)).expect("chmod");
    registry.state.journal_append(
        opened + std::time::Duration::from_secs(24 * 60 * 60),
        vec![crate::attention::Record::Dispute(
            crate::attention::DisputeRecord {
                session: corral_core::CorralSessionId::mint(),
                item: None,
                stale: false,
            },
        )],
    );

    assert!(
        registry.state.journal_unreportable(),
        "a lost record nobody could mark"
    );
    assert_eq!(
        error_code(dispatch(&request(method::ATTENTION_REPORT, None), &registry.state).await).0,
        ErrorCode::Busy
    );
    std::fs::set_permissions(&diagnostics, PermissionsExt::from_mode(0o700)).expect("restore");
}

/// The mark is the only durable thing that says a record is missing, so the
/// refusal has to end by writing it, never by the process that noticed going
/// away. A restart used to clear an in-memory flag and let the day read as a
/// complete count of what happened — the quietly smaller number this whole
/// path exists to prevent (ADR 0015 D8).
#[tokio::test]
async fn a_mark_that_could_not_be_written_lands_when_it_can_and_survives_a_restart() {
    use std::os::unix::fs::PermissionsExt;

    let registry = Registry::new("journal-mark-retried");
    let diagnostics = registry.directory.join("diagnostics");
    let opened = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_788_350_400);
    registry.state.attach_journal(
        crate::attention::Journal::open(&diagnostics, crate::attention::Budget::default(), opened)
            .expect("journal"),
    );
    let dispute = || {
        vec![crate::attention::Record::Dispute(
            crate::attention::DisputeRecord {
                session: corral_core::CorralSessionId::mint(),
                item: None,
                stale: false,
            },
        )]
    };
    registry.state.journal_append(opened, dispute());

    let next_day = opened + std::time::Duration::from_secs(24 * 60 * 60);
    std::fs::set_permissions(&diagnostics, PermissionsExt::from_mode(0o500)).expect("chmod");
    registry.state.journal_append(next_day, dispute());
    assert_eq!(
        error_code(dispatch(&request(method::ATTENTION_REPORT, None), &registry.state).await).0,
        ErrorCode::Busy,
        "nothing on disk says the record is missing"
    );

    // The filesystem comes back. The refusal ends by the mark landing.
    std::fs::set_permissions(&diagnostics, PermissionsExt::from_mode(0o700)).expect("restore");
    let value =
        result_value(dispatch(&request(method::ATTENTION_REPORT, None), &registry.state).await);
    let marked = value["days"]
        .as_array()
        .expect("days")
        .iter()
        .find(|day| day["incomplete"] == true)
        .expect("the day whose record was lost")
        .clone();
    assert_eq!(marked["disputes"], 0);

    // A restart: another daemon, the same diagnostics directory. The day it
    // reads is the day the mark made, not a complete one.
    let restarted = Registry::new("journal-mark-restarted");
    restarted.state.attach_journal(
        crate::attention::Journal::open(&diagnostics, crate::attention::Budget::default(), opened)
            .expect("journal"),
    );
    let after =
        result_value(dispatch(&request(method::ATTENTION_REPORT, None), &restarted.state).await);
    let still = after["days"]
        .as_array()
        .expect("days")
        .iter()
        .find(|day| day["date"] == marked["date"])
        .expect("the same day");
    assert_eq!(still["incomplete"], true, "a restart repairs nothing");
}

/// The scenario the marker exists for: a day is written, a later record
/// cannot be, and the day must not go on reading as a complete count of what
/// happened. The writer marks it, and the report says INCOMPLETE rather than
/// answering the smaller number (ADR 0015 D8).
#[tokio::test]
async fn a_record_the_journal_could_not_write_makes_its_day_report_incomplete() {
    let registry = Registry::new("journal-write-failed");
    let diagnostics = registry.directory.join("diagnostics");
    let opened = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_788_350_400);
    registry.state.attach_journal(
        crate::attention::Journal::open(&diagnostics, crate::attention::Budget::default(), opened)
            .expect("journal"),
    );
    let dispute = || {
        vec![crate::attention::Record::Dispute(
            crate::attention::DisputeRecord {
                session: corral_core::CorralSessionId::mint(),
                item: None,
                stale: false,
            },
        )]
    };
    registry.state.journal_append(opened, dispute());

    // A stale entry the daily prune cannot remove: the next day's rollover
    // fails before the record reaches the file. The directory itself stays
    // writable, so the marker can still be written.
    std::fs::create_dir(diagnostics.join("attention-journal-2020-01-01.incomplete"))
        .expect("a stale entry prune will choke on");
    let next_day = opened + std::time::Duration::from_secs(24 * 60 * 60);
    registry.state.journal_append(next_day, dispute());

    assert!(
        !registry.state.journal_unreportable(),
        "the marker was written, so reporting still answers"
    );
    let value =
        result_value(dispatch(&request(method::ATTENTION_REPORT, None), &registry.state).await);
    let days = value["days"].as_array().expect("days");
    let written = days
        .iter()
        .find(|day| day["disputes"] == 1)
        .expect("the day that was written");
    assert_eq!(written["incomplete"], false);
    let lost = days
        .iter()
        .find(|day| day["incomplete"] == true && day["date"] != "2020-01-01")
        .expect("the day whose record was lost");
    assert_ne!(lost["date"], written["date"], "the day after");
    assert_eq!(lost["disputes"], 0, "the record never reached the file");
}
