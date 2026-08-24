# S1 — VT serialization, verified first-party

> Spike evidence (`ROADMAP.md` §3 S1). Closes the emulator-selection gap that
> `architecture-benchmarks.md` row 5 left open and that ADR 3 needs. Every
> number below came from a run performed for this spike on this machine on
> 2026-08-23, against the versions named; nothing here is quoted from a README.
> Harness: `scratchpad/s1`, `cargo run --bin {roundtrip,qwertty,scrollback,bytelog}`.

## The question

Row 5 already fixes the wire model — daemon-owned authoritative VT, snapshot
@ seq N + sequenced raw deltas, resync-by-snapshot only, resize ⇒ new epoch —
with high confidence and three independent confirmations. It leaves one thing
open: **which VT can hold authoritative state and serialize it back to ANSI so
a client parser reproduces an identical screen**, or whether none can and the
per-epoch raw byte log is the answer.

## Method

The chain under test:

```text
PTY bytes → authoritative VT → ANSI snapshot → client parser → screen
                    │                                            │
                    └──────────── compared cell by cell ─────────┘
```

Twenty synthetic cases, one per dimension `ROADMAP` names, so a failure names
the dimension rather than "something in a vim session". Comparison is every
cell's text, width and style, plus cursor position and visibility, alternate
screen, title, and the rows scrolled off the top.

**Three of this harness's own results were false passes before they were
real.** They are worth recording, because each is a trap for whoever
integrates the winner:

1. The first dumper compared only the viewport, so a snapshot that dropped
   30 lines of history "matched".
2. It read the title from an engine that models no title, so both sides
   "agreed" by observing nothing.
3. `scrollback` means **rows** in vt100 and **bytes** in qwertty — Ghostty's
   model. Passing the same number to both gave both engines a scrollback of
   effectively nothing, and the dimension passed without being tested.

A green result from a comparison that cannot see the thing being compared is
worse than a red one. The verdicts below are from the harness after all three
were fixed.

## Triage

| candidate | version | serializes state to ANSI? | evidence |
|---|---|---|---|
| vt100 | 0.16.2 | yes | `Screen::state_formatted()`, `state_diff(prev)`, `cursor_state_formatted()` |
| qwertty-term-vt | 0.4.0 | yes | `formatter::Format::Vt` with `TerminalExtra`; a Rust port of Ghostty's `terminal/formatter.zig` @ `2da015cd6` |
| alacritty_terminal | 0.26.0 | **no** | only `renderable_content()`; no escape-emitting path anywhere in `src/`. Row 5's claim confirmed against this version |
| termwiz | 0.23.3 | n/a | `Surface` is a render abstraction fed by `Change`s, not a VT model for arbitrary PTY bytes. The terminal model is `wezterm-term`, which **is not on crates.io** |
| vtcode-ghostty-core | 0.128.4 | **no** | 1,304 lines total; `screen_dump()` is plain text |
| per-epoch raw byte log | — | n/a | needs no serializer; measured below |

**The Zig question may not need answering.** Row 5 recorded that snapshot
minting needs an emulator that can serialize, that ghostty-vt can, and that a
Zig dependency was "neither accepted nor rejected before the spike". A pure-Rust
port of that same formatter now exists on crates.io. Ghostty's serializer is
reachable without a Zig toolchain.

## Fidelity

Twenty dimensions, same corpus, same comparison.

| dimension | vt100 0.16.2 | qwertty 0.4.0 |
|---|---|---|
| text, wrap | identical | identical |
| 16 / 256 / truecolour | identical | identical |
| bold, dim, italic, underline, inverse | identical | identical |
| cursor position, cursor hidden | identical | identical |
| **alternate screen** | **DIFFERS** | identical |
| alternate screen restored | identical | identical |
| **OSC title** | **not modelled** | **DIFFERS** |
| OSC colour | not modelled | identical |
| wide CJK, emoji, combining marks | identical | identical |
| **scrollback** | **DIFFERS** | identical |
| erase + redraw, scroll region, tabs, insert/delete line | identical | identical |

**vt100 loses the alternate screen.** `write_contents_formatted` emits hide-cursor,
grid contents and an attribute diff, and not the mode. A client restoring from
the snapshot paints the alternate screen's contents onto the *main* screen; when
the program later emits `\x1b[?1049l` the client restores a main screen it never
had. Every full-screen TUI runs in the alternate screen, which is where Corral's
whole product lives.

**vt100 loses scrollback.** 400 lines into a 24-row screen: the authoritative
screen still holds `line 1`, the screen restored from its own snapshot does not.
This is the ledger's "Herdr loses >8KB/pane" note, confirmed first-party.

**vt100 models no OSC at all.** `osc_dispatch` forwards every sequence to a user
callback; the crate keeps no title, palette or hyperlink state. This is not a
round-trip failure — it is a missing model, and `ROADMAP` names OSC title/color
as a dimension.

**qwertty loses the title, and only that.** `Terminal` does model the OSC 0/2
title, and `TerminalExtra` re-emits palette, modes, scrolling region, tabstops,
pwd and keyboard modes — but not the title, so it does not survive the snapshot.
A narrow, visible gap in a formatter that has an obvious place to put the fix,
which is a different kind of problem from having no model to fix.

## Cost

80×24 screen, appending output, snapshot taken at the end.

| lines of history | qwertty snapshot | time | vt100 snapshot | time |
|---|---|---|---|---|
| 1,000 | 46 KB | 1.3 ms | 767 B | 0.1 ms |
| 10,000 | 424 KB | 8.0 ms | 790 B | 0.0 ms |
| 100,000 | 4.29 MB | 28.6 ms | 813 B | 0.0 ms |

vt100's snapshot is small because it carries the viewport and nothing else.
qwertty's carries the history, at roughly 43 bytes per line.

Row 5 makes scrollback depth and snapshot extent wire-contract numbers, with
Zed's 10k default / 100k max as the reference. **At those numbers a snapshot is
424 KB or 4.3 MB**, sent on every attach and every resync. ADR 3 should bound
snapshot extent separately from scrollback depth rather than letting one number
set both.

A fixed 5.5 KB of that is the 256-colour palette (`TerminalExtra::styles()`
costs 5,531 bytes against `none()`'s 5). Per-connection rather than
per-snapshot, if the wire lets it be.

## The byte-log fallback

Keep the raw bytes since the epoch and replay them; no serializer needed. It
divides sharply by the shape of the output.

| output shape | byte log | serialized snapshot | ratio |
|---|---|---|---|
| appending, 10,000 lines | 339 KB | 424 KB | 0.8× |
| appending, 100,000 lines | 3.5 MB | 4.3 MB | 0.8× |
| redrawing, 1,000 frames | 969 KB | 47 KB | 20× |
| redrawing, 10,000 frames | **9.9 MB** | **41 KB** | **243×** |

For output that appends, the byte log is *smaller* than the serialization and
serializing buys nothing. For output that redraws, the byte log grows without
bound while the screen does not — 243× at ten thousand repaints, and still
climbing, because it is bounded by everything ever written rather than by what
is on screen.

Corral hosts interactive agent TUIs. They redraw. **The byte log cannot be the
primary mechanism**, though it remains the honest answer for a session whose
output only ever appends, if ADR 3 ever wants that case.

## What the winner costs to depend on

| | vt100 0.16.2 | qwertty-term-vt 0.4.0 |
|---|---|---|
| direct dependencies | 3 (`itoa`, `unicode-width`, `vte`) | 1 (`base64`) |
| `unsafe` blocks in `src/` | **0** | **936**, plus 141 `unsafe fn` |
| `SAFETY` comments | — | 333 |
| `#[test]` in the published crate | 0 | 1,644 |
| version | 0.16.2 | 0.4.0 |

qwertty's unsafe is concentrated in `page/`, `pagelist/` and `screen/` — the
packed-page memory layout it ports from Ghostty, which is where its fidelity
and its speed come from. About a third of those blocks carry a `SAFETY` note.

This matters more than a dependency count usually would: `ARCHITECTURE.md` §5
declares provider data untrusted input, and the VT parser is the first thing
every byte of it reaches. AGENTS.md forbids unsafe in Corral's own crates; it
says nothing about dependencies, and the tree already contains plenty. But a
0.4.0 hand-port with 936 unsafe blocks in the untrusted-input path is a
different risk from a 0-unsafe crate, and naming it is the point of measuring.

## Recommendation

**qwertty-term-vt, with the title gap closed and the unsafe surface treated as
a known risk** — or vt100 only if ADR 3 is willing to give up the alternate
screen and scrollback in snapshots, which it should not be.

The reasoning is that vt100's three gaps are not implementation bugs. Alternate
screen and scrollback are absent from what it chose to serialize; OSC is absent
from what it chose to model. Closing them means writing a serializer and a
model, which is the work the spike exists to avoid. qwertty's one gap is a
formatter that does not emit a field it already tracks.

Against that, qwertty is 0.4.0 and carries a large unsafe surface where
untrusted bytes arrive. It has 1,644 shipped tests and a differential-testing
story against the Zig original, which is more than most crates at that version,
but neither substitutes for exposure.

## What would change this

- vt100 growing alternate-screen and scrollback into `state_formatted`, and an
  OSC model. Then it wins on every other axis.
- Fuzzing qwertty's page layer against malformed PTY output and finding it
  fragile. The spike did not fuzz it.
- ADR 3 deciding snapshot extent is small enough that scrollback need not be in
  the snapshot at all — in which case vt100's gap stops mattering and its zero
  unsafe starts to.

## Not tested

Named so nobody reads this as more complete than it is. The
cross-implementation check — serialize with one engine, parse with the other —
did not run; both round trips are self-comparisons, which cannot catch an engine
that is wrong the same way twice. Resize across a snapshot epoch, DA/DSR
query-reply, real captured streams from vim and a live TUI, Linux (measured on
macOS only), and compile time are all unmeasured.
