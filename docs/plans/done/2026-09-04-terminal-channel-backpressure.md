---
status: done
class: B   # high-consequence: terminal data path, subscriber lifecycle, backpressure/resync (grill Q13)
writes: [crates/corrald/src/terminal_channel.rs, crates/corrald/src/terminal_channel_tests.rs]
reads: [crates/corrald/src/runtime/stream.rs, crates/corrald/src/runtime/session.rs, crates/corrald/src/connection.rs, crates/corral-protocol/src/terminal.rs, docs/adr/0003-terminal-snapshot-format.md, docs/decisions/2026-08-24-pr3-plan-grill.md, docs/decisions/2026-09-04-pr9-spike-grill.md, docs/references/2026-09-04-pr9-gpui-integration-spike.md]
---

# Terminal channel backpressure — one backlog authority per subscriber

## Goal

Close finding S6 of the PR9 spike: a terminal data channel closes under
sustained output. `terminal_channel::serve` queues outbound frames in an
8-slot `mpsc` and `try_send`s into it; one full queue is read as "this
client stopped reading" and the channel returns. At the daemon's delivery
ceiling (~8 700 deltas/s) the writer task falls eight frames behind on
ordinary jitter; nine of twelve sustained storms closed the channel, with
no `ChannelError` and no log line. The TUI attaches over the same channel.

Implement the semantics grill Q5/Q10 ruled: the per-viewer
`TerminalStream` delivery state with its 4 MiB budget is the **sole**
backlog authority; a dedicated subscriber writer owns the socket write half
and may await socket progress under a 2 s no-progress deadline; the client
read loop only reads. Three distinct outcomes: jitter → drain; backlog past
budget → explicit resync barrier and a fresh snapshot; no socket progress
for 2 s → disconnect. P1; precedes Q6 and the PR9 plan.

## Non-goals

No wire change: frame kinds, epochs, sequences, and the resync-by-snapshot
contract are untouched. No change to `TerminalStream::deliver`'s budget or
to the authoritative path in `session.rs`. No snapshot-format repair (Q6),
no protocol addition (Q7), no client change (TUI and the future Desktop
benefit unchanged).

## Existing owner / architecture involved

- `runtime/stream.rs` — `TerminalStream`: one `Attached` per viewer with an
  outbox (`mpsc`, 1 024 frames) and a `room` semaphore (4 MiB). `deliver`
  uses `try_acquire` + `try_send` only; a viewer over budget is marked
  `Desynchronised::QueueOverflow` and removed, so its receiver yields what
  was queued and then `None`. This is the accepted budget and it never
  blocks the authoritative path (pr3 plan grill: a slow subscriber never
  backpressures the PTY reader, the VT, or other subscribers; never drop an
  interior delta and continue as valid).
- `runtime/session.rs` screen thread — `terminal.consume` → `stream.advance`
  → `stream.deliver`; a geometry change opens a new epoch, which clears
  every viewer; a poisoned screen drops viewers. Unchanged.
- `terminal_channel.rs` — `serve` spawns a writer over an 8-slot queue,
  then `serve_frames` runs one `select!` over the viewer's deliveries and
  the client's frames, sending through `send` = `try_send`. Every
  `false` from `send` ends the channel. Resync, resize, and a stream that
  ended all re-`attach` and send a snapshot from inside that loop.
- `connection.rs:65-90` splits the `UnixStream` and calls `serve`; its
  signature stays.

## Design

**Two tasks, two halves.** `serve` spawns a *subscriber writer* task that
owns `OwnedWriteHalf`, the current `Attachment` (viewer, epoch, sequence)
and nothing else; `serve_frames` keeps the read half and the session-handle
lookups. The 8-slot queue, `Outbound`, and `OUTBOUND_FRAMES` are deleted.

**Writer loop.** Sends the initial snapshot, then selects over
`viewer.recv()` and a *control* receiver from the read loop. Control is an
unbounded channel carrying only `Resync`, `Error(String)` — one message per
client event, never PTY data — so the read loop never awaits it. Per
delivery: if the viewer is already closed (`Receiver::is_closed`: epoch
change, desynchronised, run over), re-attach first — `Some(fresh)` means
the queued deltas are superseded: discard them (the explicit barrier), send
the fresh snapshot, continue on the fresh viewer; `None` means the run is
gone: drain what is queued so the final output still reaches the client,
then end. A `None` from `recv` re-attaches the same way. `Resync` discards
the current queue and re-attaches (the client threw its state away, so the
pending deltas are worthless to it); `Error(text)` becomes a `ChannelError`
stamped with the writer's current epoch and sequence.

**No-progress deadline.** Every frame is written by a loop of
`timeout(NO_PROGRESS_DEADLINE, write(rest))`; a write that moves at least
one byte restarts the clock; zero bytes, an error, or a timeout ends the
writer. `NO_PROGRESS_DEADLINE = 2 s`, its own constant, documented as
initial operational policy and not a wire guarantee; `FLUSH_GRACE` stays a
separate constant with its separate meaning (how long a channel that ended
waits for its last frame). Time is not counted while there is nothing to
write.

**Read loop.** Decodes client frames as today. `Input` → `ask_session`.
`Resize` → `ask_session(resize)` only: the epoch change ends every viewer's
stream, so the writer re-attaches and sends the new snapshot by itself — no
second snapshot from the read side. `ResyncRequest` → control `Resync`.
Refusals → control `Error`. Client EOF or a decode error drops the control
sender; the writer finishes its current frame and returns; `serve` then
awaits it under `FLUSH_GRACE` as today.

**What ends a channel.** Client EOF or a bad frame; the writer's
no-progress deadline; the run gone after the final drain; a snapshot that
cannot be encoded (a `ChannelError`, then end — as today). A full viewer
queue never does.

## Interfaces or persistence changed

None on the wire or in durable state. `serve`'s signature is unchanged.

## Failure / unknown states

- A client that stops reading: its socket fills, `write` makes no progress,
  the writer ends at 2 s, the channel closes. Its viewer is dropped with the
  writer; the next `deliver` finds the outbox closed and retires it. The
  PTY reader, the VT, and other viewers never waited.
- A reading client that is slower than the output: its viewer crosses 4 MiB,
  `deliver` marks it desynchronised and removes it; the writer sees the
  closed viewer, discards the stale queue, sends a fresh snapshot. The client
  sees deltas up to N, then a `Snapshot` at a later position — the sequence
  jump ADR 0003's resync-by-snapshot already defines.
- A stalled writer while the same client sends `Input` or `Resize`: the read
  loop still reads and the session still acts on them; the writer's stall
  cannot reach it.
- A run that ends with output still queued: drained before the channel
  ends; the final screen is never discarded (today's behaviour, kept).
- Two writers racing on one epoch change: each re-attaches independently;
  attachments are per viewer, snapshots are stamped with their own position.

## Tests

Channel-level, in `terminal_channel_tests.rs`, over
`tokio::net::UnixStream::pair()` with a real `sh` session
(`session_tests::started`) inside a test `DaemonState`
(`connection_tests` builds one with `DaemonState::open`). The client side
decodes frames with `TerminalFrame::decode_from_daemon` and, where the
screen matters, feeds a qwertty replica and compares visible cells against a
fresh `handle.attach()` snapshot fed to another replica (cells only: the
cursor is Q6's).

1. **Healthy reader under a 10 s storm** (`yes` through the PTY): the
   channel stays attached, no `ChannelError`, sequences contiguous within an
   epoch, and the replica's cells equal the authoritative screen after the
   storm. Fails today within seconds.
2. **Client stops reading**: output flows, the client reads nothing for
   3 s; the daemon closes that channel within the deadline plus a margin,
   and a second viewer of the same run keeps receiving contiguous deltas.
3. **Slow reader crosses 4 MiB**: the client reads at a trickle; the daemon
   delivers deltas, then a `Snapshot`, never a `Delta` after a sequence gap;
   the replica rebuilt from that snapshot plus later deltas equals the
   authoritative screen.
4. **Two viewers, one stalled**: viewer A stops reading, viewer B reads;
   B's deltas stay contiguous and timely throughout A's stall; A is dropped
   after the deadline; B is not.
5. **Stalled writer, control continues**: with A's socket unread, A sends
   `Resize` and B sends `Input`; the session's geometry changes and B sees
   the echo — both before A's deadline expires — proving socket
   backpressure never reached the read loop or the runtime.
6. **Final output is drained**: a run that prints and exits while its
   viewer is slow still delivers the last bytes before the channel ends.

`stream_tests` already prove the budget owner; they stay. Timing
assertions use the deadline plus a margin (≤ 6 s) rather than exact
values; the storm test is the one long test and is budgeted as such.

## Definition of done

- `OUTBOUND_FRAMES`, `Outbound`, and the `try_send` path are gone; one
  backlog owner; the writer alone awaits the socket; the read loop never
  does.
- Tests 1–6 pass and 1 fails on the pre-fix code for the reason S6 names;
  `./scripts/verify` passes on the final tree.
- The spike's storm (9 s of `yes` at 80×24, 6 runs) through the spike
  harness or the TUI no longer closes the channel; recorded in the PR.
- PR body: `Class: B`, high-consequence; `Applicable escalation triggers:
  none` — implements the invariant the pr3 plan grill froze and grill
  Q5/Q10 ruled; review under the high-consequence owner rules.

## Plan Size Justification

Twenty lines over the target: one owner boundary (the channel's writer and
reader) and the six regressions grill Q10 mandated, each of which names a
distinct failure the fix must separate. Splitting the tests from the fix
would ship the fix without the evidence that it separates them.

## Closed 2026-09-05

Landed as designed, with two things the plan did not foresee, both
surfaced in the PR:

- **The frame backstop was the binding budget.** `SUBSCRIBER_QUEUE_FRAMES`
  was sized against 8 KiB deliveries; a PTY under sustained output hands
  the daemon about 1 KiB per read (the spike's 69 MB over 67 304 frames),
  so 1 024 frames made the effective budget one megabyte, not the four the
  policy states, and a reader that keeps up on average was resynced eight
  times in ten seconds. The backstop now sits at
  `SUBSCRIBER_QUEUE_BYTES / 256`, so any delivery of 256 bytes or more runs
  out of bytes first — `stream.rs`'s own stated intent, one line outside the
  plan's `writes:`, with a regression at the measured size.
- **A writer that ends, ends the channel.** With the write half owned by
  the writer, a client that stopped reading but kept its socket open would
  otherwise have held the read loop, and the attachment observation, for as
  long as it liked. `serve` now returns when either task ends.

The six regressions run the daemon on a reactor thread of its own, as in
production; a client sharing the daemon's thread starved the writer every
time it parsed a frame. Resync counts are printed, not asserted: a reader
that lags for a moment is resynced, which is the channel working. Checked
against the spike harness: six 9-second storms, 79 476–79 997 frames each,
no close, no resync.
