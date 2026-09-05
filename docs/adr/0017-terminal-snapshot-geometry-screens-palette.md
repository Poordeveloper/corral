---
status: accepted
read_when:
  - building or changing a client that keeps its own terminal replica (Desktop, Mobile, Web)
  - changing what a terminal snapshot carries, or the frames around it
  - adding a terminal frame kind, or deciding what an old client does with one it does not know
  - a viewer that did not ask for a resize receives a new epoch
---

> Accepted 2026-09-05. Proposed the same day, commissioned by
> `docs/decisions/2026-09-04-pr9-spike-grill.md` Q7; ruled and corrected
> in `docs/decisions/2026-09-05-adr-0017-grill.md` (Q2's budget order and
> Q3's send condition are the founder's, not the proposal's), and accepted
> once that record's Q5 check passed on the pre-ADR decoder. Three
> protocol-completeness requirements the PR9 replica spike exposed
> (`docs/references/2026-09-04-pr9-gpui-integration-spike.md`, findings
> S3, S4, S5). Extends ADR 0003; changes none of its decisions. The
> implementation is a standalone high-consequence Class B PR that precedes
> PR9 planning.

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

**Snapshot anchor.** `Geometry(E, N)`, `Palette(E, N)` when present, and
`Snapshot(E, N)` describe one recoverable state checkpoint: the terminal's
state at epoch `E`, position `N`. That is what "the same stamp" means.
Physical frame ordering on the transport is untouched — the stamp is the
snapshot's state position, not a transport frame number — and nothing here
weakens the existing sequencing invariant of the channel.

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
advertises `terminal.geometry.v1`, and a client that does not see it may
fall back to today's rule — its replica takes the size it last asked for —
as legacy behaviour only, never presented as daemon-confirmed geometry;
with several viewers on an old daemon that assumption can already be
stale. A client that sees the capability and then a `Snapshot` without the
`Geometry` of that snapshot point treats the stream as desynchronised and
protocol-incomplete: it does not install the snapshot as authoritative, and
requests a resync.

Invariant: every geometry-capable snapshot tells a fresh replica what size
of terminal state it is reconstructing.

## D2 — A snapshot carries both screens when the alternate is active

While the alternate screen is active, the snapshot payload is: the primary
screen's contents and cursor, then `\x1b[?1049h`, then the alternate
screen's contents, then the extras ADR 0003 D3 lists. A replica that later
sees `?1049l` in a delta restores the primary screen the daemon has.

ADR 0003 D3's list gains one item: *the primary screen preserved behind an
active alternate, and which screen is active*.

**What may degrade and what may not.** While the alternate screen is
active, both current viewports — the complete primary viewport and the
complete alternate viewport — and the active-screen identity are required
state of a successful snapshot; histories are optional. D7's budget
therefore degrades in this order: the oldest history behind the primary
screen first, then any other optional history under the accepted policy,
and the two viewports are retained. If the two viewports together cannot
fit under the hard ceiling, the snapshot fails with a typed error — the
existing `ViewportExceedsCeiling` shape, extended to say which screens —
the daemon stays healthy, and no partial dual-screen snapshot is called
successful. A snapshot that knowingly omitted the screen the next `?1049l`
will expose would be claiming a recoverability it does not have.

No capability, no new frame: the payload is still ANSI every client already
replays. An old client attached to a new daemon simply has a correct
primary screen for the first time.

**Mechanism.** qwertty-term-vt 0.4.0 formats the *active* screen
(`Terminal::format_content`); its per-screen entry point is private. The
primary screen is formatted on the screen thread that owns the emulator
as an implementation-local, formatting-only operation. It must not emit a
delta, advance the epoch or the state sequence, fire a resize or
screen-change callback, change the externally visible active screen, leave
cursor or mode state different after the mint than before, or be visible
to any subscriber; a guard restores the prior state so an early error or
panic cannot leave the wrong screen active. The regression compares the
authoritative terminal's state before and after a mint on every dimension
qwertty and Corral own. If the public screen switch cannot meet that, the
implementation uses a bounded local adapter exposing the formatter rather
than a semantically mutating switch. An upstream exposure remains a
desirable follow-up and never a correctness dependency.

## D3 — The palette rides on its own frame, only when it is not the default

A new frame kind, `Palette` (number 8), daemon → client, stamped like
`Geometry`. It is a **checkpoint of the effective palette** at the snapshot
point: the ANSI the formatter's `palette` extra produces (OSC 4 per entry,
OSC 10/11 for the dynamic foreground and background), and an explicit
reset form (OSC 104/110/111, or the default entries spelled out) when the
effective palette is the default and a reset is what the receiver needs.

**When it is sent.** Every connection begins at a defined baseline: the
default palette. For each snapshot bundle the daemon compares the current
effective palette — semantic state, not formatter bytes — with the
checkpoint this connection is known to have received, and sends `Palette`
before the `Snapshot` when they differ: default → custom, custom A →
custom B, and custom → default, for which the reset form exists. It may be
omitted when the connection's checkpoint already equals the current
palette — including the common case of a session that never touched it,
which is what ADR 0003 D4 wanted when it measured 5 531 bytes against a
five-byte screen. "Send only when the palette is not the default" was
proposed and rejected: a connection that once received a custom palette,
then lost the reset delta and resynced, would keep stale colours.

Palette changes between snapshots need no frame: they are bytes in the
deltas, and a replica applies them as the daemon did. The frame exists so
that an attach or resync checkpoint is self-consistent even when earlier
deltas cannot be trusted.

Invariant: a successful resync never depends on the client having received
an earlier palette delta.

**Old client.** `Unknown(8)`, skipped; the host terminal keeps its own
palette, as today. **New client, old daemon.** No frame; the hello lacks
`terminal.palette.v1`; the replica keeps the default palette, as today.

## D4 — Order, epochs, and resync

On a geometry-capable connection the sequence is always
`Geometry [Palette if required] Snapshot Delta*`, every prefix member
stamped with the snapshot's epoch and state point. A resync yields the
complete prefix even inside the same epoch: resync means the receiver's
replica assumptions are no longer trusted, and recovery facts are not
optimised away because the daemon's epoch did not change. A
`ResyncRequest` is never answered by a bare `Snapshot`.

Stale epoch: `Geometry` or `Palette` for an epoch the receiver has left is
discarded under the same rule as such a snapshot, and a prefix member from
one epoch is never combined with a snapshot from another. Missing prefix:
a `Snapshot` without the `Geometry` its capability promised is a desync
(D1); a bundle whose palette checkpoint the connection's state requires but
which is internally inconsistent is not installed as a synchronised
replica.

Invariant: resync is a complete state checkpoint, not another copy of
screen bytes.

## D5 — Compatibility contract

- Kinds 7 and 8 are assigned here and are never reused, renumbered, or
  given another meaning, though `STORAGE_EPOCH` is still `dev`: the
  permanence is self-imposed and deliberate.
- **Acceptance rested on a check against the decoder that predates this
  ADR**, not on fixtures added to the new one: on `main` at `38c75f8`, a
  kind-7 frame with a geometry payload and a kind-8 frame with OSC text
  decode as `Unknown(7)`/`Unknown(8)`, skippable, consumed exactly, with
  the following `Snapshot` still decoding and the stream frame-synchronised
  (`corral-protocol` `the_geometry_and_palette_kinds_are_skipped_by_a_decoder_that_predates_them`);
  the TUI's `apply` writes nothing for either and still applies the snapshot
  after them, and its loop does not disconnect on an unknown kind
  (`corral-tui` `geometry_and_palette_frames_write_nothing_and_the_snapshot_after_them_still_applies`).
  Both tests remain as the record. Daemon-side capabilities are therefore
  sufficient.
- Additivity: the legacy `Snapshot` is minted with `palette: false`, so no
  old client relies on palette state in the snapshot; `Palette` (8) is
  purely additive, and `Geometry` (7) carries nothing an old client had.
  Nothing an old client relies on leaves the legacy payload.
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

## Tests the implementation must carry

- Geometry: every capability-enabled snapshot is preceded by `Geometry`;
  its geometry equals the authoritative terminal's; a resync repeats it;
  stale-epoch `Geometry` is ignored; a missing `Geometry` under the
  advertised capability forces a desync.
- Dual screen: primary content established, alternate entered, snapshot
  taken while the alternate is active, a fresh replica installed, and the
  later alternate exit reveals the exact preserved primary viewport;
  trimming primary history does not damage the primary viewport; minting
  has zero observable mutation of active screen, cursor, or mode state.
- Palette: default → default may omit `Palette`; default → custom sends a
  checkpoint; custom A → custom B sends one; custom → default sends the
  explicit reset; a lost palette delta followed by a resync still rebuilds
  the effective palette; stale-epoch `Palette` is ignored.
- Compatibility: the pre-ADR decoder skips 7 and 8 (the two tests above);
  the TUI stays functional ignoring both.
- Prefix: `Geometry → Palette → Snapshot`; `Geometry → Snapshot` when no
  palette is required; all prefix members at one snapshot point; a resync
  never emits a bare `Snapshot`.

## Evidence this stands on

`docs/references/2026-09-04-pr9-gpui-integration-spike.md` scenario 1
(S3, S4 measured cell for cell; the snapshot payload bytes quoted) and
scenario 6 (a second viewer's resize pushes a snapshot to the first, S5).
