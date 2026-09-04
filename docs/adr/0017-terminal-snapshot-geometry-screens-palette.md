---
status: proposed
read_when:
  - building or changing a client that keeps its own terminal replica (Desktop, Mobile, Web)
  - changing what a terminal snapshot carries, or the frames around it
  - adding a terminal frame kind, or deciding what an old client does with one it does not know
  - a viewer that did not ask for a resize receives a new epoch
---

> Proposed 2026-09-05, commissioned by `docs/decisions/2026-09-04-pr9-spike-grill.md`
> Q7. Three protocol-completeness requirements the PR9 replica spike
> exposed (`docs/references/2026-09-04-pr9-gpui-integration-spike.md`,
> findings S3, S4, S5). Extends ADR 0003; changes none of its decisions.
> Compatibility-facing Class C: nothing below merges before acceptance.
> Until then the daemon and the TUI behave as ADR 0003 alone describes.

# ADR 0017 — Terminal snapshot geometry, dual-screen state, and palette transport

## Context

ADR 0003 fixed the terminal wire as an ANSI snapshot at a position plus
sequenced raw deltas, resync by snapshot only, a new epoch per reshape. Its
only client so far is the TUI, whose replica is the host terminal: a real
terminal already has a size, keeps its own primary screen behind an
alternate one, and applies OSC palette changes as they pass through. The
Desktop's replica is an emulator inside the Desktop process, and the spike
measured three things such a replica cannot learn from the wire as it is:

- **S5 — geometry.** A `Snapshot` frame carries no rows or columns. A
  viewer that requested the resize knows the geometry; any other viewer of
  the same run receives the new epoch's snapshot (the daemon pushes one to
  every viewer) and can only guess its size. A replica built at the wrong
  size renders a full-screen TUI as garbage until the next resize it asks
  for itself.
- **S3 — the primary screen behind an active alternate.** The snapshot is
  the *active* screen. Attached while a full-screen program runs, a replica
  holds only the alternate screen; when the program exits (`?1049l`) the
  replica restores a blank primary where the daemon has the shell. The TUI
  never saw this because the host terminal restores *its* saved screen —
  which is also wrong, only less visibly: it is whatever the host had
  before attach, not the daemon's.
- **S4 — the palette.** ADR 0003 D4 keeps the palette out of the snapshot
  and promises it per connection, but no frame carries it. Palette changes
  a program makes after a viewer attached do reach it (they are bytes in the
  deltas); a viewer attaching afterwards renders those colours from a
  default palette.

Constraints that bind every decision below: mixed client/daemon versions
are normal (`AGENTS.md` §Protocol); unknown frame kinds are skippable and
already carried (`FrameKind::Unknown`); a client never reinterprets an
absent field as a known value; wire numbers are not yet permanent
(`STORAGE_EPOCH` is `dev`), but each kind assigned here is assigned once.

## D1 — Geometry is a frame of its own, sent before every snapshot

A new frame kind, `Geometry` (number 7), daemon → client, payload the four
big-endian bytes `rows, cols` a `Resize` already uses, stamped with the
epoch and sequence of the snapshot it precedes. The daemon sends it
**immediately before every `Snapshot`** on a connection — initial attach,
resync, reflow — so a replica is always built at the size the snapshot was
minted for, whoever caused the epoch.

Not a field inside the snapshot payload: the payload is opaque ANSI a
client writes straight to its replica, and an old client would write four
bytes of binary into its terminal. Not a reuse of `Resize`: that kind means
"what the client wants", this means "what the screen is"; one number with
two meanings by direction is exactly the ambiguity ADR 0003 D8 refused.

**Old client, new daemon.** The TUI ignores kinds it does not act on
(`attach.rs::apply` writes nothing for `Input`, `Resize`, `ResyncRequest`,
and skips `Unknown`); a `Geometry` frame reaches it as `Unknown(7)` and is
skipped. Behaviour unchanged.

**New client, old daemon.** No `Geometry` arrives before the snapshot. The
client must not infer geometry from a frame's absence: the daemon's hello
advertises `terminal.geometry.v1`, and a client that does not see it keeps
today's rule — its replica takes the size it last asked for. A client that
sees the capability and then a `Snapshot` without a preceding `Geometry` in
the same epoch treats the stream as desynchronised and requests a resync.

## D2 — A snapshot carries both screens when the alternate is active

While the alternate screen is active, the snapshot payload is: the primary
screen's contents and cursor, then `\x1b[?1049h`, then the alternate
screen's contents, then the extras ADR 0003 D3 lists. A replica that later
sees `?1049l` in a delta restores the primary screen the daemon has.

ADR 0003 D3's list gains one item: *the primary screen preserved behind an
active alternate, and which screen is active*. D7's budgets apply to the
two screens together: the primary screen counts against the row target,
history behind it is omitted before the primary screen is, and the primary
screen is omitted — recorded as `history_truncated_before`-style fact, not
silently — before the alternate is, because the alternate is what the
person is looking at.

No capability, no new frame: the payload is still ANSI every client already
replays. An old client attached to a new daemon simply has a correct
primary screen for the first time.

**Mechanism note, not a decision.** qwertty-term-vt 0.4.0 formats the
*active* screen (`Terminal::format_content`); its per-screen entry point is
private. The implementation either lands a one-line upstream exposure or
formats the primary screen through the same path with the screen set
switched for the duration of the mint on the screen thread that owns it.
Either is contained in `runtime/snapshot.rs`; neither changes this
decision.

## D3 — The palette rides on its own frame, only when it is not the default

A new frame kind, `Palette` (number 8), daemon → client, payload the ANSI
the formatter's `palette` extra already produces (OSC 4 per changed entry
and OSC 10/11 for the dynamic foreground and background), stamped like
`Geometry`. Sent **before a `Snapshot`**, after `Geometry`, **only when the
screen's palette differs from the built-in default**; and again whenever a
later snapshot's palette differs from what this connection was last sent.
A connection therefore carries the palette at most once per change, and
most sessions — which never touch it — carry nothing, which is what D4 of
ADR 0003 wanted when it measured 5 531 bytes against a five-byte screen.

Palette changes between snapshots need no frame: they are bytes in the
deltas, and a replica applies them as the daemon did.

**Old client.** `Unknown(8)`, skipped; the host terminal keeps its own
palette, as today. **New client, old daemon.** No frame; the hello lacks
`terminal.palette.v1`; the replica keeps the default palette, as today.

## D4 — Order, epochs, and resync

Within one epoch, geometry is constant and `Geometry` precedes the first
`Snapshot` of that epoch on each connection; a resync inside an epoch
repeats it (cheap, and it lets a client rebuild from the snapshot alone).
The sequence on a connection is therefore always
`[Geometry] [Palette]? Snapshot Delta*`, with `Geometry` and `Palette`
stamped identically to the `Snapshot` they precede. A client that receives
`Geometry` or `Palette` for an epoch it has left discards them, as it
discards such a snapshot. A `ResyncRequest` is answered by the full
prefix, never by a bare `Snapshot`.

## D5 — Compatibility contract

- Kinds 7 and 8 are assigned here and are never reused, renumbered, or
  given another meaning; the future-input fixtures gain both, and a fixture
  that decodes them on a build that does not know them asserts
  `Unknown(7)`/`Unknown(8)` and skippability.
- Capabilities `terminal.geometry.v1` and `terminal.palette.v1` in the
  daemon's hello name the daemon's behaviour; a client advertises nothing
  and needs nothing — the frames are skippable.
- The TUI changes only by ignoring two more kinds it already skips. The
  Desktop is the first client that acts on them, and acts on their absence
  as D1/D3 say.
- No durable state: geometry, screens and palette are live runtime facts,
  never persisted (`AGENTS.md` §Durable state).

## Rejected

- Geometry inside the snapshot payload (binary into an old client's
  terminal); geometry in the hello (per connection, but epochs reshape).
- The palette inside every snapshot (ADR 0003 D4's measured overhead on the
  recovery path).
- Leaving the primary screen to the client ("request a resync when you see
  `?1049l`"): the resync arrives after the program exited, and the daemon's
  own primary screen — the shell the person returns to — is exactly what
  the snapshot then shows; the gap in between is a blank screen the person
  sees on every exit, and a client-side heuristic on a mode sequence is a
  second interpretation of the stream the wire exists to avoid.

## Tests the acceptance needs

- Protocol: encode/decode of kinds 7 and 8; unknown-kind fixtures on the
  pre-ADR decoder; capability names present in the hello.
- Fidelity (`snapshot_fidelity_tests.rs`): the alternate-screen scenarios
  extend past `?1049l` and assert the primary screen; a palette scenario
  asserts palette entries through the `Palette` frame; a resize-from-the-
  other-viewer scenario builds the replica from `Geometry` alone.
- Channel: two viewers, one resizes, the other receives `Geometry` then
  `Snapshot` with matching stamps and rebuilds at the new size.

## Evidence this stands on

`docs/references/2026-09-04-pr9-gpui-integration-spike.md` scenario 1
(S3, S4 measured cell for cell; the snapshot payload bytes quoted) and
scenario 6 (a second viewer's resize pushes a snapshot to the first, S5).
