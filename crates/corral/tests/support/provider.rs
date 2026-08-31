//! Scripting the stand-in provider, and reading what a managed session became.
//!
//! The payloads are the shapes the providers actually produce — the same ones
//! `crates/corrald/fixtures/claude-hooks` and
//! `crates/corrald/fixtures/codex-notify` hold — so a test drives the daemon
//! with the format rather than with a shape a test author imagined.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{Value, json};

use super::TestAccount;
use super::wire::RawClient;

/// One scripted run of the stand-in provider.
pub struct Script {
    events: PathBuf,
    argv: PathBuf,
    hold: bool,
}

impl Script {
    /// A script named inside this account's scratch space, so two scenarios in
    /// one test never read each other's file.
    pub fn new(account: &TestAccount, name: &str) -> Self {
        let events = account.scratch().join(format!("{name}-events.jsonl"));
        let argv = account.scratch().join(format!("{name}-argv.txt"));
        let _ = std::fs::remove_file(&events);
        let _ = std::fs::remove_file(&argv);
        std::fs::write(&events, "").expect("an events file");
        Self {
            events,
            argv,
            hold: false,
        }
    }

    /// Keep the run alive until its terminal closes, so its Session stays
    /// `running`.
    pub fn holding(mut self) -> Self {
        self.hold = true;
        self
    }

    /// Append one payload the stand-in will fire, in order.
    pub fn fires(self, payload: &Value) -> Self {
        use std::io::Write as _;
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&self.events)
            .expect("an events file");
        writeln!(file, "{payload}").expect("append a scripted event");
        self
    }

    /// The environment a process must carry to run this script.
    pub fn environment(&self) -> Vec<(&'static str, String)> {
        let mut environment = vec![
            (
                "CORRAL_MOCK_PROVIDER_EVENTS",
                self.events.to_string_lossy().into_owned(),
            ),
            (
                "CORRAL_MOCK_PROVIDER_ARGV",
                self.argv.to_string_lossy().into_owned(),
            ),
        ];
        if self.hold {
            environment.push(("CORRAL_MOCK_PROVIDER_HOLD", "1".to_owned()));
        }
        environment
    }

    /// Every argv the stand-in was launched with, in order.
    pub fn launches(&self) -> Vec<String> {
        std::fs::read_to_string(&self.argv)
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }
}

/// A `SessionStart` payload, in the shape Claude Code writes it.
pub fn session_start(session_id: &str, source: &str) -> Value {
    json!({
        "session_id": session_id,
        "transcript_path": transcript(session_id),
        "cwd": "/work/demo",
        "hook_event_name": "SessionStart",
        "source": source,
    })
}

pub fn user_prompt_submit(session_id: &str) -> Value {
    json!({
        "session_id": session_id,
        "transcript_path": transcript(session_id),
        "cwd": "/work/demo",
        "permission_mode": "default",
        "hook_event_name": "UserPromptSubmit",
        "prompt": "say hello",
    })
}

pub fn stop(session_id: &str) -> Value {
    json!({
        "session_id": session_id,
        "transcript_path": transcript(session_id),
        "cwd": "/work/demo",
        "hook_event_name": "Stop",
        "stop_hook_active": false,
        "last_assistant_message": "hello",
    })
}

pub fn notification(session_id: &str) -> Value {
    json!({
        "session_id": session_id,
        "transcript_path": transcript(session_id),
        "cwd": "/work/demo",
        "hook_event_name": "Notification",
        "message": "Claude is waiting for your input",
    })
}

pub fn session_end(session_id: &str) -> Value {
    json!({
        "session_id": session_id,
        "transcript_path": transcript(session_id),
        "cwd": "/work/demo",
        "hook_event_name": "SessionEnd",
        "reason": "other",
    })
}

/// An `agent-turn-complete` notification, in the shape Codex writes it.
///
/// `client` is the interactive TUI's, which is the whole managed surface
/// (ADR 0009 D1).
pub fn turn_complete(thread_id: &str) -> Value {
    json!({
        "type": "agent-turn-complete",
        "thread-id": thread_id,
        "turn-id": "01a05770-2763-74e0-a44c-e6156a0f8cc3",
        "cwd": "/work/demo",
        "client": "codex-tui",
        "input-messages": ["say hello"],
        "last-assistant-message": "hello",
    })
}

fn transcript(session_id: &str) -> String {
    format!("/home/example/.claude/projects/-work-demo/{session_id}.jsonl")
}

/// How long a test waits for a hook event to travel the whole real path:
/// stand-in → relay → endpoint → ingestion → store.
pub const DELIVERED: Duration = Duration::from_secs(10);

/// The Corral-owned per-launch files a daemon currently holds.
pub fn launch_files(account: &TestAccount) -> Vec<String> {
    let directory = account.state_dir().join("launch");
    let Ok(entries) = std::fs::read_dir(directory) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

/// The injected settings file a Run was given, if it is still there.
pub fn launch_file_for(account: &TestAccount, run: &str) -> Option<PathBuf> {
    let path = account
        .state_dir()
        .join("launch")
        .join(format!("corral-launch-{run}.json"));
    path.exists().then_some(path)
}

/// The hook endpoint's pathname, by the daemon's own rule.
///
/// Asked of `corral-rendezvous` rather than spelled here: a literal would let
/// the layout move under a test that kept passing against the old pathname.
pub fn hook_socket(account: &TestAccount) -> PathBuf {
    corral_rendezvous::RendezvousPaths::for_corral_root(account.corral_root())
        .expect("a usable rendezvous layout")
        .hook_socket()
        .to_path_buf()
}

/// Every session the daemon lists.
pub fn sessions(client: &mut RawClient, id: u64) -> Vec<Value> {
    let answer = client
        .request(id, "session.list", None)
        .expect("session.list answered");
    answer["outcome"]["result"]["sessions"]
        .as_array()
        .expect("a list")
        .clone()
}

/// Every durable fact the log holds, oldest first.
///
/// Reading what the daemon wrote is the point, and going through SQLite is how
/// a fact becomes visible.
pub fn recorded_kinds(registry: &Path) -> Vec<String> {
    #[allow(clippy::disallowed_methods)]
    let connection = rusqlite::Connection::open(registry).expect("open the registry");
    let mut statement = connection
        .prepare("SELECT kind FROM session_events ORDER BY global_seq")
        .expect("prepare");
    let rows = statement
        .query_map([], |row| row.get::<_, String>(0))
        .expect("query");
    rows.map(|row| row.expect("a kind")).collect()
}

/// One session out of a `session.list` answer, by id.
pub fn listed<'a>(sessions: &'a [Value], session_id: &str) -> Option<&'a Value> {
    sessions
        .iter()
        .find(|session| session["session_id"] == session_id)
}

/// What a listed session says its provider identity is, if it says anything.
pub fn external_id(session: &Value) -> Option<&str> {
    session.get("provider")?.get("external_id")?.as_str()
}

pub fn provider_name(session: &Value) -> Option<&str> {
    session.get("provider")?.get("name")?.as_str()
}

pub fn agent_event_kind(session: &Value) -> Option<&str> {
    session.get("agent_event")?.get("kind")?.as_str()
}

/// A directory a launched session can legitimately run in.
pub fn workdir(account: &TestAccount) -> &Path {
    account.scratch()
}
