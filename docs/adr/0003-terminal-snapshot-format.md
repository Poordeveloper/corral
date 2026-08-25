---
status: accepted
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
`ROADMAP.md` §3 for PR3. Acceptance evidence:
`docs/decisions/2026-08-24-adr3-terminal-snapshot-acceptance.md`.

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

**So the emulator is fuzzed against malformed PTY output before PR3 ships;
D9 fixes the gates.** `ARCHITECTURE.md` §5 requires that malformed provider
data degrade a session rather than panic `corrald`, and a `catch_unwind`
cannot contain undefined behaviour — only evidence that the parser survives
hostile input can. A crash found later is a bug; memory unsafety found later
is a security finding.

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
lines; how that bound relates to what the daemon retains is fixed in D7. The
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

## D6 — Omitted history is a fact, not a promise

A snapshot carries the complete current viewport, a bounded run of recent
scrollback, and explicit truncation metadata: how many scrollback rows are
included, and whether history existed before them that was not. A client
therefore knows that the oldest line it can scroll to is not the start of the
session's history, and its surface shows that boundary honestly (the copy is
presentation; the semantics are frozen).

What the metadata does not say is that the omitted history is still there.
The daemon may retain more than it ships and still discard it later; the
truncation flag states what this snapshot did not include, never what a
future request could recover.

> A client may know that history was omitted without being promised that the
> omitted history remains retrievable.

There is no history backfill request in M1. If one is added later, it adds
its own contract then — retained range, oldest-available cursor, request
semantics. PR3 does not pre-build a cursor or range shape for an API that
does not exist.

Rejected: a premature backfill protocol, and the dishonest alternative of
presenting the snapshot as the whole history there ever was.

## D7 — The numbers, and what kind of numbers they are

**A snapshot targets up to 2,000 most-recent scrollback rows.** An experience
target, not a guaranteed minimum and not a fixed size: what a snapshot
actually carries is

    min( retained history, 2,000 rows, what fits the encoded budget )

and the truncation metadata of D6 reports the actual count. 2,000 rows covers
what attach and resync actually interrupt — rereading the last command's
output, finding a recent error, scrolling back a few dozen screens — without
turning the snapshot into a terminal-history archive, which is a job PR3 does
not have. (tmux has defaulted to 2,000 for decades; at S1's measured unit
cost it is ~86 KB typical.)

**The daemon initially budgets 4 MiB of retained scrollback per session,
counted in bytes** — the chosen emulator's native model — discarding
oldest-first when full. No row equivalence is promised: the in-memory
representation and the wire encoding are different unit models, and S1's
~43 bytes/line wire measurement must not be read as proof of what 4 MiB of
retention holds. The daemon should normally retain enough recent history to
satisfy the snapshot target under ordinary workloads; when pathological
styling means it holds fewer rows, the snapshot ships what is actually
retained and says so. That is honest degradation, not a correctness failure.

Retention is 4 MiB rather than more because M1 has no consumer for the
excess: with no backfill (D6), history beyond what snapshots carry is
unreachable, so a larger budget buys resident memory in a many-session daemon
and nothing else. Fifty sessions budget ~200 MiB; start there and let
dogfood data move it.

All four numbers in this ADR are initial policy defaults, not wire constants.
The wire declares actuals; changing a default later is not a compatibility
event.

> The row count is an experience target. The encoded-byte budget is the
> resource-safety authority.

## D8 — Two budgets, strictly separated

Both budgets constrain the final encoded wire payload — not estimated grid
heap size, not row counts — because what they protect is the resync-time
burst: daemon memory, IPC, client decode.

**The normal target is 1 MiB encoded.** A snapshot — viewport plus up to
2,000 recent rows — tries to land under it. Over target, the oldest
scrollback is trimmed until it fits, the viewport is kept, and the metadata
reports what was actually included. The implementation may size by estimate
and verify the final encoding; nothing obliges a row-by-row re-serialization
loop.

**The hard safety ceiling is 16 MiB encoded.** It is not a second target. It
exists for pathological geometry, extreme style state, encoder explosion,
corrupted or malicious terminal state, and accidental unbounded allocation.
If the viewport alone encodes above it, the daemon produces an explicit typed
snapshot-too-large failure and stays healthy. No partial viewport is ever
shipped masquerading as a successful snapshot.

Priority when the constraints collide:

    hard safety ceiling  >  normal encoded target  >  2,000-row target

and retention availability (D7) may independently lower the history a
snapshot carries.

The estimate that sizes the ceiling — a 4K full-screen grid of ~70k cells
with a style change per cell encodes near ~3.2 MB — is engineering rationale
for 16 MiB, not a correctness proof, and this ADR deliberately does not claim
the viewport always fits. PR3 carries a test over an approved
large-geometry/styling case showing legal extremes land well under the
ceiling; if the implementation later has explicit geometry limits, a real
bound proof may retire the failure branch through an ADR revision.

> Snapshot degradation sacrifices oldest scrollback before the current
> viewport. But no viewport bypasses the absolute safety bound merely because
> it is current.

In one breath: 2,000 rows is what we try to give the user. 1 MiB is what a
normal resync should try to cost. 16 MiB is what no successful snapshot may
exceed. 4 MiB is how much recent scrollback the daemon initially budgets per
session.

## D9 — The fuzz requirement is three layers, each with its own job

D1 requires evidence that the emulator survives hostile input. That
requirement has three parts, and none substitutes for another.

**Before PR3 merges: one recorded deep campaign.** It establishes the
initial confidence — no panic, no daemon crash, bounded handling of malformed
input, known-pathological sequences exercised — and its record lands at
`docs/evidence/pr3-terminal-fuzz-<date-or-commit>.md` with the target, the
exact commit SHA, tool and configuration, duration and/or executions,
platform and toolchain, sanitizers where relevant, crashes found, minimized
reproducers, disposition, and the result.

**In `./scripts/verify`, permanently: a deterministic bounded corpus.** Known
corpus, minimized crash cases, boundary and pathological fixtures — seconds
to run, predictable, part of the merge gate. This layer is not "fuzzing per
PR"; it is the regression floor every PR must clear.

**On schedule: real randomized deep fuzzing.** It exists to discover, and it
is never the only coverage of a merge-critical invariant (AGENTS
§Verification). A scheduled crash is a P1; its minimized reproducer joins the
permanent corpus; the affected owner's autonomous merge freezes until triage
(Workflow §7).

Routine successful scheduled runs produce CI artifacts, not repository files.
`docs/evidence/` admits only: a milestone campaign used as explicit merge or
release evidence; a material newly discovered correctness failure; a
significant campaign that closes a P1, quarantine, or release blocker; or
explicitly required long-running verification evidence. `docs/evidence/`
records why we were entitled to an engineering judgement; the verify corpus
records how the same failure is mechanically kept from returning.

> Every fuzz-discovered regression that matters to correctness is converted
> into deterministic merge-gate coverage.

## Not decided here

**Whether a screen the child reshaped propagates to the pty.** DECCOLM
(`ESC[?3h`) makes the emulator 132 columns wide without anyone asking the
kernel. Corral follows it in what it publishes and opens an epoch, so no
client is told a size the screen does not have — but the pty's own `winsize`
is deliberately left alone, so a child that re-queries `TIOCGWINSZ` reads the
size Corral gave it rather than the one it just set. A real terminal has one
object and no such gap; Corral has two, and which of them the child's query
should answer is a question this ADR does not settle. The divergence self-heals
on the next explicit resize. Raised by the PR3 fuzz campaign,
`docs/evidence/pr3-terminal-fuzz-2026-08-24.md`.

Which channel carries the bytes and how it is framed (`ARCHITECTURE.md` §3
fixes only that it is not the semantic RPC channel). ACK/credit flow control,
remote backpressure, viewport claiming — deferred until remote requires them.
Persisted scrollback: M1 keeps bounded in-memory scrollback only (sized in
D7). The lease seam that decides who may write input. No durable state is
introduced: live terminal state is runtime-owned and never persisted as fact
(AGENTS §Durable state); this ADR adds no durable schema or event.

## Evidence

Spike S1, `docs/references/2026-08-23-s1-vt-serialization.md`, and benchmark
ledger row 5. S1 names what it did not test — cross-implementation parsing,
resize across an epoch, DA/DSR query-reply, real captured streams, Linux —
and D6 through D9 rest on founder rulings over that evidence, not on
measurements S1 did not make. Acceptance:
`docs/decisions/2026-08-24-adr3-terminal-snapshot-acceptance.md` (two
rounds: Q2–Q4, then Q5–Q8).
