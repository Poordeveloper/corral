//! A deliberately dumb protocol client.
//!
//! These tests assert on the bytes a real peer would see, so they build frames
//! as plain JSON instead of reusing the production types. Reusing them would
//! let an encoding mistake agree with itself.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use serde_json::{Value, json};

const READ_TIMEOUT: Duration = Duration::from_secs(10);

pub struct RawClient {
    writer: UnixStream,
    reader: BufReader<UnixStream>,
}

impl RawClient {
    pub fn connect(socket: &Path) -> Self {
        Self::try_connect(socket).expect("connect to the daemon")
    }

    /// `None` when nothing is listening — a daemon that has already exited is
    /// an expected outcome in the lifetime tests.
    pub fn try_connect(socket: &Path) -> Option<Self> {
        let writer = UnixStream::connect(socket).ok()?;
        writer
            .set_read_timeout(Some(READ_TIMEOUT))
            .expect("set a read timeout");
        let reader = BufReader::new(writer.try_clone().expect("clone the stream"));
        Some(Self { writer, reader })
    }

    pub fn send(&mut self, frame: &Value) {
        let mut line = serde_json::to_vec(frame).expect("encode a frame");
        line.push(b'\n');
        self.writer.write_all(&line).expect("write a frame");
        self.writer.flush().expect("flush a frame");
    }

    /// Returns false when the peer closed mid-write.
    pub fn send_raw_tolerating_close(&mut self, bytes: &[u8]) -> bool {
        if self.writer.write_all(bytes).is_err() {
            return false;
        }
        self.writer.flush().is_ok()
    }

    pub fn send_raw(&mut self, bytes: &[u8]) {
        self.writer.write_all(bytes).expect("write bytes");
        self.writer.flush().expect("flush bytes");
    }

    /// The next frame, or `None` once the daemon has closed.
    pub fn receive(&mut self) -> Option<Value> {
        let mut line = String::new();
        match self.reader.read_line(&mut line) {
            Ok(0) => None,
            Ok(_) => Some(serde_json::from_str(&line).expect("the daemon sent a decodable frame")),
            Err(source) => panic!("reading from the daemon failed: {source}"),
        }
    }

    /// Complete the handshake at this build's own protocol version.
    ///
    /// Read from the constants rather than written down, so a version change
    /// moves the whole suite with it instead of leaving every test asserting
    /// against a number the build no longer declares.
    pub fn establish(&mut self) -> Value {
        let response = self.say_hello(
            corral_protocol::PROTOCOL_VERSION,
            corral_protocol::MIN_COMPATIBLE_PEER_VERSION,
        );
        assert_eq!(
            response["outcome"]["result"]["compatibility_result"], "compatible",
            "expected an established connection, got {response}"
        );
        response
    }

    pub fn say_hello(&mut self, protocol_version: u32, min_compatible_peer_version: u32) -> Value {
        self.send(&json!({
            "type": "request",
            "id": 0,
            "method": "hello",
            "params": {
                "protocol_version": protocol_version,
                "min_compatible_peer_version": min_compatible_peer_version,
            },
        }));
        self.receive().expect("the daemon answered the hello")
    }

    /// A hello that claims the terminal-data role by redeeming a token.
    pub fn say_hello_with_role(&mut self, attach_token: &str) -> Value {
        let versions = corral_protocol::local_versions();
        self.send(&json!({
            "type": "request",
            "id": 0,
            "method": "hello",
            "params": {
                "protocol_version": versions.protocol_version,
                "min_compatible_peer_version": versions.min_compatible_peer_version,
                "role": { "kind": "terminal_data", "attach_token": attach_token },
            },
        }));
        self.receive().expect("the daemon answered the hello")
    }

    /// Give up JSON framing and return the halves, for a connection that has
    /// transitioned to terminal frames.
    ///
    /// The reader is the buffered one: the daemon sends the first snapshot
    /// immediately after the hello, so those bytes may already be buffered and
    /// dropping them would lose the screen the test came for.
    pub fn into_parts(self) -> (UnixStream, BufReader<UnixStream>) {
        (self.writer, self.reader)
    }

    /// Send a hello and report whether the daemon closed instead of
    /// answering.
    ///
    /// The write itself may fail once the daemon has closed, which is the same
    /// answer: the connection is gone.
    pub fn say_hello_expecting_close(&mut self) -> bool {
        let mut line = serde_json::to_vec(&json!({
            "type": "request",
            "id": 0,
            "method": "hello",
            "params": {"protocol_version": 1, "min_compatible_peer_version": 1},
        }))
        .expect("encode a frame");
        line.push(b'\n');
        if self.writer.write_all(&line).is_err() {
            return true;
        }
        let _ = self.writer.flush();
        self.receive().is_none()
    }

    pub fn request(&mut self, id: u64, method: &str, params: Option<Value>) -> Option<Value> {
        let mut frame = json!({"type": "request", "id": id, "method": method});
        if let Some(params) = params {
            frame["params"] = params;
        }
        self.send(&frame);
        self.receive()
    }
}

/// The error code in a response frame, if it carries one.
pub fn error_code(frame: &Value) -> Option<&str> {
    frame["outcome"]["error"]["code"].as_str()
}

/// The sentence a refusal put in front of a person.
pub fn refused_with(frame: &Value) -> String {
    frame["outcome"]["error"]["message"]
        .as_str()
        .unwrap_or_default()
        .to_owned()
}

/// What a stand-in daemon does with each connection it accepts.
pub enum FakeBehaviour {
    /// Read one line, write these bytes, close.
    AnswerThenClose(Vec<u8>),
    /// Read nothing, answer nothing, hold the connection open.
    ///
    /// A peer that is reachable but never becomes protocol-ready is what an
    /// overall activation budget exists for, and a closing fake cannot
    /// produce it: closing looks like a daemon on its way out, which is
    /// legitimately retried.
    StaySilent,
    /// A daemon from before this build: it names `capabilities` in its hello,
    /// lists `session` alone, serves `session.resume`, and answers everything
    /// else the way a daemon that never heard of a method does.
    ///
    /// The only way to observe what a new client does opposite an old daemon,
    /// which is the ordinary state of a machine where one half was upgraded.
    OlderDaemon {
        capabilities: Vec<String>,
        session: String,
    },
    /// Answer the hello at once, then take `delay` over every request after
    /// it, watching for a second one arriving before the first is answered.
    ///
    /// A daemon this slow is not what a person meets; it is the only way to
    /// observe whether a polling surface waits for its answer or queues
    /// another question behind it.
    AnswerSlowly { delay: Duration },
}

/// A stand-in daemon that answers however a test needs.
///
/// Some client behaviour — refusing an incompatible daemon, refusing to trust
/// a peer's own compatibility verdict, giving up on a peer that never speaks —
/// can only be observed against a peer the real daemon would never be.
/// Everything else about the connection is real.
pub struct FakeDaemon {
    connections: Arc<AtomicUsize>,
    requests: Arc<AtomicUsize>,
    overlapped: Arc<AtomicBool>,
    methods: Arc<std::sync::Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
}

impl FakeDaemon {
    /// How many times a client has connected. A client that treats a refusal
    /// as terminal connects exactly once.
    pub fn connections(&self) -> usize {
        self.connections.load(Ordering::SeqCst)
    }

    /// How many requests after the hello this daemon has been sent.
    pub fn requests(&self) -> usize {
        self.requests.load(Ordering::SeqCst)
    }

    /// Whether a request ever arrived while an earlier one was still being
    /// answered.
    pub fn overlapped(&self) -> bool {
        self.overlapped.load(Ordering::SeqCst)
    }

    /// Every method this daemon was asked for, in order, excluding the hello.
    pub fn methods(&self) -> Vec<String> {
        self.methods
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
}

impl Drop for FakeDaemon {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

/// Serve `behaviour` to every connection.
pub fn spawn_fake_daemon(socket: &Path, behaviour: FakeBehaviour) -> FakeDaemon {
    super::create_private_dir_all(socket.parent().expect("run directory"));
    let listener = std::os::unix::net::UnixListener::bind(socket).expect("bind the fake daemon");
    listener
        .set_nonblocking(true)
        .expect("poll for connections");

    let connections = Arc::new(AtomicUsize::new(0));
    let requests = Arc::new(AtomicUsize::new(0));
    let overlapped = Arc::new(AtomicBool::new(false));
    let methods = Arc::new(std::sync::Mutex::new(Vec::new()));
    let stop = Arc::new(AtomicBool::new(false));
    let counter = Arc::clone(&connections);
    let asked = Arc::clone(&requests);
    let queued = Arc::clone(&overlapped);
    let logged = Arc::clone(&methods);
    let stopped = Arc::clone(&stop);

    std::thread::spawn(move || {
        // Held open so `StaySilent` really is silent rather than a close.
        let mut held = Vec::new();
        while !stopped.load(Ordering::SeqCst) {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    counter.fetch_add(1, Ordering::SeqCst);
                    stream.set_nonblocking(false).expect("blocking connection");
                    match &behaviour {
                        FakeBehaviour::AnswerThenClose(reply) => {
                            let mut reader = BufReader::new(stream.try_clone().expect("clone"));
                            let mut line = String::new();
                            let _ = reader.read_line(&mut line);
                            let _ = stream.write_all(reply);
                            let _ = stream.flush();
                        }
                        FakeBehaviour::StaySilent => held.push(stream),
                        FakeBehaviour::OlderDaemon {
                            capabilities,
                            session,
                        } => answer_as_an_older_daemon(stream, capabilities, session, &logged),
                        FakeBehaviour::AnswerSlowly { delay } => {
                            answer_slowly(stream, *delay, &asked, &queued);
                        }
                    }
                }
                Err(source) if source.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(_) => return,
            }
        }
    });

    FakeDaemon {
        connections,
        requests,
        overlapped,
        methods,
        stop,
    }
}

/// Serve one connection as a daemon that predates this build.
fn answer_as_an_older_daemon(
    mut stream: UnixStream,
    capabilities: &[String],
    session: &str,
    methods: &Arc<std::sync::Mutex<Vec<String>>>,
) {
    let mut reader = BufReader::new(stream.try_clone().expect("clone"));

    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).unwrap_or(0) == 0 {
            return;
        }
        let Ok(request) = serde_json::from_str::<Value>(&line) else {
            return;
        };
        let id = request.get("id").cloned().unwrap_or_else(|| json!(0));
        let method = request
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();

        let outcome = match method.as_str() {
            "hello" => json!({"result": {
                "protocol_version": corral_protocol::PROTOCOL_VERSION,
                "min_compatible_peer_version": corral_protocol::MIN_COMPATIBLE_PEER_VERSION,
                "capabilities": capabilities,
                "compatibility_result": "compatible",
            }}),
            "session.list" => json!({"result": {
                "sessions": [{"session_id": session}],
            }}),
            "session.resume" => json!({"result": {
                "session_id": session,
                "run_id": "00000000-0000-4000-8000-0000000000aa",
            }}),
            _ => json!({"error": {
                "code": "method_not_found",
                "message": format!("this daemon does not serve {method}"),
            }}),
        };
        if method != "hello" {
            methods
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push(method);
        }

        let mut reply =
            serde_json::to_vec(&json!({"type": "response", "id": id, "outcome": outcome}))
                .expect("encode");
        reply.push(b'\n');
        if stream.write_all(&reply).is_err() || stream.flush().is_err() {
            return;
        }
    }
}

/// Serve one connection slowly, and notice anything sent before an answer.
fn answer_slowly(
    mut stream: UnixStream,
    delay: Duration,
    requests: &Arc<AtomicUsize>,
    overlapped: &Arc<AtomicBool>,
) {
    let mut reader = BufReader::new(stream.try_clone().expect("clone"));

    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).unwrap_or(0) == 0 {
            return;
        }
        let Ok(request) = serde_json::from_str::<Value>(&line) else {
            return;
        };
        let id = request.get("id").cloned().unwrap_or_else(|| json!(0));

        let reply = if request.get("method").and_then(Value::as_str) == Some("hello") {
            hello_reply(
                corral_protocol::PROTOCOL_VERSION,
                corral_protocol::MIN_COMPATIBLE_PEER_VERSION,
                "compatible",
            )
        } else {
            requests.fetch_add(1, Ordering::SeqCst);
            std::thread::sleep(delay);
            // Anything already here was sent before this request was answered,
            // which is the queue a polling client must not build.
            if already_waiting(&mut reader) {
                overlapped.store(true, Ordering::SeqCst);
            }
            let mut line = serde_json::to_vec(&json!({
                "type": "response",
                "id": id,
                "outcome": {"result": {"sessions": []}},
            }))
            .expect("encode");
            line.push(b'\n');
            line
        };

        if stream.write_all(&reply).is_err() || stream.flush().is_err() {
            return;
        }
    }
}

/// Whether the peer has sent anything this connection has not read yet.
///
/// Buffered without being consumed, so the request this notices is still the
/// next one the loop reads.
fn already_waiting(reader: &mut BufReader<UnixStream>) -> bool {
    if !reader.buffer().is_empty() {
        return true;
    }
    if reader
        .get_ref()
        .set_read_timeout(Some(Duration::from_millis(20)))
        .is_err()
    {
        return false;
    }
    let waiting = reader.fill_buf().is_ok_and(|bytes| !bytes.is_empty());
    let _ = reader.get_ref().set_read_timeout(None);
    waiting
}

/// A hello reply line built from raw parts, so a test can state exactly what
/// an unusual peer would put on the wire.
pub fn hello_reply(protocol_version: u32, min_peer: u32, compatibility: &str) -> Vec<u8> {
    let mut line = serde_json::to_vec(&json!({
        "type": "response",
        "id": 0,
        "outcome": {"result": {
            "protocol_version": protocol_version,
            "min_compatible_peer_version": min_peer,
            "capabilities": [],
            "compatibility_result": compatibility,
        }},
    }))
    .expect("encode");
    line.push(b'\n');
    line
}
