use std::fmt;
use std::io;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::envelope::Frame;

/// An implementation safety limit, not a wire number.
///
/// It exists so a peer cannot make the other allocate without bound; it sits
/// far above any legitimate protocol 1 message. A future feature that could
/// approach it has to solve limit compatibility explicitly rather than quietly
/// raising it.
pub const MAX_FRAME_BYTES: usize = 1 << 20;

/// Why a frame could not be read.
///
/// The two fault classes are different facts even though PR1 closes on both: a
/// framing fault means the byte stream itself is no longer trustworthy, while
/// an envelope fault means one well-delimited message was not something this
/// version can interpret.
#[derive(Debug)]
pub enum FrameError {
    Framing(FramingFault),
    Envelope { detail: String },
    Io(io::Error),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FramingFault {
    /// A frame exceeded the safety limit before its boundary appeared.
    Oversize,
    /// The peer stopped mid-frame: bytes arrived with no boundary after them.
    Truncated,
    /// The bytes up to the boundary were not text.
    InvalidUtf8,
}

impl fmt::Display for FramingFault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            Self::Oversize => "a frame exceeded the safety limit",
            Self::Truncated => "the peer closed mid-frame",
            Self::InvalidUtf8 => "a frame was not valid UTF-8",
        };
        f.write_str(text)
    }
}

impl fmt::Display for FrameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Framing(fault) => write!(f, "framing fault: {fault}"),
            Self::Envelope { detail } => write!(f, "undecodable envelope: {detail}"),
            Self::Io(source) => write!(f, "transport failure: {source}"),
        }
    }
}

impl std::error::Error for FrameError {}

/// Decode one complete frame body.
pub fn decode_frame(line: &[u8]) -> Result<Frame, FrameError> {
    if line.len() > MAX_FRAME_BYTES {
        return Err(FrameError::Framing(FramingFault::Oversize));
    }
    let text =
        std::str::from_utf8(line).map_err(|_| FrameError::Framing(FramingFault::InvalidUtf8))?;
    serde_json::from_str(text).map_err(|source| FrameError::Envelope {
        detail: source.to_string(),
    })
}

/// Encode one frame including its boundary.
pub fn encode_frame(frame: &Frame) -> Result<Vec<u8>, FrameError> {
    let mut bytes = serde_json::to_vec(frame).map_err(|source| FrameError::Envelope {
        detail: source.to_string(),
    })?;
    bytes.push(b'\n');
    Ok(bytes)
}

/// Reads newline-delimited frames.
///
/// The delimiter is a byte a JSON encoder never emits inside a value, so the
/// boundary is recoverable without a length prefix while messages stay legible
/// in a log.
pub struct FrameReader<R> {
    inner: R,
    pending: Vec<u8>,
    eof: bool,
}

impl<R: AsyncRead + Unpin> FrameReader<R> {
    pub fn new(inner: R) -> Self {
        Self {
            inner,
            pending: Vec::new(),
            eof: false,
        }
    }

    /// The next frame, or `None` once the peer has closed cleanly.
    ///
    /// Bytes left over without a boundary at EOF are a truncated frame, not a
    /// clean close, and are reported as a framing fault.
    pub async fn read_frame(&mut self) -> Result<Option<Frame>, FrameError> {
        loop {
            if let Some(boundary) = self.pending.iter().position(|byte| *byte == b'\n') {
                let line: Vec<u8> = self.pending.drain(..=boundary).collect();
                let body = &line[..boundary];
                return decode_frame(body).map(Some);
            }
            if self.pending.len() > MAX_FRAME_BYTES {
                return Err(FrameError::Framing(FramingFault::Oversize));
            }
            if self.eof {
                return if self.pending.is_empty() {
                    Ok(None)
                } else {
                    Err(FrameError::Framing(FramingFault::Truncated))
                };
            }

            let mut chunk = [0_u8; 4096];
            let read = self.inner.read(&mut chunk).await.map_err(FrameError::Io)?;
            if read == 0 {
                self.eof = true;
            } else {
                self.pending.extend_from_slice(&chunk[..read]);
            }
        }
    }
}

/// Writes newline-delimited frames.
pub struct FrameWriter<W> {
    inner: W,
}

impl<W: AsyncWrite + Unpin> FrameWriter<W> {
    pub fn new(inner: W) -> Self {
        Self { inner }
    }

    pub async fn write_frame(&mut self, frame: &Frame) -> Result<(), FrameError> {
        let bytes = encode_frame(frame)?;
        self.inner.write_all(&bytes).await.map_err(FrameError::Io)?;
        self.inner.flush().await.map_err(FrameError::Io)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::envelope::RequestId;

    #[tokio::test]
    async fn frames_are_read_one_at_a_time_from_a_shared_buffer() {
        let mut stream = Vec::new();
        stream.extend(encode_frame(&Frame::request(RequestId(1), "ping", None)).expect("encode"));
        stream.extend(encode_frame(&Frame::request(RequestId(2), "ping", None)).expect("encode"));

        let mut reader = FrameReader::new(stream.as_slice());

        let first = reader.read_frame().await.expect("first").expect("present");
        let second = reader.read_frame().await.expect("second").expect("present");
        assert!(reader.read_frame().await.expect("eof").is_none());

        assert!(matches!(first, Frame::Request(request) if request.id == RequestId(1)));
        assert!(matches!(second, Frame::Request(request) if request.id == RequestId(2)));
    }

    #[tokio::test]
    async fn a_truncated_frame_is_a_framing_fault_not_a_clean_close() {
        let mut reader = FrameReader::new(&b"{\"type\":\"request\""[..]);

        let error = reader.read_frame().await.expect_err("truncated");

        assert!(matches!(
            error,
            FrameError::Framing(FramingFault::Truncated)
        ));
    }

    #[tokio::test]
    async fn an_oversized_frame_is_refused_before_it_is_buffered_whole() {
        let oversized = vec![b'x'; MAX_FRAME_BYTES + 16];
        let mut reader = FrameReader::new(oversized.as_slice());

        let error = reader.read_frame().await.expect_err("oversize");

        assert!(matches!(error, FrameError::Framing(FramingFault::Oversize)));
    }

    #[test]
    fn a_non_utf8_frame_is_a_framing_fault() {
        let error = decode_frame(&[0xff, 0xfe]).expect_err("not text");

        assert!(matches!(
            error,
            FrameError::Framing(FramingFault::InvalidUtf8)
        ));
    }

    #[test]
    fn a_complete_but_undecodable_frame_is_an_envelope_fault() {
        let error = decode_frame(b"{\"type\":\"request\"}").expect_err("missing fields");

        assert!(matches!(error, FrameError::Envelope { .. }));
    }
}
