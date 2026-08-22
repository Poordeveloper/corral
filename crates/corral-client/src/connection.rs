use std::path::{Path, PathBuf};

use corral_protocol::method::{self, SessionListResult};
use corral_protocol::{
    ClientHello, Compatibility, Frame, FrameError, FrameReader, FrameWriter, Outcome, PeerVersions,
    RequestId, ServerHello, compatible, local_versions,
};
use serde_json::Value;
use tokio::net::UnixStream;
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};

use crate::error::{ActivationContext, ActivationError, HandshakeFault, RequestError};
use crate::spawn::SpawnedDaemon;

/// An established connection to a compatible `corrald`.
pub struct Connection {
    reader: FrameReader<OwnedReadHalf>,
    writer: FrameWriter<OwnedWriteHalf>,
    endpoint: PathBuf,
    peer: ServerHello,
    next_id: u64,
    /// Retained so a daemon this surface started is reaped rather than
    /// orphaned. Dropping it never stops the daemon.
    _daemon: Option<SpawnedDaemon>,
}

impl Connection {
    /// Keep a daemon this surface started alive for reaping purposes.
    pub(crate) fn attach_daemon(&mut self, daemon: Option<SpawnedDaemon>) {
        self._daemon = daemon;
    }

    pub fn endpoint(&self) -> &Path {
        &self.endpoint
    }

    /// The daemon's half of the negotiated handshake.
    pub fn peer(&self) -> &ServerHello {
        &self.peer
    }

    pub fn local_versions(&self) -> PeerVersions {
        local_versions()
    }

    pub async fn ping(&mut self) -> Result<(), RequestError> {
        self.call(method::PING, None).await.map(|_| ())
    }

    pub async fn session_list(&mut self) -> Result<SessionListResult, RequestError> {
        let value = self.call(method::SESSION_LIST, None).await?;
        serde_json::from_value(value).map_err(|source| RequestError::Protocol {
            detail: format!("the session list did not decode: {source}"),
        })
    }

    async fn call(&mut self, method: &str, params: Option<Value>) -> Result<Value, RequestError> {
        let id = RequestId(self.next_id);
        self.next_id += 1;

        self.writer
            .write_frame(&Frame::request(id, method, params))
            .await
            .map_err(|error| self.transport_failure(error))?;

        match self.reader.read_frame().await {
            Ok(None) => Err(RequestError::DaemonConnectionLost {
                endpoint: self.endpoint.clone(),
            }),
            Ok(Some(Frame::Response(response))) if response.id == id => match response.outcome {
                Outcome::Result(value) => Ok(value),
                Outcome::Error(error) => Err(RequestError::Refused(error)),
            },
            Ok(Some(Frame::Response(response))) => Err(RequestError::Protocol {
                detail: format!(
                    "a response for request {} arrived while {} was outstanding",
                    response.id.0, id.0
                ),
            }),
            // Protocol 1 daemons answer; they never originate. Accepting an
            // unsolicited frame here would quietly admit a wire direction this
            // version has not defined.
            Ok(Some(Frame::Request(_) | Frame::Notification(_))) => Err(RequestError::Protocol {
                detail: "the daemon sent an unsolicited frame".to_owned(),
            }),
            Err(error) => Err(self.transport_failure(error)),
        }
    }

    fn transport_failure(&self, error: FrameError) -> RequestError {
        match error {
            FrameError::Io(_) => RequestError::DaemonConnectionLost {
                endpoint: self.endpoint.clone(),
            },
            other => RequestError::Protocol {
                detail: other.to_string(),
            },
        }
    }
}

/// Run the client-first hello and decide, independently of the daemon, whether
/// the connection may be used.
///
/// `Ok(None)` means the daemon went away mid-bootstrap — a shutting-down
/// daemon, not a broken one — so the caller may keep trying within its overall
/// deadline.
pub(crate) async fn handshake(
    stream: UnixStream,
    endpoint: &Path,
    context: ActivationContext,
) -> Result<Option<Connection>, ActivationError> {
    let (read_half, write_half) = stream.into_split();
    let mut reader = FrameReader::new(read_half);
    let mut writer = FrameWriter::new(write_half);

    let ours = local_versions();
    let hello = ClientHello {
        protocol_version: ours.protocol_version,
        min_compatible_peer_version: ours.min_compatible_peer_version,
        capabilities: Default::default(),
    };
    let params = serde_json::to_value(&hello).map_err(|source| ActivationError::Handshake {
        endpoint: endpoint.to_path_buf(),
        fault: HandshakeFault::Malformed {
            detail: source.to_string(),
        },
    })?;

    let id = RequestId(0);
    if writer
        .write_frame(&Frame::request(id, method::HELLO, Some(params)))
        .await
        .is_err()
    {
        return Ok(None);
    }

    let malformed = |detail: String| ActivationError::Handshake {
        endpoint: endpoint.to_path_buf(),
        fault: HandshakeFault::Malformed { detail },
    };
    let violation = |detail: String| ActivationError::Handshake {
        endpoint: endpoint.to_path_buf(),
        fault: HandshakeFault::ProtocolViolation { detail },
    };

    let frame = match reader.read_frame().await {
        Ok(Some(frame)) => frame,
        Ok(None) => return Ok(None),
        Err(FrameError::Io(_)) => return Ok(None),
        Err(error) => return Err(violation(error.to_string())),
    };

    let response = match frame {
        Frame::Response(response) if response.id == id => response,
        Frame::Response(response) => {
            return Err(violation(format!(
                "the daemon answered request {} before the hello",
                response.id.0
            )));
        }
        Frame::Request(_) | Frame::Notification(_) => {
            return Err(violation(
                "the daemon spoke before answering the hello".to_owned(),
            ));
        }
    };

    let value = match response.outcome {
        Outcome::Result(value) => value,
        Outcome::Error(error) => {
            return Err(ActivationError::Handshake {
                endpoint: endpoint.to_path_buf(),
                fault: HandshakeFault::Refused(error),
            });
        }
    };
    let peer: ServerHello =
        serde_json::from_value(value).map_err(|source| malformed(source.to_string()))?;
    let theirs = peer.versions();

    // Both sides run the same symmetric predicate. Trusting the daemon's
    // verdict instead would make one peer's bug the other peer's behaviour.
    let our_verdict = compatible(ours, theirs);
    let their_verdict = matches!(peer.compatibility, Compatibility::Compatible);
    if our_verdict != their_verdict {
        return Err(ActivationError::Handshake {
            endpoint: endpoint.to_path_buf(),
            fault: HandshakeFault::DivergentCompatibilityVerdict { ours, theirs },
        });
    }
    if !our_verdict {
        return Err(ActivationError::IncompatibleDaemon {
            ours,
            theirs,
            endpoint: endpoint.to_path_buf(),
            context,
        });
    }

    Ok(Some(Connection {
        reader,
        writer,
        endpoint: endpoint.to_path_buf(),
        peer,
        next_id: id.0 + 1,
        _daemon: None,
    }))
}
