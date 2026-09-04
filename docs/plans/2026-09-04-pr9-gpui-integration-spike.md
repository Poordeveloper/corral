---
status: active
class: B
writes: [docs/references]
reads: [docs/adr/0003-terminal-snapshot-format.md, docs/references/architecture-benchmarks.md, docs/references/2026-08-23-s1-vt-serialization.md, docs/decisions/2026-08-22-surface-sequencing.md, crates/corral-protocol, crates/corral-client]
---

# PR9 spike — the facts the Desktop plan stands on

## Goal

Measure, first-party and on Corral's own chain, the facts the benchmark
ledger's Desktop row (`architecture-benchmarks.md` §8) decided from Zed's
source: that a client replica fed the daemon's snapshot and deltas
reproduces the authoritative screen; that gpui pins and builds inside this
workspace under its lints; that a custom Element painting that replica
meets a frame budget at realistic sizes with entity-per-terminal
invalidation; and that the tokio client and gpui's executors bridge with
bounded delta-to-paint latency. The ledger names its own gap — "no perf
measurements of ANSI-replica rendering vs Zed's direct-lock model; verify
in the spike" — and the tree has no client replica at all: the TUI writes
the daemon's bytes straight to the host terminal (`attach.rs`), so the
Desktop is the first client that must parse them itself.

Output: one dated reference,
`docs/references/<date>-pr9-gpui-integration-spike.md`, with the matrix
fields per scenario — gpui/emulator versions, OS, toolchain, scenario,
exact command, expected, observed, date, pass/fail — and a section of
findings addressed to the PR9 plan and to the ledger. The harness is
throwaway in the scratchpad, as S1's was; nothing it contains enters the
workspace.

## Non-goals

No production crate, no session list, no attention rendering, no tray, no
protocol or daemon change, no ADR. No emulator is committed here: a
verdict names what the PR9 plan may choose. Linux is compile-only — the
Linux host has no display. No measurement is invented to fill a cell; an
unmeasurable scenario is recorded as such with the reason.

## Method

One machine, the local Mac: the only host with a display. Recorded at spike
time: macOS 26.5.2, Xcode 26.2 (Metal toolchain present at
`xcrun -sdk macosx -f metal`), rustc 1.95.0, gpui 0.2.2 from crates.io
(the ledger of 2026-08-21 says "not on crates.io; pin a rev" — the first
finding to correct), gpui-component 0.6.0.

No provider runs. Sessions are `corral new -- bash` against a `corrald`
built from this tree; the harness is a second client that calls
`terminal.attach` and consumes the frame channel exactly as the TUI does
(`FrameKind::Snapshot` / `Delta` / `Resize` / `ResyncRequest`). Fidelity
is judged as S1 judged it: cell text, width and style, cursor position and
visibility, alternate-screen mode, scrollback presence — compared against
the daemon's own screen, never against the candidate's opinion of itself.
Timing is `Instant` around the Element's prepaint and paint, and a
wall-clock tag carried from delta arrival to paint completion.

## Scenarios

**Replica** (ADR 0003 leaves the client parser to the client)

1. **Fidelity per candidate.** S1's twenty dimension cases (alternate
   screen, scrollback, scroll region, wide CJK, emoji, combining marks,
   colors, cursor state, OSC title/color, DECCOLM …) driven through the real
   chain into each candidate: `vt100` 0.16.2, `alacritty_terminal` 0.26.0
   (Zed's display-only mode), `qwertty-term-vt` 0.4.0 reused as replica.
   Three arrivals per case: snapshot alone; snapshot then deltas; resize
   epoch then resync. Per candidate also: grid read-out API (row iteration,
   dirty tracking), palette intake (D4 sends it per connection), `unsafe`
   count that would now sit in the client, crate weight.

**Build** (ROADMAP: "pinned gpui rev")

2. **Pin and build.** gpui 0.2.2 in a scratch workspace that takes
   `corral-client` and `corral-protocol` as path deps under the workspace
   lints. Record: builds or not; crates added; duplicate-version conflicts
   with the workspace; cold and warm wall time; binary size; whether the
   Metal toolchain is needed at build, at run, or neither; `cargo deny
   check` against this repository's `deny.toml` (the merge gate's license
   and advisory set) over the added tree; gpui-component 0.6.0 compiles
   against that gpui. Linux: `cargo check` with x11 +
   wayland features on host `ne` (compile-only; time or failure recorded).

**Element** (ROADMAP: "custom Element; entity-per-terminal")

3. **Frame cost.** A custom Element painting the replica grid with one
   shaped line per style run, monospace with CJK and emoji fallback.
   Prepaint+paint p50/p95 at 80×24 and 200×60 under: full-screen scroll
   (`yes`), partial redraw (vim, htop), idle. Budget under test: p95 ≤ 8 ms
   at 200×60 — half a 60 Hz frame. With and without 4 ms coalescing:
   paints per second against deltas per second.
4. **Invalidation.** Twelve attached terminals in one window, one under a
   byte storm: paint count per entity — does only the stormed entity
   repaint? Cost of the off-screen ones. The same Element mounted embedded
   in a pane and standalone in its own window, the mode an enum fixed at
   construction, with nothing else differing.

**Bridge** (ARCHITECTURE: surfaces render streams they do not own)

5. **Executors.** `corral-client` on a tokio runtime thread; frames handed
   to the entity through a channel and `cx.spawn` / `update`. Delta
   arrival to paint completion p50/p95, idle and under storm. Frames
   applied in sequence order across an epoch change; a `ResyncRequest`
   round-trips. Input: key event → bytes the replica encodes from its own
   mode state (application cursor keys, bracketed paste) → `Input` frame →
   echoed `Delta`; round-trip latency.
6. **Lifecycle.** The harness window closes and its connection drops: the
   run is still alive and re-attachable (the TUI proves this daily; here it
   is measured once from a GPUI client so the PR9 plan does not assume it).
   Two clients attached to one run, one of them the harness: both see the
   same screen after a resize from either.

## Failure / unknown states

A candidate that fails a dimension is recorded per dimension, not as
"fails". A budget missed is a number, and the finding names what was
measured — not a recommendation to loosen the budget. A scenario the
machine cannot run (no Linux display; a Metal toolchain download the spike
may not perform) is recorded as unmeasurable with the blocking fact.

## Definition of done

- All six scenarios recorded with the header fields, or as unmeasurable
  with the reason.
- Verdicts stated explicitly: which replica candidates reproduce the
  daemon's screen on every dimension; the frame budget met or missed with
  the numbers; bridge latency numbers; ledger corrections listed under
  their own heading.
- Findings that contradict a ledger decision (§8's "Decision" line) are
  listed in a section addressed to the PR9 plan, never edited into the
  ledger by this spike.
- The reference lands with this plan moving to `done/`; the PR9 plan then
  cites it.
