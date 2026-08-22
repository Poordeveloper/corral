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
