use super::*;

#[test]
fn a_geometry_survives_its_wire_form() {
    let geometry = PtyGeometry::expect_valid(40, 132);

    let decoded = decode_geometry(&encode_geometry(geometry)).expect("a well-formed payload");

    assert_eq!(decoded, geometry);
}

/// A short resize payload is ignored rather than guessed at: a geometry
/// invented from missing bytes would reflow a real screen.
#[test]
fn a_truncated_geometry_is_not_a_geometry() {
    assert!(decode_geometry(&[0, 24, 0]).is_none());
    assert!(decode_geometry(&[]).is_none());
}

/// The wire carries any `u16`, the daemon builds only what it will serve.
#[test]
fn a_geometry_past_what_corral_will_build_is_refused() {
    let mut payload = Vec::new();
    payload.extend_from_slice(&(crate::runtime::MAX_TERMINAL_ROWS + 1).to_be_bytes());
    payload.extend_from_slice(&80_u16.to_be_bytes());

    assert!(decode_geometry(&payload).is_none());
}

#[test]
fn an_empty_geometry_is_refused() {
    assert!(decode_geometry(&[0, 0, 0, 80]).is_none());
    assert!(decode_geometry(&[0, 24, 0, 0]).is_none());
}

#[test]
fn the_largest_geometry_corral_builds_survives_the_round_trip() {
    let geometry = PtyGeometry::expect_valid(
        crate::runtime::MAX_TERMINAL_ROWS,
        crate::runtime::MAX_TERMINAL_COLS,
    );

    assert_eq!(decode_geometry(&encode_geometry(geometry)), Some(geometry));
}

// ---------------------------------------------------------------------------
// The channel under load: grill Q10's regression matrix
// (docs/decisions/2026-09-04-pr9-spike-grill.md). A real `sh` session in a
// test daemon state, served over a socket pair, read by a client that keeps a
// qwertty replica. Screens are compared on visible cells only: the snapshot's
// cursor is Q6's defect, not this channel's.

mod under_load {
    use std::ffi::OsString;
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::time::{Duration, Instant};

    use corral_core::{CorralSessionId, RunId};
    use corral_protocol::terminal::{Epoch, FrameKind, Sequence, TerminalFrame};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::UnixStream;

    use crate::runtime::{
        AuthoritativeTerminal, LaunchRequest, PtyGeometry, SessionHandle, spawn_session,
    };
    use crate::state::DaemonState;

    const GEOMETRY: PtyGeometry = PtyGeometry::expect_valid(24, 80);
    /// A line with wide cells and an emoji, so shaping-sensitive bytes flow.
    const STORM_LINE: &str = "The quick brown fox 中文字 🦀 jumps over the lazy dog 0123456789";
    /// The last thing a storm prints, without a newline: a blank last row
    /// would meet the snapshot's trailing-row defect (Q6), which is not what
    /// these tests measure.
    const DONE: &str = "STORM-DONE";

    static COUNTER: AtomicU32 = AtomicU32::new(0);

    /// One session, registered in a daemon state of its own, torn down with
    /// the test so no `yes` outlives it.
    struct Served {
        state: Arc<DaemonState>,
        session: CorralSessionId,
        run: RunId,
        handle: Arc<SessionHandle>,
        others: Vec<Peer>,
        directory: PathBuf,
    }

    /// A second session in the same daemon state.
    struct Peer {
        session: CorralSessionId,
        run: RunId,
        handle: Arc<SessionHandle>,
    }

    impl Drop for Served {
        fn drop(&mut self) {
            self.handle.shut_down();
            for peer in &self.others {
                peer.handle.shut_down();
            }
            let _ = std::fs::remove_dir_all(&self.directory);
        }
    }

    fn storm_for(seconds: u32) -> String {
        format!(
            "yes '{STORM_LINE}' & p=$!; sleep {seconds}; kill $p; wait $p 2>/dev/null; printf '{DONE}'; sleep 60"
        )
    }

    fn served(name: &str, script: &str) -> Served {
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let directory = std::env::temp_dir().join(format!(
            "corrald-channel-{}-{unique}-{name}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).expect("create the scratch directory");
        let state = Arc::new(
            DaemonState::open(
                &directory.join("registry.sqlite3"),
                &directory.join("launch"),
                &directory,
            )
            .expect("open"),
        );
        let Peer {
            session,
            run,
            handle,
        } = spawn_into(&state, script);
        Served {
            state,
            session,
            run,
            handle,
            others: Vec::new(),
            directory,
        }
    }

    fn spawn_into(state: &Arc<DaemonState>, script: &str) -> Peer {
        let request = LaunchRequest::new(
            "/bin/sh",
            ["-c", script].iter().map(OsString::from),
            std::env::temp_dir(),
        )
        .expect("a valid launch request");
        let session = CorralSessionId::mint();
        let run = RunId::mint();
        let handle = spawn_session(&request, GEOMETRY)
            .expect("the session starts")
            .serve(session, run, state.observations().clone());
        state
            .with_runtime(|runtime| runtime.sessions.insert(handle))
            .expect("a runtime");
        let handle = state
            .with_runtime(|runtime| runtime.sessions.get(session))
            .flatten()
            .expect("the session is registered");
        Peer {
            session,
            run,
            handle,
        }
    }

    /// One client end of a channel the daemon is serving.
    struct Client {
        stream: UnixStream,
        pending: Vec<u8>,
        /// Position the last frame established, for contiguity checks.
        position: Option<(Epoch, Sequence)>,
        /// Where a sequence jump was seen and what preceded it: a jump is
        /// legal only as the first delta after a snapshot.
        illegal_gaps: Vec<((Epoch, Sequence), (Epoch, Sequence))>,
        snapshots: usize,
        deltas: usize,
        bytes: usize,
        errors: Vec<String>,
        replica: AuthoritativeTerminal,
        eof: bool,
    }

    impl Served {
        async fn open(&self) -> Client {
            self.open_for(self.session, self.run).await
        }

        fn another(&mut self, script: &str) -> usize {
            self.others.push(spawn_into(&self.state, script));
            self.others.len() - 1
        }

        async fn open_other(&self, index: usize) -> Client {
            let peer = &self.others[index];
            self.open_for(peer.session, peer.run).await
        }

        /// The daemon serves the channel on a reactor thread of its own, as
        /// in production: a client that shared the daemon's thread would
        /// starve the daemon's writer every time it parsed a frame, and the
        /// budget it overran would be the test's, not the channel's.
        async fn open_for(&self, session: CorralSessionId, run: RunId) -> Client {
            let (client, server) = std::os::unix::net::UnixStream::pair().expect("a socket pair");
            client.set_nonblocking(true).expect("nonblocking");
            server.set_nonblocking(true).expect("nonblocking");
            let state = Arc::clone(&self.state);
            std::thread::spawn(move || {
                let reactor = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("a reactor");
                reactor.block_on(async move {
                    let server = UnixStream::from_std(server).expect("adopt the server end");
                    let (mut read, write) = server.into_split();
                    super::super::serve(&mut read, write, Vec::new(), session, run, &state).await;
                });
            });
            let client = UnixStream::from_std(client).expect("adopt the client end");
            Client {
                stream: client,
                pending: Vec::new(),
                position: None,
                illegal_gaps: Vec::new(),
                snapshots: 0,
                deltas: 0,
                bytes: 0,
                errors: Vec::new(),
                replica: AuthoritativeTerminal::new(GEOMETRY),
                eof: false,
            }
        }

        /// The daemon's own screen, read through a fresh replica of its
        /// snapshot: the same bytes any newly attached client would build from.
        fn authoritative_cells(&self) -> Vec<qwertty_term_vt::snapshot::SnapshotRow> {
            let attachment = self.handle.attach().expect("the session answers");
            let snapshot = attachment.snapshot.expect("the screen encodes");
            let mut fresh = AuthoritativeTerminal::new(GEOMETRY);
            let _ = fresh.consume(b"\x1b[H\x1b[2J");
            let _ = fresh.consume(snapshot.payload());
            fresh
                .terminal()
                .expect("not poisoned")
                .snapshot()
                .visible_window(0)
                .to_vec()
        }
    }

    impl Client {
        /// The next frame, `None` at EOF, `Some(None)` when nothing arrived
        /// within the budget.
        async fn next(&mut self, budget: Duration) -> Option<Option<TerminalFrame>> {
            let deadline = Instant::now() + budget;
            let mut buffer = vec![0_u8; 65536];
            loop {
                if let Ok(Some((frame, used))) = TerminalFrame::decode_from_daemon(&self.pending) {
                    self.pending.drain(..used);
                    self.account(&frame);
                    return Some(Some(frame));
                }
                let left = deadline.saturating_duration_since(Instant::now());
                if left.is_zero() {
                    return Some(None);
                }
                match tokio::time::timeout(left, self.stream.read(&mut buffer)).await {
                    Ok(Ok(0)) | Ok(Err(_)) => {
                        self.eof = true;
                        return None;
                    }
                    Ok(Ok(read)) => self.pending.extend_from_slice(&buffer[..read]),
                    Err(_) => return Some(None),
                }
            }
        }

        fn account(&mut self, frame: &TerminalFrame) {
            let here = (frame.epoch, frame.sequence);
            match frame.kind {
                FrameKind::Snapshot => {
                    self.snapshots += 1;
                    self.replica = AuthoritativeTerminal::new(GEOMETRY);
                    let _ = self.replica.consume(b"\x1b[H\x1b[2J");
                    let _ = self.replica.consume(&frame.payload);
                    // The first delta after a snapshot carries the snapshot's
                    // own sequence (the spike's client note).
                    self.position = Some((frame.epoch, Sequence(frame.sequence.0.wrapping_sub(1))));
                }
                FrameKind::Delta => {
                    self.deltas += 1;
                    self.bytes += frame.payload.len();
                    if let Some((epoch, sequence)) = self.position
                        && !(epoch == frame.epoch && sequence.0.wrapping_add(1) == frame.sequence.0)
                    {
                        self.illegal_gaps.push(((epoch, sequence), here));
                    }
                    self.position = Some(here);
                    let _ = self.replica.consume(&frame.payload);
                }
                FrameKind::ChannelError => {
                    self.errors
                        .push(String::from_utf8_lossy(&frame.payload).into_owned());
                }
                _ => {}
            }
        }

        /// Read at full speed until a frame carries `marker`, then keep
        /// reading briefly so the tail of the stream lands.
        async fn read_until(&mut self, marker: &str, budget: Duration) -> bool {
            let deadline = Instant::now() + budget;
            let mut seen = false;
            while Instant::now() < deadline {
                match self.next(Duration::from_millis(300)).await {
                    None => return seen,
                    Some(None) => {
                        if seen {
                            return true;
                        }
                    }
                    Some(Some(frame)) => {
                        if !seen && String::from_utf8_lossy(&frame.payload).contains(marker) {
                            seen = true;
                        }
                    }
                }
            }
            seen
        }

        /// Drain whatever is buffered until the daemon's EOF, within a budget.
        async fn drained_to_eof(&mut self, budget: Duration) -> bool {
            let deadline = Instant::now() + budget;
            while Instant::now() < deadline {
                if self.next(Duration::from_millis(200)).await.is_none() {
                    return true;
                }
            }
            false
        }

        /// Read at full speed for a while, counting what arrives.
        async fn read_for(&mut self, span: Duration) -> usize {
            let deadline = Instant::now() + span;
            let mut frames = 0;
            while Instant::now() < deadline {
                match self
                    .next(deadline.saturating_duration_since(Instant::now()))
                    .await
                {
                    None => break,
                    Some(None) => break,
                    Some(Some(_)) => frames += 1,
                }
            }
            frames
        }

        fn visible_cells(&self) -> Vec<qwertty_term_vt::snapshot::SnapshotRow> {
            self.replica
                .terminal()
                .expect("not poisoned")
                .snapshot()
                .visible_window(0)
                .to_vec()
        }

        async fn send(&mut self, kind: FrameKind, payload: Vec<u8>) {
            let (epoch, sequence) = self.position.unwrap_or((Epoch(0), Sequence(0)));
            let frame = TerminalFrame {
                kind,
                epoch,
                sequence,
                payload,
            };
            self.stream
                .write_all(&frame.encode().expect("encodes"))
                .await
                .expect("the client socket writes");
        }
    }

    fn assert_healthy(client: &Client, who: &str) {
        assert!(!client.eof, "{who}: the daemon closed the channel");
        assert!(
            client.errors.is_empty(),
            "{who}: channel errors {:?}",
            client.errors
        );
        assert!(
            client.illegal_gaps.is_empty(),
            "{who}: deltas continued across a gap {:?}",
            &client.illegal_gaps[..client.illegal_gaps.len().min(3)]
        );
    }

    /// Regression 1. Before the fix, an eight-frame outbound queue closed the
    /// channel within seconds of a storm like this one.
    #[tokio::test]
    async fn a_reading_client_survives_a_ten_second_storm() {
        let served = served("storm", &storm_for(10));
        let mut client = served.open().await;

        let done = client.read_until(DONE, Duration::from_secs(25)).await;

        assert!(done, "the storm's end never arrived (eof {})", client.eof);
        assert_healthy(&client, "the reader");
        assert!(
            client.bytes > 10_000_000,
            "only {} bytes flowed",
            client.bytes
        );
        // Resyncs are a diagnostic, not the contract: a reader that lags
        // behind the budget for a moment is resynced, which is the channel
        // working. What may never happen is a close or a gap.
        eprintln!(
            "storm: {} deltas averaging {} bytes, {} snapshots",
            client.deltas,
            client.bytes / client.deltas.max(1),
            client.snapshots
        );
        assert_eq!(
            client.visible_cells(),
            served.authoritative_cells(),
            "the replica diverged from the daemon's screen"
        );
    }

    /// Regression 2. A client that stops reading is dropped within the
    /// no-progress deadline, and only that client.
    #[tokio::test]
    async fn a_client_that_stops_reading_is_dropped_within_the_deadline() {
        let served = served("stall", &storm_for(30));
        let mut stalled = served.open().await;
        let mut healthy = served.open().await;

        assert!(stalled.read_for(Duration::from_millis(300)).await > 0);
        let stall_began = Instant::now();
        let during_stall = healthy.read_for(Duration::from_secs(4)).await;
        let observed_eof = stalled
            .drained_to_eof(
                (stall_began + Duration::from_secs(8)).saturating_duration_since(Instant::now()),
            )
            .await;

        assert!(observed_eof, "the stalled client was never dropped");
        assert!(
            during_stall > 100,
            "the healthy viewer starved: {during_stall} frames"
        );
        assert_healthy(&healthy, "the healthy viewer");
    }

    /// Regression 3. A reader that falls past its byte budget is resynced by
    /// a snapshot and never handed a gap presented as continuous.
    #[tokio::test]
    async fn a_slow_reader_past_its_budget_is_resynced_by_a_snapshot() {
        let served = served("slow", &storm_for(6));
        let mut client = served.open().await;

        // A trickle: well below the daemon's delivery rate, well above zero.
        let trickle_until = Instant::now() + Duration::from_secs(5);
        while Instant::now() < trickle_until {
            let _ = client.next(Duration::from_millis(50)).await;
            tokio::time::sleep(Duration::from_millis(40)).await;
        }
        let done = client.read_until(DONE, Duration::from_secs(20)).await;

        assert!(done, "the storm's end never arrived (eof {})", client.eof);
        assert_healthy(&client, "the slow reader");
        assert!(
            client.snapshots >= 2,
            "the reader was never resynced ({} snapshots)",
            client.snapshots
        );
        assert_eq!(client.visible_cells(), served.authoritative_cells());
    }

    /// Regression 4. One stalled viewer leaves the other synchronized.
    #[tokio::test]
    async fn one_stalled_viewer_leaves_the_other_synchronized() {
        let served = served("two", &storm_for(6));
        let mut stalled = served.open().await;
        let mut healthy = served.open().await;

        assert!(stalled.read_for(Duration::from_millis(300)).await > 0);
        let done = healthy.read_until(DONE, Duration::from_secs(20)).await;

        assert!(done, "the healthy viewer never saw the storm end");
        assert_healthy(&healthy, "the healthy viewer");
        eprintln!(
            "two viewers: the healthy one was resynced {} times",
            healthy.snapshots
        );
        assert_eq!(healthy.visible_cells(), served.authoritative_cells());
        assert!(
            stalled.drained_to_eof(Duration::from_secs(3)).await,
            "the stalled viewer is still attached"
        );
    }

    /// Regression 5. A writer blocked on its socket holds neither the read
    /// loop nor the runtime: the stalled client's own resize still reshapes
    /// the session, another client's input is still echoed, and the PTY keeps
    /// flowing — all inside the deadline, while the writer is still stuck.
    #[tokio::test]
    async fn a_stalled_writer_holds_neither_the_read_loop_nor_the_runtime() {
        let mut served = served("control", &storm_for(30));
        let quiet = served.another("cat");
        let mut stalled = served.open().await;
        let mut healthy = served.open().await;
        let mut other = served.open_other(quiet).await;

        assert!(stalled.read_for(Duration::from_millis(300)).await > 0);
        tokio::time::sleep(Duration::from_millis(500)).await;
        // The writer is stuck by now and its deadline has not run out.
        stalled
            .send(
                FrameKind::Resize,
                super::encode_geometry(PtyGeometry::expect_valid(30, 100)),
            )
            .await;
        other.send(FrameKind::Input, b"CTRL-MARK\n".to_vec()).await;

        let echoed = other.read_until("CTRL-MARK", Duration::from_secs(3)).await;
        let reshaped = tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if served.handle.geometry().ok().and_then(Result::ok)
                    == Some(PtyGeometry::expect_valid(30, 100))
                {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .is_ok();
        let flowing = healthy.read_for(Duration::from_millis(500)).await;

        assert!(
            echoed,
            "another client's input was not echoed while a writer was stalled"
        );
        assert!(
            reshaped,
            "the stalled client's own resize did not reach the session"
        );
        assert!(flowing > 0, "PTY ingestion stopped behind a stalled writer");
        assert!(!healthy.eof && healthy.errors.is_empty());
    }

    /// Regression 6. A run that ends with output still queued for a slow
    /// viewer delivers it before the channel ends — a process's last words are
    /// never superseded.
    #[tokio::test]
    async fn final_output_reaches_a_slow_viewer_before_the_channel_ends() {
        let served = served(
            "final",
            &format!("yes '{STORM_LINE}' | head -c 1000000; printf 'FINAL-MARK'"),
        );
        let mut client = served.open().await;

        // Slow enough that the run finishes with most of its output queued,
        // fast enough to stay inside the byte budget.
        let mut seen = false;
        let deadline = Instant::now() + Duration::from_secs(20);
        while Instant::now() < deadline {
            match client.next(Duration::from_millis(50)).await {
                None => break,
                Some(None) => {}
                Some(Some(frame)) => {
                    if String::from_utf8_lossy(&frame.payload).contains("FINAL-MARK") {
                        seen = true;
                        break;
                    }
                }
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }

        assert!(
            seen,
            "the run's final output never reached the viewer (eof {})",
            client.eof
        );
        assert!(client.errors.is_empty(), "{:?}", client.errors);
    }
}
