# Founder Decision Record — a transient-state error code, and PR2's zero-wire boundary

> Status: founder-accepted, 2026-08-23. Narrows the Q4 zero-wire ruling in
> `docs/decisions/2026-08-22-pr2-resume-lineage-acceptance.md` for one
> case. Materialized in the PR2 implementation: `ErrorCode::Busy` in
> `corral-protocol`, and `corral-protocol` entering the PR2 plan's
> `writes:` claim.

## What was asked

Q14 fixed how `corrald` behaves when its registry store cannot answer, and
Q4 ruled PR2 zero-wire. Implementation split the store's failures into two:
a **fatal** conclusion, where the store can no longer vouch for durable
truth, and a **refusal**, where the store is intact and the same call may be
made again — contention being the canonical case.

Fatal is settled: the daemon stops serving. A refusal had nowhere to go.
Protocol 1 defines no code for it, so the implementation closed the
connection without answering — leaving the caller with a lost connection
where the honest answer was "not now, try again". A fresh-context review
pointed out that this was a choice rather than a limitation: the wire
already carries `ErrorCode::Unknown(String)`, the additive-evolution seam
every future code arrives through, so a retryable code could be returned
compatibly to older peers.

## The ruling

**An exception to zero-wire, scoped to this stage.** One error code —
`busy` — is added to protocol 1. A transient refusal is answered rather
than dropped, and the connection stays usable.

## What this does and does not cover

Covered: one additive `ErrorCode` discriminant and its wire spelling; older
peers decode it through the existing unknown-code seam and keep working.

Not covered, and Q4 otherwise stands: no RPC, no `session.list` fields, no
session wire shape, no mutating method, no stream or event wire vocabulary.
Nothing is pre-staged for PR3.

**This does not reopen Q14's ban.** Q14 forbade `StateUnavailable`,
`RepairStore` and `DegradedMode` — the vocabulary of a degraded
alive-but-stateless mode with a UX behind it. `busy` is not that: it names
a moment, not a mode, says nothing about the daemon's health, and has no
surface. PR2 still implements no degraded store mode.

**Why the exception is cheap now.** Wire permanence begins at the first
external tagged release exposing the contract (AGENTS.md §Protocol). Corral
has not made one, so `busy` is still renumberable or removable if a later
phase decides differently — which is what "现阶段" bounds.

## Consequence for the PR2 plan

`corral-protocol` joins the plan's `writes:` claim, and the plan's
zero-wire non-goal and its "no wire diff" definition-of-done point here.
The plan is otherwise unchanged.
