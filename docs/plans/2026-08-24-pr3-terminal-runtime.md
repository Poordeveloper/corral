---
status: active
class: C
writes: [corrald, corral-protocol, corral-client, corral, corral-state, scripts-ci, canonical-docs]
reads: [corral-core, corral-rendezvous, docs/adr/0003-terminal-snapshot-format.md, docs/decisions/2026-08-24-adr3-terminal-snapshot-acceptance.md]
---

# PR3 — PTY ownership, the authoritative VT, and the terminal channel

## Goal

`corrald` owns PTYs, processes, and the authoritative terminal state; a
client subscribes, receives a snapshot @ seq N, replays sequenced deltas,
and survives resize and resync under ADR 0003's budget rules. `corral new
-- bash`, `corral attach`, detach/reattach work end to end (ROADMAP §3).

## Non-goals

No TUI (PR4), providers/hooks (PR5+), discovery (PR7), attention (PR8),
remote. No live-handoff upgrade preservation: M1 crash semantics are "no
survival guarantee + no-lying reconciliation" (ledger row 3). No history
backfill, no persisted scrollback, no ACK/credit flow, no viewport
claiming (ADR 3 §Not decided here). The lease seam is advisory — nothing
is enforced. **Zero durable schema/event diff**: PR2's accepted event set
(`SessionCreated`, `RunStarted`, `RunEnded`, `RunAttached`,
`RunDetached`) covers every fact PR3 produces.

## Existing owner / architecture involved

ARCHITECTURE §3 terminal model and ADR 0003 D1–D9 are accepted; this plan
implements them and decides only what ADR 3 explicitly left to
implementation (channel mechanics and framing). PR1 owns
endpoint/hello/lifecycle; PR2 owns the store and events. New domain nouns
enter the glossary in the same change.

## Design

1. **PTY runtime** (`corrald::runtime`): spawn/supervise via
   `portable-pty` — alternatives: hand-rolled `rustix::pty` + `pre_exec`
   (needs a Corral unsafe boundary crate and an ADR naming it, and we own
   controlling-terminal/EIO platform subtleties) or `nix` (same unsafe
   surface). The maintained WezTerm crate keeps Corral at
   `#![forbid(unsafe_code)]` everywhere. Exit reaping → `RunEnded` with
   authoritative status. Startup reconciliation: runs recorded live under
   a dead daemon are re-verified and ended exited/unverifiable — never
   shown stale-running. Idle-exit: live runs hold the daemon busy. Clippy
   `disallowed-methods`: only the runtime module spawns PTYs.
2. **Terminal authority** (`corrald::terminal`): one `qwertty-term-vt`
   emulator per session (ADR 3 D1; alternatives measured by S1), fed by a
   per-session PTY reader; 4 MiB byte-counted retention, oldest-first
   discard (D7). Answers terminal queries (DA/DSR) when no client is
   attached, so unattached agents never stall. S1 did not test
   query-reply: verify what the emulator answers itself and fill only the
   gap. The PTY reader is never blocked by a slow subscriber.
3. **Snapshot encoder**: viewport + up to 2,000 most-recent rows
   (experience target), palette excluded (per-connection, D4), Corral
   emits OSC 2 for the tracked-but-unserialized title (D3), truncation
   metadata = included row count + truncated-before flag (D6). Budgets on
   the final encoded payload: over 1 MiB → trim oldest scrollback and
   report; viewport alone over 16 MiB → typed snapshot-too-large channel
   error, daemon healthy, never a partial viewport (D8). Sizing may
   estimate-then-verify.
4. **Terminal data channel**: a second connection to the same endpoint,
   never the semantic RPC channel. RPC `terminal.attach` returns a
   one-time token; the new connection's hello declares the data-channel
   role + token (additive hello field). Binary length-prefixed frames
   (kind, epoch, seq, payload) with an explicit unknown-kind rule; the
   channel's frame ceiling derives from the 16 MiB snapshot ceiling —
   deliberately not RPC's 1 MiB `MAX_FRAME_BYTES`. Kinds: Snapshot,
   Delta (raw PTY bytes, unmodified), Input, Resize, ResyncRequest,
   ChannelError. Palette rides the subscription start.
5. **Epoch and resync**: client Resize frame → `TIOCSWINSZ` + emulator
   reflow → new epoch + fresh snapshot at the new geometry; pending
   resizes coalesce; the attaching client's geometry drives an initial
   resize. Gap, decode failure, or overflow → the client discards
   incremental state and requests a snapshot — the only recovery path.
6. **RPC additions** (additive, protocol 1): `session.new` (argv, cwd →
   session id + run id), `terminal.attach`. `session.list` elements get
   their first real shape — session id, title, execution state — the
   shape PR2 deliberately left unassigned until a producer and consumer
   existed. Absent fields are never read as known negatives.
7. **Advisory lease seam**: corrald records the current attach holder
   (exclusive-write intent), reports it, enforces nothing. Attach/detach
   append `RunAttached`/`RunDetached`.
8. **CLI**: `corral new -- <cmd>` creates and attaches; `corral attach
   <id-prefix>` reattaches; detach chord `Ctrl-\` (0x1C) intercepted
   client-side, no literal escape in M1. The CLI client is a byte pipe:
   snapshot + delta bytes to the local tty (the user's terminal is the
   replica; it encodes input per the mode bytes that passed through),
   local input → Input frames, SIGWINCH → Resize. `corral-client` gains
   data-channel support.
9. **Fuzz (D9)**: `cargo-fuzz` target on the emulator ingest path;
   recorded pre-merge campaign → `docs/evidence/pr3-terminal-fuzz-*.md`;
   deterministic corpus regression test inside `./scripts/verify`;
   scheduled deep-fuzz job calls a repository script (CI never owns
   verification semantics).

## Interfaces or persistence changed

Wire: two additive RPC methods, one additive hello field, the first
session.list element shape, and a new binary framing for the data channel
— all pre-release, renumbering legal, future-input coverage required.
Persistence: none. `corral-state` gains at most read accessors for
reconciliation (live-run listing); the schema gate will still route any
`corral-state` diff to human eyes, correctly. Dependencies added:
`qwertty-term-vt` (ADR 3 D1), `portable-pty` (Design 1); licenses through
cargo-deny.

## Failure / unknown states

Child exit → authoritative `RunEnded`; unreapable exit ends unverifiable.
PTY EIO is an ordinary end, not an error path. Daemon restart →
reconciliation; formerly-live sessions report exited/unverifiable, never
stale-running. Data-channel loss ≠ detach intent ≠ session end; closing
the CLI leaves the session running. Slow client: bounded per-subscriber
delta buffering; overflow drops that subscriber's incremental state and
forces resync — the PTY and other subscribers are unaffected. Viewport
over ceiling: typed failure, session and daemon live on. `session.new`
under store refusal answers `busy` (PR2 exception). Unknown frame kinds
and unknown fields follow the declared compatibility behaviour.

## Tests

- Integration (AGENTS-required): new → attach → output visible → detach →
  reattach → exit → state reflects it; daemon restart reconciliation;
  child crash; client disconnect mid-stream.
- Snapshot contract fixtures: replaying a snapshot into a fresh parser
  reproduces the daemon's screen across S1's dimensions — alternate
  screen, scrollback, cursor state, wide chars, charsets, and the
  Corral-emitted OSC 2 title.
- Budgets: oldest-first trim with honest metadata; the approved
  large-geometry/styling case lands well under the ceiling (D8); a
  synthetic over-ceiling viewport yields the typed failure and a healthy
  daemon.
- Epochs: resize mid-stream → new epoch + snapshot at the new geometry;
  coalescing; stale-epoch deltas are discarded by the client.
- Byte fidelity: delta frames byte-identical to PTY output (no munging).
- Unattached DA/DSR answered.
- Wire future-input: unknown frame kind, unknown fields, token reuse
  refused, data channel without token refused.
- D9 layer 2: corpus regression in `verify` — every corpus file ingests
  without panic in bounded time.
- Lifecycle failures per AGENTS §Tests: detach, disconnect, restart,
  crash, unverifiable exit.

## Definition of done

- Design 1–9 landed; `./scripts/verify` green on the final tree.
- The fuzz campaign record exists in `docs/evidence/` with every field
  the acceptance requires (target, SHA, tool/config, executions,
  platform, sanitizers, crashes, reproducers, disposition, result).
- Zero durable schema/event diff on the final tree.
- Glossary rows: snapshot epoch, terminal data channel, attach lease,
  snapshot budgets.
- Two independent fresh-context reviews before merge — one contract
  conformance against ADR 0003/ARCHITECTURE §3, one adversarial — fixes
  reviewed too (PR2 precedent).
- Plan moves to done/ on land.

## Plan Size Justification

One coherent semantic scope: every design item exists to uphold a single
invariant — corrald owns the terminal truth and a client can always
reconstruct it. The chain PTY → emulator → snapshot → channel → replica
is only provable end to end; splitting it ships a PTY nobody can see or a
snapshot nothing produces. The diff will exceed the normal staging
threshold: commits are staged by design item, and fixtures/corpus files
are mechanical content evaluated separately (AGENTS §Change size).
