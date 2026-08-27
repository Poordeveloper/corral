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

/// The terminology law, enforced where it is easiest to break: every refusal a
/// person reads names facts and actions, never Corral's machinery
/// (`PRODUCT.md` §8).
///
/// Session is the one domain noun a person is shown. `run` survives in exactly
/// one sentence — the one the founder fixed verbatim for an unverifiable end
/// (grill Q7) — and nowhere else.
#[test]
fn no_continuation_refusal_exposes_corral_machinery() {
    let refusals = [
        ResumeRefused::NotThisDaemon,
        ResumeRefused::IdentityUnknown,
        ResumeRefused::Eligibility(NativeResumeEligibility::IdentityContested),
        ResumeRefused::Eligibility(NativeResumeEligibility::AssuranceTooWeak),
        ResumeRefused::Eligibility(NativeResumeEligibility::Eligible),
        ResumeRefused::UnknownProvider("codex".to_owned()),
        ResumeRefused::RunStillLive,
        ResumeRefused::EndUnverifiable,
        ResumeRefused::NoPreviousRun,
    ];
    for refusal in refusals {
        let said = refusal.to_string().to_lowercase();
        for jargon in [
            "binding",
            "assurance",
            "evidence",
            "contested",
            "token",
            "attested",
            "heuristic",
            "deterministic",
            "eligibility",
        ] {
            assert!(!said.contains(jargon), "{said:?} exposes {jargon}");
        }
        assert!(!said.is_empty());
    }
}

/// The one sanctioned exception, kept where a change to it is visible: the
/// founder fixed this sentence, and a reworded version would be a different
/// promise about what Corral checked (grill Q7).
#[test]
fn the_unverifiable_refusal_states_the_ruling_verbatim() {
    assert_eq!(
        ResumeRefused::EndUnverifiable.to_string(),
        "Corral cannot verify that the previous run has exited, so it will not resume this \
         provider session automatically",
    );
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
    assert!(
        said.contains(crate::provider::KnownProvider::Claude.as_str()),
        "{said:?} does not name what Corral does know",
    );
}
