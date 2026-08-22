# Founder Decision Record — ADR 0002 acceptance and the PR2 semantic boundary

> Status: founder-accepted, 2026-08-22. Materialized by ADR 0002 flipping
> to `accepted`, the `ARCHITECTURE.md` §1/§11 sync, and the PR2 plan
> unblocking — all in this change set. Ruled across a three-round grill
> (Q1–Q15) of ADR 0002 and the PR2 plan, with Spike S2
> (`docs/references/2026-08-22-s2-session-identity-verification.md`) as
> the only new evidence.

Two rulings overturned the drafting agent's recommendation (Q1, Q7), four
modified it (Q5, Q9, Q12, Q13), one added a missing failure path (Q14).
Those are recorded with the reasoning that produced them, because the
reasoning is the part a later implementer needs.

## Explicit durable-event acceptance

Required separately by AGENTS.md §Durable state — never implied by ADR
status. The founder explicitly accepts these additions to the durable
semantic event set:

- **`RunStarted`**
- **`RunEnded`**
- **`SessionForkedFrom`**

Not accepted, and deferred to the phase that first has a producer *and* a
consumer: `BindingSuperseded`, any assurance-change event, correction
events, archive/delete events.

## Round 1

**Q1 — Does a heuristic-only Session have a Run? → OVERTURNED.** The draft
said no Run. Ruled: that welds runtime existence, identity assurance, and
control eligibility back together after the architecture spent effort
separating them. Three concepts are frozen orthogonal — Session (logical
identity/lineage), Run (a concrete runtime occurrence), binding
assurance/control eligibility (how sure Corral is, and whether it may
act). Therefore: history/heuristic session evidence with no independent
runtime evidence → Session with zero Runs; runtime observed but
association only Heuristic → **Run exists**, Heuristic binding, control
unavailable, semantic status may be Unknown; Deterministic/Attested →
control follows existing law. No second-class "non-control Run": Runs
carry no grade, bindings do. The failure this prevents is concrete — at
PR7 external discovery, "I can see the process but only suspect its
Session" must not force the model to discard the fact that the runtime
exists, which would contradict the frozen principle *preserve what we know
instead of collapsing everything to Unknown*.

**Q2 — NativeResume continuity. → ACCEPTED.** D3 stands as drafted;
recognition is `(node, provider, external_id, kind)` binding uniqueness.
The blocking unknown becomes resolved evidence citing S2. No heuristic
continuity fallback is designed. Sole reopen condition: supported
provider/version matrix re-verification failing to uphold native identity
semantics — an obligation of provider integration's version matrix, not a
speculative fallback carried in PR2.

**Q3 — Run lifecycle vs attachment events. → ACCEPTED.** Both pairs kept:
`RunStarted`/`RunEnded` for Run lifetime, `RunAttached`/`RunDetached` for
attachment availability. `Started, Attached, Detached, Attached, Detached,
Ended` is a legal history, so the pairs cannot merge. Plus the explicit
durable-event acceptance above.

**Q4 — Does PR2 touch the wire? → ACCEPTED (a): zero wire change.** PR2 is
daemon/core-internal semantic foundation only: Session, Run, Binding,
assurance, identity resolution, durable state, receipt/idempotency
foundation, lineage semantics. No RPC, no `session.list` fields, no
session wire shape, no mutating methods, no stream/event wire vocabulary.
`session.list` keeps returning what it can truthfully provide; if that is
`[]`, it stays `[]`. Nothing is pre-staged for PR3. The ADR's dangling
"D9" reference is an editorial defect, fixed without a decision.

**Q5 — Fork lineage assurance. → ACCEPTED WITH CUT.** The split is law:
Corral-initiated fork (it knows the parent external id, its own operation,
and the child) → Deterministic → `SessionForkedFrom` may be written;
externally observed Claude fork → evidence names no parent (S2:
`source: "fork"` only, zero parent references in the forked transcript),
uuid/transcript overlap is Heuristic → **no durable edge**. Cut from PR2:
lineage proposal objects, pending-confirmation state, manual-confirmation
workflow — none has a consumer. Diagnostic heuristic evidence may be
retained. The law: *Heuristic similarity may suggest lineage, but MUST NOT
create a `SessionForkedFrom` fact.*

**Q6 — Missing needs-input vocabulary. → ACCEPTED (a).** ROADMAP §3 lists
"needs-input request + actionable-status vocabulary" under PR2 and the
plan omitted it; the plan was wrong, not the ROADMAP. PR2 establishes the
minimal `NeedsInputRequest` and `AttentionItem`/actionable-status domain
vocabulary per the frozen `ARCHITECTURE.md` §2 concepts — not as
"placeholder types" but as core domain meaning. Strictly forbidden in PR2:
wire representation, durable representation, notification, scoring, the
Attention Engine, UI projection, provider-hook mapping, speculative
fields. *PR2 owns the shared domain meaning, not the later evidence
ingestion or attention behavior.*

**Q7 — The 60-line plan hard cap. → OVERTURNED, AND SPLIT OUT.** The
recommendation to raise the cap to 150 was rejected: do not replace one
failed absolute with another. The cap is deleted; ~150 lines becomes a
size target whose breach requires a `Plan Size Justification` ruled on by
the fresh reviewer, never a line-count CI gate. Governance change, not
part of ADR 0002 — landed separately in
`docs/decisions/2026-08-22-plan-size-budget.md` with a `## Transition`
covering the PR0/PR1/PR2 plans.

## Round 2

**Q8 — Which object carries the Run↔Session association? → ACCEPTED (a).**
Reuse the existing `RuntimeBinding`; add no second edge type. Frozen: Run
= concrete runtime occurrence; RuntimeBinding = Session ↔ that runtime's
external identity; `RuntimeBinding.assurance` = how sure Corral is of the
association; control eligibility continues to follow existing law. Not
created: `RunSessionBinding`, `RunAssurance`, control-level Runs — each
would manufacture a second assurance owner. A Run may reference `RunId`,
`SessionId`, and `RuntimeBindingId`, but the `SessionId` on a Run is a
structural reference to its current association only; its trustworthiness
is the referenced binding's assurance. Forbidden shape:

```text
if run.session_id exists { control_allowed = true }
```

Control always resolves through binding assurance and the existing Control
facet policy. Canon (three sentences, verbatim in ADR 0002 and
ARCHITECTURE §1):

> A Run records a concrete runtime occurrence. Its RuntimeBinding relates
> that runtime to a Session and carries the assurance of that association.
> Run existence alone never grants control eligibility.

**Q9 — Minimum evidence to mint a Run. → ACCEPTED IN PRINCIPLE, PID
REMOVED FROM THE LAW.** A Run may be created only from independent
authoritative evidence that a concrete runtime occurrence exists or
existed, in two classes: constructive (Corral created the runtime) and
authoritative node-local runtime observation (through the node's accepted
runtime-observation mechanism). For host-native M1 the latter is typically
process identity plus OS-level liveness/start evidence — but domain law
must not read "a Run always requires a PID", or a future runtime owner
with a stronger authoritative handle would be forced to impersonate one.
Never sufficient alone: hook event, transcript/history line, cwd/time
correlation, or a provider semantic event that merely implies a process
probably existed. *A hook having fired ≠ the runtime is alive.* PR2
introduces no Session-less Run: a process with no candidate Session is
provisional discovery state; Run + RuntimeBinding begins once an
association target exists — and that association need not be trustworthy
(known runtime + Heuristic candidate → Run exists, no control).

**Q10 — Do heuristic Runs enter the durable log? → ACCEPTED, WITH THE
BACKFILL SEMANTICS CORRECTED.** A heuristic-bound Run may exist as live
runtime state but must not emit durable `RunStarted`/`RunEnded` under that
Session, because writing `Session S / RunStarted R` durably asserts "R
belongs to S" — a guess promoted to fact. Lifecycle facts become durable
only at Deterministic/Attested/Manual. The correction the founder
insisted on: **no retroactive insertion**. Event seq stays append-order;
if assurance is established later, append then. Two time semantics are
distinct — event sequence = when Corral accepted the fact; occurrence time
= when the runtime fact actually happened. So this is legal and must be
readable as legal:

```text
seq 40  BindingConfirmed
seq 41  RunStarted(occurrence 20 minutes ago)
```

Any historical occurrence timestamp on a later-appended lifecycle fact
must be independently supported by authoritative runtime evidence; if the
real start time is unknown, `first_observed_at` is never dressed up as
`started_at`. And a Run that ended while still heuristically bound gains
durable `Started`/`Ended` only if authoritative runtime evidence still
supports those facts — later confirmation of the association never
auto-promotes earlier heuristic runtime metadata to durable truth. PR2
adds no correction event, unlink-correction flow, or manual-link workflow.
Rule: *Durability follows fact assurance, not object existence.*

**Q11 — Landing mechanism. → ACCEPTED, WITH AN ORDERING CONSTRAINT.** Two
PRs. **PR A** (this one, Class C, human merge): ADR 0002 rewritten and
flipped to accepted; this acceptance record with the explicit event
acceptance; ARCHITECTURE synced (Run canon, Run existence ≠ control
eligibility, RuntimeBinding owns association assurance, runtime truth not
inferable from semantic evidence) using existing vocabulary — no new
"ControlFacet" noun, the existing *Control facet* and *control-capable
runtime binding* stand; PR2 plan updated and unblocked. The constraint:
the plan's status may only become ready **inside the same change set** as
the ADR acceptance — never marked ready first while the ADR is still
pending. **PR B**: the plan-size governance cleanup, separate, with its
own Transition section; not mixed into PR A.

## Round 3

**Q12 — Same command id, different payload. → ACCEPTED, DEFINITION
REPLACED.** Conflict must be rejected: same id + same command fingerprint
→ return the original receipt, do not re-execute; different fingerprint →
`CommandIdConflict`, execute nothing, leave the original receipt
untouched. But *not* defined as byte-identical payloads: idempotency binds
to command semantics, not to one serialization's incidental bytes. PR2 has
no wire at all; writing raw bytes into canon would let field ordering,
serializer changes, or encoding changes split semantically identical
commands — and would smuggle wire representation into core correctness one
question after Q4 ruled zero-wire. Frozen: a command fingerprint covers at
least the command kind and every semantic input affecting the mutation,
and excludes serialization formatting, tracing metadata, transport
metadata, and retry timestamps. Storage may keep a typed/canonical command
representation or a stable digest of it; the encoding and hash algorithm
are implementation detail. Invariant: *A command id identifies one
immutable semantic command. Once a command_id has been associated with
command X, it can never later mean command Y.* Required tests: (1) first
execution mutates once and stores a receipt; (2) same id + same semantic
command returns the same receipt with no second mutation; (3) same id +
different semantic command → `CommandIdConflict`, receipt unchanged, no
mutation; (4) equivalent representations of one command, where
representation can vary, must not conflict merely because encoding
differs. `CommandIdConflict` is a state/domain error in PR2; PR3's first
mutating RPC owns its wire mapping.

**Q13 — Command id uniqueness scope. → ACCEPTED, RENAMED.** Not
"daemon-global": `command_id` belongs to the **node's durable command
namespace**, unique across Session, Run, client, connection, and `corrald`
process restart. So the receipt table keys on `command_id` alone, not
`(session_id, command_id)`. Two reasons: `corral new` has no target
Session before it executes, so a per-session namespace fails at the very
first mutation; and more importantly a daemon restart must not reset
uniqueness, or daemon A executes UUID-X, crashes, and daemon B treats
UUID-X as new — executing the mutation twice and destroying the point of
receipts. UUID is the recommended generation form, but correctness does
not rest on UUIDs never colliding: a real collision resolves through
Q12 — same fingerprint → retry, different fingerprint → conflict. PR2
designs no cross-node receipt database; the guarantee is *within one
Corral node's durable state, one command_id has exactly one semantic
meaning.*

**Q14 — Fail-closed shape for an unusable store. → ACCEPTED (a), WITH THE
RUNTIME PATH ADDED.** Zero-wire holds; `corral-protocol` leaves the PR2
`writes:` claim. Two moments, both required. **Startup**: if the DB cannot
open, the schema cannot initialize, integrity validation fails, or the
state directory is unusable, `corrald` must not enter protocol-ready
serving state — no successful hello, process exits as an internal/state
startup failure, and the client resolves through PR1's existing activation
failure path. Ordering matters: the state subsystem must be usable
**before** the daemon advertises PR2 serving readiness; "hello succeeds,
then a millisecond later we discover the DB will not open" is forbidden.
**Runtime**: if an already-ready daemon hits an unrecoverable state
failure — detected corruption, persistent I/O failure, an invariant
violation making projections untrustworthy — it fails closed the same way:
stop serving state-dependent truth, move to fatal shutdown, established
callers see connection loss, exit non-zero. The next activation retries
initialization; if the problem persists it still cannot become ready. PR2
implements no degraded "daemon alive but state unavailable" mode, and adds
no `StateUnavailable`, `RepairStore`, or `DegradedMode` vocabulary — none
has a consumer with a UX yet. Whether ENOSPC is permanently fatal, and
which SQLite errors are transient, is implementation error mapping; but
once the state layer concludes it can no longer vouch for durable truth,
it must not return a normal-looking projection. Plan sentence, verbatim:
*Fail closed applies both before readiness and after an unrecoverable
runtime state failure; PR2 never continues to serve state-derived claims
from an untrusted store.* Tests: cannot-open store → no successful
readiness; corrupt/invalid store → no successful readiness; injected fatal
state failure after ready → the daemon stops serving rather than returning
trusted-looking state.

**Q15 — Projection mutations without events. → ACCEPTED (a), GENERALIZED
INTO AN INVARIANT.** *Every persistent projection mutation must be
justified by an accepted durable semantic event.* If the accepted
vocabulary cannot express a change, that persistent mutation is out of
scope and the owning future phase must extend the event set first. So PR2
Design 4 drops/defers process-only binding supersession and
assurance-downgrade persistence: neither has a producer in PR2, neither
has a corresponding event, and implementing them early would let the
projection know more facts than the log — destroying rebuildability. PR2
implements only what the current event set can express: identity
uniqueness resolution, idempotent rediscovery, accepted
binding/session/run mutations, transactional event + projection update.
Persistent projection and live evidence state stay distinct — later phases
may re-evaluate evidence in live state freely; only the durable write is
gated. ARCHITECTURE's supersede and assurance semantics remain valid law;
an accepted semantic rule simply does not oblige PR2 to build a mutation
path with no producer. Required mechanical test: replaying the event log
into empty projections reproduces the same persistent session/binding/run
projection state, covering every durable transition PR2 can produce. A
projection field that changed but cannot be derived from replay is an
architecture violation. Canon: *The event log owns durable semantic facts.
Projections may summarize those facts; they may not silently acquire
additional durable truth.*

## Scope guard

PR2 owns: Session/Run/Binding domain types, assurance, identity
resolution, the durable store with its event log, receipt/idempotency
foundation, lineage semantics, needs-input/actionable-status vocabulary.

PR2 does not own, and must not pull forward: any wire change; PTY and
runtime ownership (PR3); the TUI (PR4); hooks and provider evidence (PR5+);
discovery and supersession producers (PR7); the Attention Engine (PR8);
lineage proposals, correction flows, degraded-store UX, and cross-node
receipts (unscheduled, each with its own future phase).

## Frontier

Closed. The remaining PR2 questions — SQLite pragmas, hash algorithm,
receipt table columns, test clocks — are implementation planning and
review, not founder decisions.
