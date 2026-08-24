---
status: done
class: B
writes: [corrald, corral-protocol, corral-client, corral, corral-state, scripts-ci, canonical-docs]
reads: [corral-core, corral-rendezvous, docs/adr/0003-terminal-snapshot-format.md, docs/decisions/2026-08-24-adr3-terminal-snapshot-acceptance.md, docs/decisions/2026-08-24-pr3-plan-grill.md]
---

# PR3 — PTY ownership, the authoritative VT, and the terminal channel

> **Correction, added after this plan was marked done.** Design 7 did not
> land, and neither did the persistence behind it: `corrald` performed no
> durable write at all, and §Interfaces below contradicts Design 7 by saying
> "Persistence: none". Four review rounds did not catch it, because a review
> compares a diff to a goal and a design item that was never written is not in
> the diff. It landed instead in
> `docs/plans/done/2026-08-24-pr3-durable-run-lifecycle.md`, restated as the
> advisory *attachment* seam it can honestly be: PR3 has no client identity, so
> "the attach holder" was never implementable. The "attach lease" glossary row
> this plan's Definition of Done required landed there too, as **Attachment
> seam**.

## Goal

`corrald` owns PTYs, processes, and the authoritative terminal state; a
client subscribes, receives a snapshot @ seq N, replays sequenced deltas,
and survives resize and resync under ADR 0003's budget rules. `corral new
-- bash`, `corral attach`, detach/reattach work end to end (ROADMAP §3).
Every plan-level decision below was ruled in the founder grill
(`docs/decisions/2026-08-24-pr3-plan-grill.md`); implementation
materializes those rulings and the fresh review checks fidelity to them.

## Non-goals

No TUI (PR4), providers/hooks (PR5+), discovery (PR7), attention (PR8),
remote. No live-handoff upgrade preservation: M1 crash semantics are "no
survival guarantee + no-lying reconciliation" (ledger row 3). No history
backfill, no persisted scrollback, no ACK/credit flow, no viewport
claiming (ADR 3 §Not decided here). The lease seam is advisory — never a
server-side correctness gate. **Zero durable schema/event diff** —
confirmed by the Q9 capability check, not assumed: PR2's accepted
vocabulary already expresses every fact PR3 produces. `corral new` is the
managed-runtime walking-skeleton CLI; it does not define the final M1
"New Session" product UX (grill Q7).

## Existing owner / architecture involved

ARCHITECTURE §3 and ADR 0003 D1–D9 are accepted; this plan implements
them plus the channel mechanics ADR 3 left to implementation. PR1 owns
endpoint/hello/lifecycle; PR2 owns the store and events. New domain nouns
enter the glossary in the same change.

**The Q5/Q9 invariant, quoted into code and tests verbatim:**
*"RunEnd::Unverifiable closes Corral's managed episode; it never claims
that the OS process exited."* Loss of PTY ownership terminates Corral's
ability to manage the runtime; it does not prove the process exited.

## Design

1. **PTY runtime** (`corrald::runtime`), backend `portable-pty`. Owner
   boundary frozen (grill Q1): portable-pty owns PTY allocation,
   controlling-terminal setup, spawn plumbing, PTY I/O, resize, and
   platform EIO/EOF mechanics; **Corral owns managed-runtime lifecycle
   semantics** — process-group identity capture, child/reaper
   bookkeeping, descendant teardown policy (via `rustix`'s safe
   process-group kill, not `Child::kill`), detach semantics,
   daemon-shutdown policy, Run lifecycle truth. **Prerequisite gate,
   before Design 1 is built on:** a focused spawn-semantics compatibility
   test — nonexistent executable, non-executable target, invalid cwd,
   normal executable, normal child exit, PTY resize, process-group
   identity, Linux + macOS. Its load-bearing assertion: Corral can
   distinguish *failed exec* from *successful exec followed by process
   exit* (upstream has an open exec-failure report). If it cannot: STOP —
   pin a fixed upstream, add a bounded workaround, or reopen the backend
   choice. Never demoted to a later bug. Startup reconciliation and
   idle-exit: see Failure states. Clippy `disallowed-methods`: only the
   runtime module spawns PTYs.
2. **Terminal authority** (`corrald::terminal`): one `qwertty-term-vt`
   emulator per session (ADR 3 D1), fed by a per-session PTY reader;
   4 MiB byte-counted retention, oldest-first discard (D7). Answers
   terminal queries (DA/DSR) when no client is attached. S1 did not test
   query-reply: verify what the emulator answers itself, fill only the
   gap. Child environment policy: `TERM=xterm-256color` — a property of
   each managed Run, distinct from the daemon's own
   environment-independence; revisit only if the emulator measurably
   emits beyond that contract.
3. **Snapshot encoder**: viewport + up to 2,000 most-recent rows
   (experience target), palette per connection (D4), Corral emits OSC 2
   for the tracked-but-unserialized title (D3), truncation metadata =
   included row count + truncated-before flag (D6). Budgets on the final
   encoded payload: over 1 MiB → trim oldest scrollback and report;
   viewport alone over 16 MiB → typed SnapshotTooLarge, daemon healthy,
   never a partial viewport (D8). Sizing may estimate-then-verify.
4. **Terminal data channel** (grill Q2, human-gated): RPC
   `terminal.attach` issues a one-time token; the client opens a second
   connection to the **same canonical endpoint** (never a second
   namespace), completes the existing bootstrap hello declaring the
   terminal-data role + token, and the connection transitions **one way,
   permanently** into terminal binary framing — that fd never carries
   RPC again, and no generic multiplexing framework exists. Frames:
   length-prefixed (kind, epoch, seq, payload), explicit unknown-kind
   rule, frame ceiling derived from the 16 MiB snapshot ceiling — not
   RPC's 1 MiB `MAX_FRAME_BYTES`. Kinds: Snapshot, Delta (raw PTY bytes,
   unmodified), Input, Resize, ResyncRequest, ChannelError. Token: local
   bearer capability under the same-user UDS boundary; 128-bit CSPRNG;
   TTL 30 s from issuance; bound to `CorralSessionId` **and** the
   concrete Run/terminal runtime identity (a Session outlives Runs — a
   stale token must never attach to a later terminal); not bound to
   snapshot epoch or the issuing RPC connection's lifetime. Redemption
   is one atomic step (validate token + TTL + runtime binding, mark
   consumed, transition the connection); a consumed token never
   resurrects — if the initial snapshot then fails, the attach attempt
   fails and the client requests a fresh `terminal.attach`.
5. **Epoch, resize, multi-viewer** (grill Q6): geometry is shared
   authoritative session state, never per-viewer. Multiple simultaneous
   data channels are allowed; each attach takes an independent snapshot
   and joins the same authoritative delta stream; all attached clients
   may submit input. Last **explicit local** resize wins → `TIOCSWINSZ` +
   reflow → new epoch + fresh snapshot; pending resizes coalesce; the
   attaching client's geometry drives an initial resize. Anti-feedback
   invariant: a client sends Resize only because its own desired
   geometry changed (or on initial attach) — **never as an echo of a
   received server geometry or epoch**. No smallest-common-geometry, no
   primary viewer, no resize ownership, no enforced lease in M1. Gap,
   decode failure, or desync → the client discards incremental state and
   requests a snapshot — the only recovery path.
6. **RPC additions** (grill Q3, human-gated): `session.new` (argv, cwd →
   session id + run id), `terminal.attach`. First `session.list` element
   shape: `session_id` (identity); `title` — an optional,
   **non-authoritative display label chosen by Corral** that clients
   must never parse or use for identity/control, defaulting to
   `basename(argv[0])` and **never argv concatenation** (argv carries
   tokens, URLs, customer identifiers); `execution_state` ∈ {Running,
   Exited, Unknown}. Unknown means "Corral cannot currently make a
   reliable execution-state claim" — the execution dimension's own
   value, not PR8 attention vocabulary, not assurance vocabulary (a
   future assurance surface is an additive separate field). Absent
   fields are never known negatives.
7. **Advisory lease seam**: corrald records the attach holder, reports
   it, enforces nothing. Attach/detach append `RunAttached`/
   `RunDetached`.
8. **CLI** (grill Q4, Q7): `corral new -- <cmd>` creates and immediately
   attaches — no `--detached`, no `--name`; the session id prints to
   stderr before entering interactive mode (stdout carries no control
   metadata). Detach chord `Ctrl-\` (0x1C): unconditionally intercepted
   by the interactive client, never forwarded, no literal escape, no
   keybinding configuration. Conscious M1 limitation, recorded: a
   literal 0x1C cannot reach the child through Corral's interactive
   attach. Future configurability is client-side evolution, no wire
   change. The CLI client is a byte pipe: snapshot + delta bytes to the
   local tty, local input → Input frames, SIGWINCH → Resize.
   `corral-client` gains data-channel support.
9. **Fuzz (D9)**: `scripts/fuzz-terminal` owns fuzz target/config
   semantics; the scheduled CI job only calls it — nightly, 30-minute
   bound, evidence amplification never merge gate; failure → P1,
   minimized reproducer → the permanent deterministic corpus inside
   `./scripts/verify`; material findings archived per the
   `docs/evidence/` admission rules; no nightly success records. The
   recorded pre-merge campaign lands at
   `docs/evidence/pr3-terminal-fuzz-*.md`.

## Interfaces or persistence changed

Wire — all human-gated as compatibility-facing commitments, founder
decisions recorded in the grill: `terminal.attach` + token semantics, the
ClientHello terminal-data role extension, the one-way framing transition,
and the first concrete `session.list` element shape. Persistence: none.
`corral-state` gains at most read accessors for reconciliation; the
schema gate still routes any `corral-state` diff to human eyes.
Dependencies: `qwertty-term-vt` (ADR 3 D1) and `portable-pty` (grill Q1)
— both human-gated as new third-party dependencies; licenses through
cargo-deny.

## Failure / unknown states

Child exit observed → `RunEnded(Exited(cause))`, authoritative. Daemon
crash → on restart, reconciliation maps: PTY/runtime ownership
irrecoverably lost → the managed episode cannot continue →
`RunEnded(Unverifiable)` — and the user-visible claim is
`execution_state = Unknown`, **never Exited** (grill Q5/Q9). These two
facts do not conflict: Run lifecycle says Corral can no longer manage the
episode; process fate stays Unknown. No process is killed by
reconciliation; no `Orphaned`/`OwnershipLost` discriminant is added —
PR2's vocabulary already expresses the fact, and a second representation
would be the real violation. `last_known_pid` (grill Q10): runtime
diagnostic only — process-memory of the spawning daemon, never
persisted, never reconstructed after restart, never identity evidence,
never a kill target; reconciliation reads no pid, probes no pid, infers
neither survival nor death. Idle-exit: live runs hold the daemon busy.
PTY EIO is an ordinary end. Data-channel loss ≠ detach intent ≠ session
end; closing the CLI leaves the session running. Slow subscriber: per-
subscriber encoded queue bounded at 4 MiB (per subscriber, not a shared
pool); overflow marks the subscriber desynchronized and closes that data
channel — **never drop an interior delta and keep streaming** — the
client re-attaches for a fresh snapshot. A slow subscriber never
backpressures the PTY reader, the authoritative VT, or other
subscribers. `session.new` under store refusal answers `busy` (PR2
exception). Unknown frame kinds and fields follow declared compatibility
behaviour.

## Tests

- **Spawn gate (prerequisite)**: the Design 1 compatibility suite; its
  exec-vs-exit distinction assertion is load-bearing.
- Integration: new → attach → output → detach → reattach → exit;
  daemon restart reconciliation asserts `RunEnd::Unverifiable` **and**
  `execution_state == Unknown`, never Exited; child crash; client
  disconnect mid-stream.
- Snapshot contract fixtures: replaying a snapshot into a fresh parser
  reproduces the daemon's screen across S1's dimensions, including the
  Corral-emitted OSC 2 title.
- Budgets (two distinct jobs, grill Q8): the approved representative
  extreme — 500×140, per-cell varying truecolor fg/bg, attribute
  variation, wide characters, actual encoder, actual encoded payload —
  asserts < 8 MiB, permanent regression (evidence a realistic extreme
  sits far under the ceiling — **not** a proof all viewports do); a
  synthetic > 16 MiB viewport asserts typed SnapshotTooLarge, no
  partial-success, daemon healthy. Oldest-first trim with honest
  metadata.
- Epochs and viewers: resize mid-stream → new epoch + snapshot at new
  geometry; coalescing; stale-epoch deltas discarded; two viewers with
  different geometries converge to the authoritative geometry; no client
  echoes a resize it did not locally originate.
- Detach chord: typed 0x1C detaches; pasted/stream 0x1C detaches; detach
  while the child is in raw mode; the child continues after detach; no
  0x1C ever reaches the PTY writer.
- Subscriber isolation: a stalled subscriber's queue overflows → that
  channel closes desynchronized; PTY reader, VT state, and other
  subscribers proceed unaffected.
- Byte fidelity: delta frames byte-identical to PTY output. Unattached
  DA/DSR answered.
- Wire future-input: unknown frame kind, unknown fields; token: atomic
  single redemption under concurrent attempts, expiry refused, no
  resurrection after a failed initial snapshot, unaffected by RPC
  connection death, refused for a later Run of the same Session.
- D9 layer 2: corpus regression in `verify`.
- Lifecycle failures per AGENTS §Tests: detach, disconnect, restart,
  crash, unverifiable state.

## Definition of done

- **Classification (grill ruling)**: high-consequence Class B. Founder
  decisions are resolved by the grill — implementation crosses no open
  decision boundary — but the PR is `HUMAN_REVIEW_REQUIRED` (new
  third-party dependencies; `terminal.attach`/data-channel protocol
  additions; ClientHello role extension; first `session.list` shape) and
  therefore human-merged. No autonomous merge.
- Design 1–9 landed; `./scripts/verify` green on the final tree; the
  spawn gate passed before the backend was relied on.
- The fuzz campaign record exists in `docs/evidence/` with every field
  the ADR 3 acceptance requires.
- Zero durable schema/event diff on the final tree (Q9).
- Glossary rows: snapshot epoch, terminal data channel, attach lease,
  snapshot budgets.
- Fresh-context reviews before merge — one contract conformance against
  ADR 0003/ARCHITECTURE §3 **and the grill record** (fidelity to
  rulings, per the founder: review materialization, do not re-grill),
  one adversarial; fixes reviewed too.
- Plan moves to done/ on land.

## Plan Size Justification

One coherent semantic scope: every design item upholds a single
invariant — corrald owns the terminal truth and a client can always
reconstruct it. The chain PTY → emulator → snapshot → channel → replica
is only provable end to end; splitting it ships a PTY nobody can see or
a snapshot nothing produces. The growth over the ~150-line target is the
grill's rulings materialized inline — the alternative is oral history,
which the plan rule exists to forbid. The diff will exceed the normal
staging threshold: commits are staged by design item; fixtures/corpus
are mechanical content evaluated separately (AGENTS §Change size).
