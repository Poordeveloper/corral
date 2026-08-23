---
status: proposed
read_when:
  - choosing or changing the VT implementation `corrald` runs
  - changing what a terminal snapshot contains or how large it may be
  - changing sequence, epoch, or resync mechanics on the terminal channel
  - deciding what the daemon answers while no client is attached
  - exposing terminal state on the wire or in a surface
---

# Terminal snapshot format: what a client is sent, and what it may assume

`ARCHITECTURE.md` §3 fixes the outcome — `corrald` owns the authoritative VT,
the wire is an ANSI replay serialization rather than a cell grid, recovery has
exactly one path, resize starts a new epoch, input is encoded client-side, and
PTY bytes are replayed unmodified. This ADR fixes the mechanics under that, on
the measurements spike S1 produced
(`docs/references/2026-08-23-s1-vt-serialization.md`). Scheduled by
`ROADMAP.md` §3 for PR3.

**The invariant.** A snapshot is a claim about what is on a screen, and a
client that replays one must arrive at the screen the daemon actually holds.
Anything the daemon knows and the snapshot cannot express is a divergence the
client has no way to detect — so the snapshot's contents are a contract, not an
implementation detail of whichever emulator is underneath.

## D1 — The authoritative VT is `qwertty-term-vt`, and its risk is named

One bounded emulator per session, in `corrald`. S1 measured the chain on twenty
dimensions: `alacritty_terminal` cannot serialize at all, `termwiz`'s terminal
model is not published, and `vt100` drops the alternate-screen mode, drops all
scrollback, and models no OSC — three of the dimensions `ROADMAP` names.
`qwertty-term-vt` 0.4.0, a pure-Rust port of Ghostty's formatter, round-trips
every dimension but the OSC title. The Zig dependency the benchmark ledger left
open therefore does not need deciding.

It is chosen with a cost stated rather than discovered later: 936 `unsafe`
blocks and 141 `unsafe fn`, concentrated in the packed-page memory layer, on
the path every byte of untrusted provider output takes first. About a third
carry a `SAFETY` note. `vt100`, the alternative, has none.

**So the emulator is fuzzed against malformed PTY output before PR3 ships.**
`ARCHITECTURE.md` §5 requires that malformed provider data degrade a session
rather than panic `corrald`, and a `catch_unwind` cannot contain undefined
behaviour — only evidence that the parser survives hostile input can. A crash
found later is a bug; memory unsafety found later is a security finding.

Rejected: `vt100` with a hand-written alternate-screen and scrollback
serializer, which is the work the spike existed to avoid, and would leave
Corral maintaining a VT serializer as a side effect of shipping a session
manager.

## D2 — Snapshot extent is its own number, not scrollback depth

`ARCHITECTURE.md` §3 already calls both wire-contract numbers. S1 measured what
happens when one sets both: at the reference scrollback depths a snapshot is
**424 KB at 10k lines and 4.29 MB at 100k**, sent on every attach and every
resync.

A snapshot therefore carries the viewport plus a bounded number of scrollback
lines, and that bound is smaller than the scrollback the daemon retains. The
daemon may hold more history than it ships; a client is told how much it got
and does not infer that it received everything.

Rejected: shipping whatever the emulator holds. Resync is the only recovery
path, so its cost is paid at exactly the moment a session is already in
trouble.

## D3 — What a snapshot must carry

Screen contents with styles, cursor position and visibility, the
alternate-screen mode, the scrolling region, tabstops, the active character
sets, and the window title.

The title is called out because the chosen formatter tracks it and does not
re-emit it: **Corral emits OSC 2 into the snapshot itself.** A field the
emulator models but the serializer omits is exactly the divergence D1's
invariant is about, and it is Corral's to close.

## D4 — The palette is sent per connection, not per snapshot

S1 measured 5,531 bytes of 256-colour palette in a snapshot whose content was
five bytes. Resync is the recovery path, so that overhead lands repeatedly and
precisely when a connection is already struggling. The palette is part of the
subscription, not the snapshot.

## D5 — The per-epoch byte log is not the mechanism

Keeping raw bytes since the epoch and replaying them needs no serializer, and
S1 measured why it cannot be the primary path: for output that appends it is
0.8× the serialized size, but for output that redraws it is **243× larger at
ten thousand repaints** and unbounded thereafter, because it is bounded by
everything ever written rather than by what is on screen. Corral hosts
interactive agent TUIs. They redraw.

## Open questions

Ruled by the founder before this ADR is accepted.

**Q1 — How much scrollback does a snapshot carry?** D2 fixes that the bound
exists and is separate; it does not fix the number. A client that gets the
viewport only is cheap and forgets everything a person scrolled back to read.

**Q2 — What does a client do with the history it did not receive?** Ask for
more on demand, show a truncation boundary, or treat the snapshot as the whole
history it will ever have. This decides whether the terminal channel needs a
backfill request at all, which `ARCHITECTURE.md` §3 currently defers.

**Q3 — Is the fuzzing requirement in D1 a release gate or a PR3 gate?** It is
written as a PR3 gate above. The stricter reading is that no build ships to a
person until it passes.

**Q4 — Does a snapshot's size have a hard ceiling, and what happens at it?**
4.29 MB is a measurement, not a limit. A ceiling means deciding what is dropped
when a screen legitimately exceeds it.

## Not decided here

Which channel carries the bytes and how it is framed (`ARCHITECTURE.md` §3
fixes only that it is not the semantic RPC channel). ACK/credit flow control,
remote backpressure, viewport claiming — deferred until remote requires them.
Persisted scrollback: M1 keeps bounded in-memory scrollback only. The lease
seam that decides who may write input.

## Evidence

Spike S1, `docs/references/2026-08-23-s1-vt-serialization.md`, and benchmark
ledger row 5. S1 names what it did not test — cross-implementation parsing,
resize across an epoch, DA/DSR query-reply, real captured streams, Linux — and
Q1 through Q4 should not be read as if it did.
