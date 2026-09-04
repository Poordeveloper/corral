---
status: done
class: B   # high-consequence: terminal snapshot correctness (grill Q6)
writes: [crates/corrald/src/runtime/snapshot.rs, crates/corrald/src/runtime/snapshot_fidelity_tests.rs]
reads: [crates/corrald/src/runtime/terminal.rs, docs/adr/0003-terminal-snapshot-format.md, docs/decisions/2026-09-04-pr9-spike-grill.md, docs/references/2026-09-04-pr9-gpui-integration-spike.md]
---

# Snapshot fidelity — the adapter meets ADR 0003's contract, and a test owns it

## Goal

Close findings S1 and S2 of the PR9 spike, under grill Q6/Q11. The
snapshot corrald mints is the formatter's output plus a title; two things
in that output break the contract ADR 0003 D3 accepted:

- **S1.** The formatter writes the cursor position, then the tab-stop
  trailer (`\x1b[3g` and one `\x1b[<n>G\x1bH` per stop), and never restores
  the cursor. Every attach, resync and resize leaves the client's cursor on
  the last stop; the next bytes land there (spike: `hello fr` at column 72).
- **S2.** The formatter always drops trailing blank rows. A client scrolls
  fewer times than there are history rows, so history rows sit on its
  screen and its scrollback is short (spike: seven history rows on screen,
  one row short after 400 lines).

Corral's render adapter compensates after the formatter, as it already does
for the title; a permanent fidelity test — authoritative screen → snapshot →
fresh replica → equivalent state — owns the user-visible contract.

## Non-goals

Nothing Q7 has not ruled: no primary screen behind an active alternate
(S3), no palette transport (S4), no geometry in the snapshot (S5). No
upstream patch (a follow-up; upstream is never a correctness dependency).
No wire change, no client change.

## Existing owner / architecture involved

`runtime/snapshot.rs::render` selects the row range (history within the
budget plus the viewport), calls `Terminal::format_content` with
`Options::vt()` and every extra but the palette, and appends OSC 2 for the
title the formatter omits. `runtime/terminal.rs` owns the emulator and
exposes it read-only. ADR 0003 D3: a snapshot carries screen contents with
styles, cursor position and visibility, alternate-screen mode, scrolling
region, tabstops, charsets, title. The formatter (qwertty-term-vt 0.4.0,
`formatter.rs`): blank rows are deferred and flushed only when a later row
has text, so trailing blank rows never appear; `trim` does not change that.

## Design

**Row completion (S2).** The client must scroll exactly `history` times.
The formatter emits `range_rows − trailing_blank` rows; the adapter learns
that count from the formatter itself — the same range formatted as
`Format::Plain` with `unwrap: false` yields one `\n` per emitted row but the
last, and the same trailing-blank rule — and appends `\r\n` for each
missing row. Every LF past the last row scrolls one line, so the history
rows end up in history whether or not the content reached the bottom.
The extra pass is bounded by the same row budget as the snapshot itself
(D7) and runs once per attach, resync, or reflow.

**Cursor (S1).** After the padding and the title, the adapter writes
`CUP` from the active screen's cursor and `DECTCEM` from the
`cursor_visible` mode. Corral compensates for the formatter's order; it
does not claim the formatter is fixed.

**The trailer's order** is: formatter output (content, screen extras,
scrolling region, tab stops) → open what constrains movement (origin mode
off, full-screen region, margins off) → row padding → tab stops restated
absolutely when margins were on → margins, region, origin restored → title
→ cursor (region-relative under origin mode) → visibility. Padding precedes
the title and cursor because the LFs move the cursor; it needs the full
screen because a line feed scrolls into history only at the bottom of a
full-screen region.

**The fidelity test** (`snapshot_fidelity_tests.rs`, registered from
`snapshot.rs`) runs the spike's 22 scenarios in-process at 80×24: the
authoritative `AuthoritativeTerminal` fed the scenario's first half;
`encode` mints the snapshot; a fresh replica consumes `\x1b[H\x1b[2J` and
the payload as every client does; both then consume the second half.
Compared, before and after the second half: geometry, every visible cell
(`SnapshotRow` equality: grapheme, width class, style, link), cursor row,
column and visibility, active screen, scrolling region, every tab stop,
the title, and the modes the extras carry (cursor keys, bracketed paste).
History: the replica's scrollback length equals the rows the snapshot
declared it carries, and the last rows match. The resize scenario resizes
the authoritative screen first and builds the replica at the new geometry.

## Interfaces or persistence changed

None. The payload gains a few trailing bytes a VT already understands.

## Failure / unknown states

- A poisoned screen still refuses to snapshot (unchanged).
- A range with no rows: no padding, no cursor beyond `CUP 1;1`.
- A whole range of blank rows: the padding alone performs the scroll.
- A wrapped line across the history boundary re-wraps at the same width on
  the client, as before; the plain pass counts rows the same way.

## Tests

The fidelity module above; each scenario fails on the pre-fix adapter for
S1 (cursor column) or S2 (row count) or both. Existing `snapshot_tests`
(history budget, title, palette) stay and pass.

## Definition of done

- Fidelity module green on the 22 scenarios; the storm regression from
  `terminal_channel_tests` still green (its cell comparison already
  tolerated S1 by construction and now needs no such care).
- `./scripts/verify` on the final tree.
- PR body: `Class: B`, high-consequence; escalation triggers: none.

## Closed 2026-09-05

Landed with the design above, plus what the fidelity test found on the
way. A scroll region turns the padding's line feeds into region scrolls, so
the trailer opens the region and origin mode first and restores them after;
and left/right margins (DECLRMM) bend the formatter's own tab-stop trailer —
its `CHA`s are margin-relative, so the stops land shifted and clipped — so
the trailer restates the stops absolutely when margins are on. Both are
compensations in the same adapter, not new semantics. Twenty-four scenarios
(the spike's twenty-two plus origin mode with and without margins) and a
history test; before the fix all but one scenario failed on the cursor and
the history test on the row count.
