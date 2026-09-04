# PR9 GPUI integration spike — measured 2026-09-04

> Plan: `docs/plans/done/2026-09-04-pr9-gpui-integration-spike.md`. Harness:
> scratchpad `replica/` (fidelity, wire check) and `desk/` (GPUI app),
> throwaway, not in the workspace. Every number below comes from a run on
> this date on the hosts named here; nothing is quoted from documentation.

Hosts. **Host A** (all rendering, all daemon runs): macOS 26.5.2 on an
ultrawide 2560×1080 display, Xcode 26.2 (17C52), rustc 1.95.0, cargo 1.95.0,
cargo-deny as installed by the merge gate. **Host `ne`** (Linux compile only):
Ubuntu 24.04 host, udocker 1.3.17 PRoot container `spike` (node:22-bookworm,
rustc 1.95.0), 12 cores, 31 GB.

Versions under test: gpui 0.2.2 and gpui-component 0.6.0 from crates.io;
vt100 0.16.2; alacritty_terminal 0.26.0; qwertty-term-vt 0.4.0; corrald and
corral built from this tree at `2af25a8` with `test-support` so the daemon
could run under a private root (`CORRAL_TEST_ROOT=/tmp/pr9`; the scratchpad
path exceeds the Unix socket address limit, and the root must be mode 0700).

One limitation shaped the method and is stated up front: **the Mac's console
session was locked for the whole spike** (`CGSSessionScreenIsLocked = true`
in `ioreg`), and the spike shell runs under SSH + tmux
(`launchctl managername` = `Background`). A locked screen means no GPUI
window is ever occluded-visible, gpui 0.2.2 never starts its CVDisplayLink,
and a window draws exactly once — at creation. Wrapping the harness in an
`.app` and launching it through `open` or through `launchctl bootstrap
gui/<uid>` changed nothing. So the harness drives `Window::draw` itself on a
16.7 ms ticker whenever a view asked to be notified (`--drive-frames`); text
shaping and rasterisation go through the real `MacTextSystem`, only vsync
pacing and GPU present are absent. Scenario 3/4 tables below are those
CPU-side numbers. The screen was unlocked later the same day and the whole
pass was repeated under the real display link, launched through
`launchctl bootstrap gui/<uid>`; that pass is the section "Display-link
pass" and confirms the self-driven numbers. Evidence is layered (grill
Q1): the self-driven numbers are diagnostic and comparative; only the
display-link pass carries a performance claim, and any later performance
gate uses a real display-link environment.

## Scenario 1 — replica fidelity

**Method.** Reference = corrald's own `AuthoritativeTerminal` (qwertty) fed
the raw bytes, read through qwertty's renderer-facing `Terminal::snapshot()`.
Snapshot = corrald's own `runtime::encode`, i.e. the bytes a client is sent.
Three candidates fed (a) the snapshot alone, (b) the snapshot then the second
half of the case as deltas, (c) the raw bytes with no snapshot at all.
Compared per cell: text incl. combining marks, width class, fg/bg (exact,
then palette-resolved), bold/dim/italic/underline/inverse/strikethrough;
plus cursor position and visibility, alternate-screen mode, scrollback row
count and the last five history rows, OSC title, DECCKM and bracketed-paste
modes, palette entry 1. Twenty-two cases covering the S1 dimensions plus
resize-as-epoch and input modes.

**Wire check (`replica/src/bin/live.rs`).** Against the live daemon: client A
attached before the child printed a fixture and collected the deltas; client
B attached afterwards and received a snapshot. corrald's `encode` over A's
delta bytes equals B's wire snapshot **byte for byte** (663 bytes). The
in-process matrix below is therefore the real chain. (First attempt fed A's
own initial snapshot into the reference too, which displaced its cursor — the
defect in finding S1 below, found a second way.)

**Result on the snapshot as the daemon mints it today** — every candidate,
including qwertty reading its own format, failed most cases the same way,
because the snapshot itself is wrong in four respects (findings S1–S4).
With finding S1 patched in the harness (a `CUP` + `DECTCEM` appended after
the payload), the matrix is:

| dimension | vt100 0.16.2 | alacritty_terminal 0.26.0 | qwertty-term-vt 0.4.0 |
|---|---|---|---|
| text, wrap; 16/256/true colour; bold dim italic underline inverse | identical | identical | identical |
| strikethrough | not modelled | identical | identical |
| cursor position, cursor hidden | identical | identical | identical |
| alternate screen (snapshot taken inside it) | identical | identical | identical |
| alternate screen left after the snapshot | **DIFFERS** (S3) | **DIFFERS** (S3) | **DIFFERS** (S3) |
| OSC title | not modelled | identical | identical |
| OSC 4 palette | not modelled | not carried (S4) | not carried (S4) |
| wide CJK, emoji, combining marks | identical | identical | identical |
| scrollback (400 lines) | off by one (S2) | off by one (S2) | off by one (S2) |
| erase + redraw with history | **DIFFERS** (S2) | **DIFFERS** (S2) | **DIFFERS** (S2) |
| scroll region | identical (raw: **DIFFERS**) | identical | identical |
| tabs (TBC/HTS in the trailer) | **DIFFERS** (no TBC/HTS) | identical | identical |
| insert/delete line and char | identical | identical | identical |
| resize → new epoch snapshot | identical | identical | identical |
| DECCKM, bracketed paste carried | identical | identical | identical |

Raw-path notes: alacritty pushes the screen into history on `ED 2` (7 vs 30
history rows on the erase case) and stores `\t` in cells — both invisible
through the snapshot path; vt100 mishandles a DECSTBM region on raw input.
Snapshot payloads: 80–191 bytes for a mostly empty 80×24 screen, 3 967 bytes
with 376 history rows.

**Verdict.** On the snapshot path all three candidates reproduce the daemon's
screen wherever the snapshot carries the fact. qwertty-term-vt reproduces
every dimension the daemon can express, including title, modes and
strikethrough, and is the same engine on both ends; alacritty_terminal is a
close second (everything but the palette, which the snapshot omits by D4);
vt100 lacks title, palette, strikethrough, tab stops and has a DECSTBM defect.

### Findings for corrald's snapshot minting (`runtime/snapshot.rs::render`)

- **S1 — the tab-stop trailer displaces the cursor.** The formatter emits the
  cursor position, then `\x1b[3g` and one `\x1b[<n>G\x1bH` per stop, and
  never restores the cursor; every snapshot leaves the client cursor on the
  last stop (column 73 of 80, 97 of 100). Observed on the wire: the first
  bytes a freshly attached client sees land at column 72 (`SCREEN 00|` … 72
  spaces … `hello fr`), and after every resync or resize the next typed
  characters appear there (`echo hello from a k` … `ey`). The TUI applies
  the same payload to the host terminal (`attach.rs::apply`), so it is
  affected by construction; not separately measured here.
- **S2 — trailing blank rows are trimmed, so history rows land on screen.**
  The payload is history rows followed by viewport rows as plain text; blank
  trailing viewport rows are not emitted, so a client scrolls too few times.
  Erase case: 7 history rows painted onto the visible screen, scrollback 0;
  400-line case: one row short.
- **S3 — the primary screen is omitted while the alternate is active.** The
  snapshot is `\x1b[?1049h` + alternate content; a client that later sees
  `\x1b[?1049l` restores a blank main screen where the daemon has `main1`.
- **S4 — OSC 4 palette changes are not carried**, by D4's intent
  (per-connection palette), but no frame kind for the palette exists yet.
- **S5 — a snapshot carries no geometry.** A viewer that did not request a
  resize receives the new epoch's snapshot (scenario 6 confirms the daemon
  pushes one to every viewer) and cannot size its replica. The TUI never
  needed it: its replica is the host terminal. Additive protocol work for
  PR9 — rows/cols in the snapshot, or a geometry frame.
- Client note: the frame after a snapshot carries the snapshot's own
  sequence number (`Snapshot@e0s0`, then `Delta@e0s0`); a gap detector must
  start at the snapshot's sequence, not one past it.

## Scenario 2 — pin and build

| fact | observed |
|---|---|
| gpui on crates.io | yes: 0.2.2 (ledger §8 says "not on crates.io; pin a rev") |
| macOS cold build, dev profile, deps at `opt-level = 3`, hello-window crate | **4 m 28 s** wall (1 278 s user), 445 rlibs, target 2.3 GB, binary 8.3 MB |
| same, before the Metal toolchain existed | fails at 3 m 25 s: `cannot execute tool 'metal' due to missing Metal Toolchain; use: xcodebuild -downloadComponent MetalToolchain` |
| Metal toolchain | needed at **build** time only (gpui's `build.rs` compiles `shaders.metal`; the metallib is embedded). The spike ran `xcodebuild -downloadComponent MetalToolchain` on Host A: 704.6 MB downloaded in 45 s, no admin prompt, "Metal Toolchain 17C7003j". This changed the development machine's Xcode toolchain state; it is not a repository artifact and stays installed because PR9's macOS build needs it (grill Q1) |
| warm rebuild after a harness edit | 0.73 s; a fresh `cargo check` of the same tree 1 m 40 s |
| harness with corral-client + corral-protocol + qwertty + tokio as path/registry deps | builds; Cargo.lock 712 packages; binary 11.5 MB |
| duplicate-version conflicts with the workspace | none blocking; cargo-deny counts 64 duplicated crates in gpui's tree (3× `syn`, `hashbrown`, `getrandom`, `ttf-parser`; 4× `windows-sys`; 2× `rustix`, `nix`, `toml`, `thiserror`, …) |
| gpui-component 0.6.0 | resolves to the same gpui 0.2.2 (no second gpui); `cargo check` 1 m 06 s; +237 packages (949 total) |
| `cargo deny check` with this repo's `deny.toml` | **advisories FAILED, licenses FAILED**, bans ok, sources ok |
| Linux (`ne` container, default features x11 + wayland) | cold `cargo check` **1 m 31 s** with no system package; cold `cargo build` compiles everything in 2 m 08 s and fails at link (`unable to find library -lxkbcommon-x11`); after `apt-get install libxkbcommon-dev libxkbcommon-x11-dev libwayland-dev libx11-xcb-dev libfontconfig1-dev` the relink takes 4.6 s; 466 MB unstripped debug binary, 4.3 GB target |

cargo-deny detail. Licenses outside the allow list, 25 crates:
BSD-3-Clause (avif-serialize, bindgen, exr, instant, lebe, ravif, sha1_smol,
subtle, tiny-skia, tiny-skia-path), BSD-2-Clause (arrayref, av1-grain,
rav1e, v_frame), ISC (libloading, rustls-webpki, untrusted), MPL-2.0
(cbindgen, dwrote, option-ext), CC0-1.0 (hexf-parse, tiny-keccak),
`Apache-2.0 WITH LLVM-exception` (ar_archive_writer), `Apache-2.0 AND ISC`
(ring), `(Apache-2.0 OR MIT) AND BSD-3-Clause` (encoding_rs),
`(MIT OR Apache-2.0) AND NCSA` (libfuzzer-sys). Advisories, all
unmaintained: async-std, instant, paste, proc-macro-error2, rustls-pemfile,
rustybuzz (two versions), ttf-parser (three versions). Whether the allow
list grows to admit these is a founder decision the PR9 plan must carry; the
spike changes no policy.

## Scenario 3 — Element frame cost

Harness element: one custom `Element`; per row, consecutive narrow cells are
shaped as one line with style runs, each wide cell is its own segment
positioned at `col × cell_width`; cell backgrounds and the cursor are quads;
Menlo 12 px, line height 15 px, cell 7.22 px. Harness crate at `opt-level =
2`, deps at 3. `render` = `snapshot_window(0)` when dirty; `paint` = the
Element's paint phase (shaping included, cached across frames by gpui's
line-layout cache). Latency = delta arrival on the tokio thread → paint
complete, under the 16.7 ms self-driven tick. Storm = `yes` of a 100-byte
line with CJK and an emoji; the daemon delivers it at ~8.7 MB/s in ~1 KiB
deltas, which is the ceiling every storm row below shares.

| run | frames / bytes in the window | paints | render p50/p95 | paint p50/p95/max | arrival→paint p50/p95 | apply total |
|---|---|---|---|---|---|---|
| idle 80×24 | 1 / 76 B | 2 | 47 µs | 35 µs | — | — |
| idle 200×60 | 1 / 193 B | 2 | 268 µs | 191 µs | — | — |
| storm 80×24, coalesce 4 ms | 67 304 / 68.9 MB | 427 | 31 / 37 µs | **178 / 195 µs** (max 29 ms, first frame) | 18.2 / 18.9 ms | 1 360 ms of 8 s (repeat run, 69 956 frames) |
| storm 80×24, no coalescing | 71 215 / 72.9 MB | 318 | 33 / 37 µs | 180 / 201 µs | 25.2 / 25.3 ms | — |
| storm 200×60, coalesce 4 ms | 73 495 / 75.2 MB | 414 | 159 / 177 µs | **1 052 / 1 081 µs** (max 6.3 ms) | 20.4 / 21.3 ms | — |
| storm 200×60, no coalescing | 72 709 / 74.4 MB | 318 | 161 / 187 µs | 1 056 / 1 088 µs | 26.0 / 26.8 ms | — |
| `top -s 1` 200×60 | 15 / 11 KB | 4 | 253 µs | 191 µs (p95 4.0 ms) | 17.3 ms | — |

Budget under test was p95 ≤ 8 ms at 200×60; measured **1.08 ms**, seven
times inside it, on a dev-profile harness. The replica parse (`apply`) costs
about 19 ns per byte, 17 % of the UI thread at the daemon's ceiling. Without
coalescing each of ~9 000 deltas per second notifies the view; paints drop
from 427 to 318 and arrival-to-paint rises 7 ms: the notify effects crowd
the foreground executor. 4 ms coalescing is the right default.

**Font fallback (`FONT_DEBUG`).** Menlo 12 px cell = 7.22 px. The CJK
fallback glyph advances 12 px (less than the 14.45 px two-cell slot); the
emoji advances 16 px (more). A row shaped whole would drift; per-wide-cell
positioning keeps the grid, with a 2.4 px gap after CJK and a 1.5 px overlap
after emoji that the Element must clip or scale. Bold and italic resolve to
distinct font ids as expected.

## Scenario 4 — invalidation

Twelve terminals in one window, one under the storm, eleven idle
(`sleep`); storm delayed 4 s so every session existed first (session_new
4.6–13.1 ms each, attach + channel 0.2–0.4 ms).

| configuration | storming view paints | each idle view paints | idle view paint cost |
|---|---|---|---|
| plain child views | 117 | **117** | 36 µs / 80×24 |
| child views wrapped `AnyView::cached` | 164 | **2** (initial) | — |
| 3 views, plain | 453 | 453 | 35 µs |
| 12 views, 8×40 cells | 455 | 455 | 6 µs |
| 2 standalone windows | 454 | **2** | — |

A gpui view that is not `cached` is re-rendered and re-painted on every
window frame regardless of who was notified; `cached` gives the
entity-per-terminal invalidation the ledger describes. Standalone windows are
independent by construction (per-window dirty). Cost of an off-screen or
idle 80×24 terminal when not cached: ~35 µs per frame; a Desktop with a few
dozen visible terminals affords either, but `cached` is the shape to start
from. The same `TerminalElement` renders in both the embedded pane and the
standalone window with a mode enum fixed at construction and no other
difference.

## Scenario 5 — executors and the wire

tokio (multi-thread runtime on its own thread; reader and writer task per
channel) → `futures::channel::mpsc::unbounded` → `cx.spawn` loop applying
frames on the foreground. No deadlock in any run; frames applied in order;
epoch changes handled by rebuilding the replica on the new snapshot.

| measurement | observed |
|---|---|
| key → echoed delta arrival (interactive bash, 22 keys) | p50 **0.98 ms**, p95 1.8 ms |
| key → echo painted (with the 16.7 ms tick + 4 ms coalescing) | p50 20.6 ms |
| `ResyncRequest` → fresh `Snapshot` | **1.43 ms** |
| `Resize` 24×80 → 30×100 → new-epoch `Snapshot` | **3.15 ms** |
| session_new (daemon spawns `sh -c`) | 4.6–13.1 ms |
| terminal_attach + second connection hello | 0.2–0.4 ms |

**Finding S6 — the daemon closes a terminal channel under sustained output.**
`terminal_channel.rs` queues outbound frames in an 8-slot `mpsc` and
`try_send`s; a full queue is treated as "this client has stopped reading" and
the channel returns. At the storm's ~8 700 deltas per second the writer task
falls 8 frames behind within a few milliseconds of jitter, and the client
sees EOF with no `ChannelError` frame and no daemon log line. Six sustained
9-second storms: channel closed in **5** (at 2.1, 7.1, 7.2, 7.8, 8.9 s). A
one-shot 40 MB `cat` survived twice; a 1 MB/s stream survived. The TUI
attaches over the same channel and would detach under the same load, by
construction; not measured with the TUI. Not the ADR 0003 D-path
(per-viewer queue overflow → resync by snapshot), which never triggered here
(4 MiB / 1 024 frames). Owner: corrald; a P1 candidate for the PR9 plan's
prerequisites.

## Scenario 6 — lifecycle

- Client types `echo alive-after-detach`, window closes, connection drops:
  `corral list` shows the session `Running · Status unknown`; a new client
  attaches and receives the same screen (`alive-after-detach` present).
- Two clients on one run: B resizes to 30×100; A receives a second snapshot
  (`snapshots=2`) without asking — and, per S5, no geometry with it.

## Display-link pass (screen unlocked, same day)

Same harness, no `--drive-frames`; the window is driven by gpui 0.2.2's
CVDisplayLink on macOS 26.5. `ticks` counts `on_next_frame` callbacks per
view (twelve views register twelve chains, so divide by the view count).

| run | ticks (per view, per s) | paints | paint p50/p95/max | arrival→paint p50/p95 | notes |
|---|---|---|---|---|---|
| idle 80×24, 8 s | 65 | 3 | 38 / 40 µs | — | the link keeps ticking while the window is visible; nothing is repainted |
| storm 80×24, coalesce 4 | 66 | 221 | 204 / 260 µs | 16.9 / 17.5 ms | channel closed at 5.7 s (S6) |
| storm 200×60, coalesce 4 | 60 | 537 | **1 161 / 1 343 µs** (max 16.9 ms) | 18.0 / 18.8 ms | 82 482 frames, 84.4 MB, full run |
| storm 200×60, no coalescing | 66 | 8 | — | — | channel closed at 2.1 s (S6); no comparison possible |
| interactive echo + resync | 65 | 14 | 755 / 925 µs | 17.8 / 27.0 ms | echo arrival p50 0.67 ms; echo painted p50 18.2 ms; resync 1.31 ms |
| twelve plain, storm on one | 62 | storm 436, each idle **436** | idle 35 / 50 µs | 17.0 / 33.2 ms | channel closed at 11.7 s (S6) |
| twelve `cached` | 60 | storm 467, each idle **3** | — | 16.9 / 33.6 ms | full run |
| two standalone windows | 66 | storm 26, idle **2** | — | — | channel closed at 2.4 s (S6) |

Verdicts stand: paint p95 1.34 ms at 200×60 under vsync (budget 8 ms);
arrival-to-paint is vsync-bound at 17–19 ms p50; `cached` views isolate;
standalone windows are independent. CVDisplayLink works on macOS 26.5 for
this gpui version. S6 recurred in four of six sustained storms in this pass
(2.1, 2.4, 5.7, 11.7 s), which makes it the most reproducible defect the
spike found.

## Unmeasured

- Linux rendering (no display on `ne`).
- Release-profile frame numbers (dev profile only; deps optimised).
- GPU present cost as a separate number (the pass above includes it in the
  vsync-bound latency, not in `paint`).

## Findings addressed to the PR9 plan and the ledger

1. Ledger §8 "gpui not on crates.io; pin a rev" is out of date: pin
   `gpui = "0.2.2"`; its CVDisplayLink runs at 60 Hz on macOS 26.5.
2. Xcode 26 ships without the Metal toolchain; the PR9 plan's dev-loop
   section must name the download (build-time only).
3. `deny.toml` cannot admit gpui as it stands (25 licences, 7 unmaintained
   crates); a founder decision on the allow list precedes any Desktop crate.
4. Replica: qwertty-term-vt as the client engine (same engine both ends;
   everything the snapshot carries reproduces). The Element positions per
   wide cell; do not shape rows whole.
5. Five snapshot-format defects (S1–S5) sit in corrald and are visible in the
   TUI today (S1 on every attach); S5 needs an additive protocol change
   before a Desktop can host a second viewer. Follow-ups, not spike edits.
6. Channel close under load (S6) is a daemon defect that PR9 would ship on
   top of — nine of twelve sustained storms across both passes closed the
   channel; its fix precedes dogfood with a Desktop.
7. 4 ms coalescing, `AnyView::cached` per terminal, tokio-on-a-thread with an
   unbounded futures channel: all hold; numbers above.

## Addendum — Linux cold timings (`ne` container, 12 cores)

- `cargo check`, cold, default features: 1 m 31 s wall, 5 m 40 s user. No
  system package needed: x11rb and wayland-client are pure Rust and
  `cargo check` never links.
- `cargo build`, cold: every crate compiles in 2 m 08 s, then the link fails
  on `-lxkbcommon-x11`. Bookworm packages that satisfy it:
  `libxkbcommon-dev libxkbcommon-x11-dev libwayland-dev libx11-xcb-dev
  libfontconfig1-dev`; relink 4.6 s afterwards. Debug binary 466 MB
  unstripped; target 4.3 GB. The PR9 plan's Linux dev-loop section names
  these packages.
- Container jobs on `ne` must run under `tmux new -d`; `nohup`/`setsid`
  launches die with the ssh session.
