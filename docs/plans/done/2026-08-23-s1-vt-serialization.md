---
status: done
class: B
writes: [docs/references, docs/references/architecture-benchmarks.md]
reads: [ROADMAP.md, ARCHITECTURE.md, docs/references/architecture-benchmarks.md]
---

# S1 — VT serialization spike

## Goal

Close the one gap ledger row 5 names: **which VT implementation can hold
authoritative screen state and serialize it back to ANSI such that a client
parser reproduces an identical screen** — or prove that none can and the
per-epoch raw byte log is the answer.

The wire model is already decided and is not in scope: daemon-owned
authoritative VT, snapshot @ seq N + sequenced raw deltas, resync-by-snapshot
only, resize ⇒ new snapshot epoch, PTY bytes replayed unmodified
(`ARCHITECTURE.md` §4, ledger row 5). This spike selects the engine under it.

## Non-goals

No production code. No ADR 3 — the founder accepts that on this evidence, and
this document does not pre-empt it. No PR3 work. No emulator committed by the
act of measuring it.

## Method

S2's bar: every claim in the report comes from a run performed for this spike,
on named versions, not from documentation or memory. A crate's README claiming
a serializer is a reason to test it, never a finding.

The chain under test, per ROADMAP §3 S1:

```text
PTY bytes → authoritative VT → ANSI snapshot → client parser → screen
                    │                                            │
                    └──────────── compared ──────────────────────┘
```

Fidelity is decided by comparing the authoritative grid against the grid a
client rebuilt from the snapshot alone: every cell's character, width, and
style, plus cursor position, cursor visibility, alternate-screen flag, title,
and scrollback extent. A dimension passes only when the two are identical.

**Cross-check.** Daemon and client will run the same implementation in
production, so a self-comparison can be fooled by an engine that is wrong in
the same way twice. Each survivor's snapshot is therefore also parsed by a
*different* implementation and compared, and disagreements are reported as
findings rather than averaged away.

## Corpus

Real byte streams captured from real programs under a real PTY, plus targeted
sequences for the dimensions a captured stream may not exercise. ROADMAP fixes
the list: scrollback, resize, alternate screen, cursor state, OSC title/color,
colors, wide chars, Unicode, query/reply, snapshot restore.

Captured: a shell session with scrollback overflow; `vim` (alternate screen,
cursor shapes); a full-colour TUI; a CJK/emoji/combining-mark text dump; a
program that issues DA/DSR queries. Synthetic sequences fill any dimension the
captures leave untested, and the report says which were which.

## Candidates

Triaged on whether the implementation can serialize state back to ANSI at all;
survivors get the full chain.

- `alacritty_terminal` 0.26 — ledger row 5 records that it has no
  re-serializer. Measured, not assumed, and if confirmed, measured again for
  whether one can be written over its grid.
- `vt100` 0.16 and the forks that exist because someone needed exactly this.
- `termwiz` 0.23.
- The pure-Rust Ghostty ports that have appeared since the ledger was written
  (`qwertty-term-vt`, `vtcode-ghostty-core`). If one is real, the Zig question
  the ledger left open may not need answering.
- `ghostty-vt` via Zig — the Herdr reference, and the reason the ledger says
  "Zig dependency neither accepted nor rejected".
- Per-epoch raw byte log — the fallback that needs no serializer, measured on
  the same corpus for snapshot size and bounded memory.

## Measured per survivor

Fidelity per dimension; snapshot bytes and serialization time at the scrollback
depths row 5 names as wire-contract numbers (10k default, 100k max); memory per
live session; behaviour across a resize epoch; what the build costs (toolchain,
compile time, platform reach — macOS and Linux both, since ADR 5 fixes the
platform scope).

## Where the code lives

The harness is throwaway and stays out of the repository: the deliverable is
evidence, not a component, and adding a `spikes/` tree is a structural decision
nobody has taken. Reproducibility comes from the report carrying the harness's
comparison core and corpus definitions verbatim, the way S2 carried its captured
payloads.

## Deliverable

`docs/references/2026-08-23-s1-vt-serialization.md` — versions, method,
per-candidate findings with real output, and a recommendation with its
reasoning. Ledger row 5's gap and confidence lines updated in the same change.

A recommendation, not a decision: ADR 3 is where it becomes law, and that needs
explicit founder acceptance.

## Definition of done

- Every ROADMAP-named dimension has a stated result for every survivor, or a
  stated reason it could not be tested.
- The cross-implementation check ran, and disagreements are in the report.
- The per-epoch byte-log fallback is measured, not dismissed.
- The report names what would change the recommendation.
- Plan moves to done/ on land.
