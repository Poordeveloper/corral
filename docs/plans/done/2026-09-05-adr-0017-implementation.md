---
status: done
class: B   # high-consequence: protocol, terminal runtime, snapshot/resync correctness (ADR 0017 grill Q6)
writes: [crates/corral-protocol/src/terminal.rs, crates/corral-protocol/src/hello.rs, crates/corrald/src/connection.rs, crates/corrald/src/runtime/snapshot.rs, crates/corrald/src/runtime/palette.rs, crates/corrald/src/runtime/session.rs, crates/corrald/src/runtime/terminal.rs, crates/corrald/src/terminal_channel.rs, crates/corral-tui/src/attach.rs]
reads: [docs/adr/0017-terminal-snapshot-geometry-screens-palette.md, docs/decisions/2026-09-05-adr-0017-grill.md, docs/adr/0003-terminal-snapshot-format.md]
---

# ADR 0017 materialized — the snapshot prefix, both screens, the palette checkpoint

## Goal

Implement the accepted ADR 0017 as one PR, before PR9 planning (grill Q6):
frame kinds `Geometry` (7) and `Palette` (8); daemon capabilities
`terminal.geometry.v1` and `terminal.palette.v1`; the snapshot prefix
`Geometry [Palette] Snapshot` at one state point, repeated on every resync;
the primary screen carried behind an active alternate; the per-connection
palette checkpoint including the explicit return to default.

## Non-goals

No client that acts on the prefix (the Desktop is PR9's; the TUI ignores
both kinds by design). No durable state. No change to the budgets' numbers.

## Design

- `corral-protocol`: the two kinds, numbered once; the two capability
  names. `corrald::connection` advertises both unconditionally.
- `runtime/palette.rs`: `PaletteCheckpoint` — the effective palette read
  from the emulator's live colour state, compared semantically; its frame
  payload is a full reset (OSC 104/110/111) followed by every non-default
  entry and the dynamic foreground/background, so it is exact whatever the
  client held. `Attachment` carries the checkpoint and the geometry the
  snapshot was minted for; the final screen record carries both too.
- `runtime/snapshot.rs`: `encode` takes the terminal mutably; while the
  alternate is active the primary screen is formatted first through a
  `FormattingScreen` guard that flips the active key and restores it on
  every exit, then `?1049h` and the alternate. History is the primary's and
  is what the budget trims; both viewports are required, and the ceiling
  error names which screens it refused. The S1/S2 trailer applies per
  screen.
- `terminal_channel`: the writer keeps the connection's known checkpoint
  (baseline default) and sends `Geometry`, then `Palette` when the
  attachment's checkpoint differs, then `Snapshot`, all stamped alike; the
  known checkpoint advances only after the frame is written.
- `corral-tui`: `apply` writes nothing for the two kinds.

## Tests

Protocol round-trip and the still-unknown kind after them; palette
checkpoint unit tests (default→default omitted, default→custom, A→B,
custom→default explicit reset, lost delta then checkpoint); fidelity
scenarios leaving the alternate screen, with history behind the primary,
and the zero-mutation regression across a dual-screen mint; channel tests
for the prefix and its stamps, resync repeating it, the bystander viewer
learning a resize from `Geometry`, and a changed palette reaching a later
viewer.

## Definition of done

All of the above green, `./scripts/verify` on the final tree, and the PR
body carries `Class: B` with no escalation triggers.

## Closed 2026-09-05

Landed as designed. One detail the fidelity test caught: entering 1049
keeps the cursor's coordinates, so the alternate's rows are written from
home explicitly. Client-side rules (stale-epoch discard, desync on a
missing prefix) belong to the first replica client, PR9.
