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
    pub fn establish(&mut self) -> Value {
        let response = self.say_hello(1, 1);
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

    /// Send a hello and report whether the daemon closed instead of
    /// answering.
    ///
    /// The write itself may fail once the daemon has closed, which is the same
    /// answer: the connection is gone.
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
}

/// A stand-in daemon that answers however a test needs.
///
/// Some client behaviour — refusing an incompatible daemon, refusing to trust
/// a peer's own compatibility verdict, giving up on a peer that never speaks —
/// can only be observed against a peer the real daemon would never be.
/// Everything else about the connection is real.
pub struct FakeDaemon {
    connections: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
}

impl FakeDaemon {
    /// How many times a client has connected. A client that treats a refusal
    /// as terminal connects exactly once.
    pub fn connections(&self) -> usize {
        self.connections.load(Ordering::SeqCst)
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
    let stop = Arc::new(AtomicBool::new(false));
    let counter = Arc::clone(&connections);
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
                    }
                }
                Err(source) if source.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(_) => return,
            }
        }
    });

    FakeDaemon { connections, stop }
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
