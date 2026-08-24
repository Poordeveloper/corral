# Founder Decision Record — ADR 0003 acceptance

> Status: founder-accepted, 2026-08-24. Materialized by ADR 0003 flipping to
> `accepted` (D6–D9), the `docs/evidence/` documentation class in
> `docs/GOVERNANCE.md` and `AGENTS.md`, and the fuzz-distillation mechanics
> in the Workflow §7 — all in this change set. Ruled across a two-round
> grill of ADR 0003's open questions, with Spike S1
> (`docs/references/2026-08-23-s1-vt-serialization.md`) as the evidence
> base.

Numbering below is the grill's: round one ruled the ADR's open questions
Q2–Q4, round two the numbers and locations those rulings unblocked (Q5–Q8;
Q5 is the ADR's original Q1). Three rulings modified the drafting agent's
recommendation (Q4, Q5, Q8); none were overturned outright. The
modifications carry the reasoning that produced them, because the reasoning
is the part a later implementer needs.

No durable-event or schema acceptance is involved: this ADR introduces no
durable state, and live terminal state is never persisted as fact.

## Round 1

**Q2 — omitted history → (a), sharpened.** M1 snapshots carry the complete
current viewport, bounded recent scrollback, and explicit truncation
metadata; no terminal-history backfill request. The sharpening: the
truncation boundary is a fact, not a promise. Metadata states that history
was not included — never that the daemon still holds it, and never that a
future API can fetch it. "Daemon may retain more than it sends ≠ daemon
guarantees retrievability of everything omitted." Do not design the boundary
as "tap to load more later"; do not pre-build cursor/range contracts in PR3
— if backfill ever ships, it adds retained-range, oldest-available-cursor,
and request semantics then. The UI at the boundary is honest ("earlier
terminal history was not included in this snapshot" — copy later, semantics
frozen). Frozen invariant: *a client may know that history was omitted
without being promised that the omitted history remains retrievable.*
Rejected: (b) premature backfill protocol; (c) dishonest completeness.

**Q3 — fuzz gates → (c), three layers with distinct jobs.**
(1) *PR3 pre-merge deep campaign*, recorded: target, exact commit SHA,
tool/config, duration and/or executions, platform/toolchain,
sanitizer/instrumentation where relevant, crashes found, minimized
reproducers, disposition, final result. Goal: initial confidence — no
panic, no daemon crash, bounded handling of malformed input,
known-pathological sequences exercised. Evidence location left to Q8 rather
than forced into `docs/references/`, to avoid conflating the
benchmark/reference ledger with verification evidence.
(2) *`./scripts/verify` permanent regression layer*: deterministic bounded
corpus — known corpus, minimized crash cases, boundary/pathological
fixtures. Seconds, predictable, merge gate. Explicitly not "fuzz per PR".
(3) *Scheduled deep fuzz*: real randomized discovery; never the sole
evidence of merge-readiness. A scheduled crash → P1 → minimized reproducer
enters the permanent corpus → affected owner's autonomous merge freezes
until triage/fix. Scheduled fuzz's value is distilled back into verify.
Frozen invariant: *every fuzz-discovered regression that matters to
correctness must be converted into deterministic merge-gate coverage.*
Rejected: (a) one-shot campaign then nothing; (b) covering the
merge-critical crash invariant only at release/scheduled layers.

**Q4 — snapshot cap → (a′), MODIFIED.** The draft recommendation froze
"viewport always complete, unconditionally". Ruled instead: two numbers with
strictly separate jobs, and two layers of behaviour.
*Layer one (normal trimming)*: when the current viewport itself fits the
budget — preserve the viewport in full, trim scrollback oldest-to-newest
until the encoded snapshot fits, report actual included history plus
truncation. Scrollback is the first and only history sacrificed.
*Layer two (abnormal viewport)*: if the encoded current viewport alone
exceeds the hard safety ceiling — never truncate half a viewport and
pretend recovery succeeded, never allocate unboundedly, never bypass the
cap. Produce an explicit typed snapshot failure / unsupported-geometry
condition and keep the daemon healthy. An exception path, not a product
experience.
Therefore two numbers: a *target snapshot budget* (what a normal resync
strives to meet, via scrollback trimming) and a *hard snapshot safety
ceiling* (against pathological geometry, style explosion, encoder
explosion, malicious state, unbounded allocation). Both constrain the
actual encoded wire payload — not grid heap estimates, not struct size, not
row-count arithmetic — because what is protected is resync-time memory, the
IPC burst, and client decode cost. Sizing may estimate-then-verify; no
obligatory O(n) per-row re-serialization. If a stricter geometry invariant
is ever proven to bound the viewport below the ceiling, the exception
branch can be retired by that proof — until it exists the ADR must not say
"viewport always fits". Frozen invariant: *snapshot degradation sacrifices
oldest scrollback before the current viewport, but no viewport is allowed
to bypass an absolute safety bound merely because it is "current".*

## Round 2

**Q5 — snapshot scrollback → (a′), 2,000 rows as an experience target,
MODIFIED.** Defined as the *desired maximum* scrollback rows in a normal
snapshot — not a guaranteed minimum, not a fixed history size. Actual
carriage is min(available retained history, 2,000 rows, what fits under the
target snapshot budget); trimming starts from the oldest; the included
count may be < 2,000; truncation metadata declares it honestly. The ADR
must not say "snapshot contains 2,000 lines" — it says "PR3 initially
targets up to 2,000 recent scrollback rows per snapshot, subject to the
encoded snapshot budget". Why 2,000: covers rereading the last command's
output, finding a recent error, scrolling back a few dozen screens; 10,000
looks more generous under no-backfill but tilts every resync's cost toward
history-browsing, a responsibility that is not PR3's. Frozen invariant:
*the row count is an experience target; the encoded-byte budget is the
resource-safety authority.*

**Q6 — daemon retained scrollback → (a), 4 MiB/session, byte-counted.**
Bounded by bytes, not lines; oldest scrollback discarded first when full.
Explicitly not promised: "4 MiB necessarily holds ≥ 2,000 rows" — the
retention memory representation and the snapshot wire encoding are
different unit models, and the ~43 bytes/line *wire* measurement must not
be used to prove what 4 MiB of daemon *memory* stores. The Q5/Q6 relation
is an expectation — "the daemon should normally retain enough recent
history to satisfy the snapshot target under ordinary workloads" — not the
mathematical invariant `retention_rows >= 2000`. Pathological styling or
wide rows shrinking the effective row count means the snapshot sends what
is actually retained and the metadata says so; not a correctness failure.
10 MiB rejected: M1 has no history-backfill consumer, so extra retention
raises per-session resident memory (50 × 10 MiB ≈ 500 MiB vs 50 × 4 MiB ≈
200 MiB worst configured) without improving the attach contract or resync
correctness. 4 MiB is an initial policy default, not a persisted or wire
compatibility contract; adjust later on real memory data.

**Q7 — budgets → (a), 1 MiB target / 16 MiB ceiling, both on the final
encoded payload.** Target = what a normal snapshot (viewport + up to 2,000
recent rows) tries to land under; over it, trim oldest scrollback, keep the
viewport, update metadata. Ceiling = not a second target; it protects
against pathological geometry, extreme style state, encoder explosion,
corrupted/malicious terminal state, accidental unbounded allocation;
viewport-only above it → explicit snapshot-too-large failure, daemon
healthy, no partial viewport masquerading as success. The "4K ≈ 70k cells
≈ 3.2 MB" estimate is engineering rationale for the initial 16 MiB, never
a correctness proof; PR3 must carry a test over an approved
large-geometry/styling case proving normal extremes land clearly under the
ceiling. Priority: hard safety ceiling > normal encoded target > 2,000-row
experience target; retention availability may also reduce actual snapshot
history. Freeze level of all four numbers (2,000 rows / 4 MiB retention /
1 MiB target / 16 MiB ceiling): initial policy defaults, not wire
constants.

**Q8 — fuzz evidence location → (a′), `docs/evidence/`, with admission
criteria, MODIFIED.** Canonical categories: `docs/references/` = external /
research / benchmark / spike evidence; `docs/evidence/` = repository-owned
verification evidence supporting a merge, release, or governance claim. The
PR3 initial deep campaign lands at
`docs/evidence/pr3-terminal-fuzz-<date-or-commit>.md`. The sharpening: the
directory is not a CI-run history database. Routine successful scheduled
runs produce no repository files — the CI/artifact system owns run history.
Admitted only: (1) a milestone campaign used as explicit merge/release
evidence; (2) a material newly discovered correctness failure; (3) a
significant campaign closing a P1, quarantine, or release blocker; (4)
explicitly required long-running verification evidence. A fuzz crash's
durable product is not the log but the minimized deterministic reproducer
in the permanent corpus under `./scripts/verify`. Division of labour:
`docs/evidence/` keeps "why we were entitled to this engineering
judgement"; the verify corpus keeps "how the same failure is mechanically
prevented from returning". Governance defines the one class without
enumerating future subtypes. The directory addition is a canonical
documentation-structure change — a human-reviewed governance edit — but not
a new product architecture decision and not a reopening of workflow
governance.

## The closing formulation

> 2,000 rows is what we try to give the user.
> 1 MiB is what a normal resync should try to cost.
> 16 MiB is what no successful snapshot may exceed.
> 4 MiB is how much recent scrollback the daemon initially budgets per
> session.
