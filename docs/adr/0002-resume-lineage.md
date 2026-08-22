---
status: accepted
read_when:
  - changing what a Run is, or how Runs relate to a Session
  - deciding whether a newly seen provider session continues an existing one
  - adding a durable session event or changing the event set
  - implementing NativeResume, ContextHandoff, or RuntimeMove
  - exposing lineage on the wire or in a surface
---

# Resume lineage: a Session outlives the processes that run it

`ARCHITECTURE.md` §1 fixes the outcome — a resumed provider session is the
**same Session with a new Run**, and `NativeResume`, `ContextHandoff` and
`RuntimeMove` are never collapsed into a generic "resume". This ADR fixes
the mechanics. Scheduled by `ROADMAP.md` §3 for PR2. The durable-store
shape it writes into is already founder-accepted
(`docs/references/architecture-benchmarks.md` row 44). Acceptance evidence:
`docs/decisions/2026-08-22-pr2-resume-lineage-acceptance.md`.

**The invariant.** A Session's identity never depends on any process that
ran it. A Run is one concrete runtime occurrence of a Session. Nothing
about a Run — its pid, its terminal, its start time, its provider-side
id — is ever promoted into Session identity. And the three concepts stay
orthogonal:

> A Run records a concrete runtime occurrence. Its RuntimeBinding relates
> that runtime to a Session and carries the assurance of that association.
> Run existence alone never grants control eligibility.

Run existence, identity assurance, and control eligibility are three
separate facts. Assurance lives on the binding, never on the Run: there is
no second assurance carrier and no second-class "non-control Run". Control
continues to follow the accepted law — only Deterministic, Attested, or
Manual bindings drive control, and at most one control-capable runtime
binding is active per Session.

## D1 — A Run is a record with a Corral-minted id

`RunId` (UUID), owned by Corral, unique across Sessions. Runs carry an
ordinal within their Session for display only.

Durable events and runtime bindings must name a specific episode, and a
display ordinal cannot serve: correcting a wrong binding renumbers it, and
a renumbered reference is a rewritten fact. Rejected: an ordinal index; a
`(session, started_at)` composite, which is not unique under clock
adjustment and encodes a timestamp as identity.

## D2 — What begins and ends a Run

A Run may be created only from **independent authoritative evidence that a
concrete runtime occurrence exists or existed**. Two evidence classes
qualify:

- **constructive** — Corral created/spawned the runtime and therefore owns
  the occurrence fact by construction;
- **authoritative node-local runtime observation** — Corral independently
  observes the runtime through the node's accepted runtime-observation
  mechanism. For host-native M1 that is typically process identity plus
  OS-level liveness/start evidence, but the law names evidence classes,
  not pids: a future runtime owner with a stronger authoritative handle
  must not be forced to impersonate one.

Never sufficient alone: a hook event, a transcript or history line,
cwd/time correlation, or any provider semantic event that only implies "a
process probably existed". These are identity evidence, and semantic
evidence never implies live runtime truth — a hook having fired does not
mean the runtime is alive now (AGENTS.md §Runtime truth).

The orthogonality plays out in three states:

- provider/history evidence only, no independent runtime evidence → the
  Session exists with **zero Runs**;
- runtime observed, association to a Session only Heuristic → the **Run
  exists** in live state, its RuntimeBinding carries Heuristic assurance,
  control is unavailable, semantic status may be Unknown. The model keeps
  "the runtime exists" as a fact instead of collapsing it because identity
  is weak;
- Deterministic/Attested/Manual association → control eligibility follows
  the existing assurance/control law.

There is no Session-less Run. An observed process with no candidate
Session is provisional discovery state (`ARCHITECTURE.md` §1 binding
invariants), not a Run; the Run + RuntimeBinding model begins once at
least one association target exists.

A Run ends when the process is observed to have exited, recording the
cause when determinable. When `corrald` cannot establish that it exited,
the Run ends as **unverifiable** — never assumed exited.

## D3 — NativeResume: same Session, new Run

Recognition is binding resolution, not inference. Binding uniqueness on
`(node, provider, external_id, kind)` resolves a seen provider session to
its existing Session; a new Run opens under it. Heuristic correlation may
never create this edge.

**Resolved first-party.** Spike S2
(`docs/references/2026-08-22-s2-session-identity-verification.md`)
verified that both current providers keep the session id stable across
resume: Claude Code 2.1.239 (`--resume`, `--continue`) and codex-cli
0.145.0 (`exec resume`) continue the same id and append the same
transcript/rollout file. Recognition by binding uniqueness therefore works
as drafted, and **no heuristic continuity fallback exists**. The sole
reopen condition is the supported provider/version matrix re-verification
failing to uphold native identity semantics; that obligation lives with
provider integration's version matrix (PR5+), not as a speculative
fallback in PR2.

## D4 — ContextHandoff: a new Session with a recorded lineage edge

Handing context into a fresh provider session produces a **new Session**,
with a durable Corral-owned edge to its predecessor. Both can be live and
independently actionable at once, and `PRODUCT.md` §6 requires one row per
independently actionable branch.

The edge is a Corral-owned fact between two Sessions, not a binding:
bindings relate a Session to an external identity. Rejected: modelling a
handoff as a new Run of the same Session, which would put two live agents
behind one row and make "interrupt this Session" ambiguous.

**The edge's assurance is split, and the split is law.** When Corral
itself initiates the fork/handoff it knows the parent external id, its own
operation, and the resulting child — Deterministic lineage, and
`SessionForkedFrom` may be recorded. When a fork is observed externally,
the evidence does not name the parent: S2 verified that Claude's
`--fork-session` reports only `source: "fork"`, and the forked transcript
holds zero references to the parent session id — message-uuid overlap is
Heuristic by definition. Therefore:

> Heuristic similarity may suggest lineage, but MUST NOT create a
> `SessionForkedFrom` fact.

PR2 builds no lineage-proposal object, pending-confirmation state, or
manual-confirmation workflow — none has a consumer yet. Diagnostic-level
heuristic evidence may be retained; whether to surface candidates is a
later phase's decision, taken with its UI.

## D5 — RuntimeMove: same Session, new Run, new runtime binding

The process changes, so the Run changes; the provider session does not, so
the Session does not. At most one control-capable runtime binding is active
per Session (`ARCHITECTURE.md` §1), so the previous binding ends or is
explicitly superseded before the new one is acquired.

## D6 — What is recorded durably

Added to the accepted event set: `RunStarted`, `RunEnded`,
`SessionForkedFrom`. Alongside the already-accepted `SessionCreated`,
`BindingAdded`, `BindingConfirmed`, `RunAttached`, `RunDetached`,
`CommandAccepted`. **This expansion is explicitly founder-accepted** in
the acceptance record — never implied by the ADR's status alone.

`RunStarted`/`RunEnded` are the process episode; `RunAttached`/`RunDetached`
are a runtime binding becoming available or not. They are different facts —
`Started, Attached, Detached, Attached, Detached, Ended` is a legal
history, which is the whole point of "closing a surface never terminates
managed work" — so the pairs are never merged.

**Durability follows fact assurance, not object existence.** Writing
`RunStarted` into Session S's stream durably asserts "this Run belongs to
S". Under a Heuristic RuntimeBinding that assertion is a guess, so:

- a heuristic-bound Run exists as live runtime state only and **must not
  emit durable `RunStarted`/`RunEnded` under that Session**;
- lifecycle facts enter the log only once the association is
  Deterministic, Attested, or Manual;
- the log is **append-only in seq order**. If assurance is established
  later, the facts are appended then (`BindingConfirmed`, then
  `RunStarted` if the lifecycle fact is now sufficiently supported) —
  never retroactively inserted into an earlier seq. Event seq is the order
  Corral accepted the fact; occurrence time is when the runtime fact
  happened. Any historical occurrence timestamp carried by a
  later-appended lifecycle fact must be independently supported by
  authoritative runtime evidence; a first-observed time is never dressed
  up as a start time;
- a Run that ended while its binding was still Heuristic gains durable
  `Started`/`Ended` only if authoritative runtime evidence still supports
  those facts — confirming the association never automatically promotes
  prior heuristic runtime metadata to durable truth.

**Projection law.** Every persistent projection mutation must be justified
by an accepted durable semantic event. A change the accepted vocabulary
cannot express is out of scope until the owning phase explicitly extends
the event set (AGENTS.md §Durable state). Deferred with their producers:
binding supersession, assurance-change persistence, correction events,
archive/delete events. Live evidence re-evaluation is unrestricted; only
the write into a durable projection is gated.

> The event log owns durable semantic facts. Projections may summarize
> those facts; they may not silently acquire additional durable truth.

Never recorded: PTY bytes, raw hook events, provider transcripts, derived
status. Live runtime state stays runtime-owned.

## D7 — Unlinked is not the same as unrelated

When Corral cannot establish whether a newly seen provider session
continues a previous one, it records no lineage. The Session exists,
unrelated, and a person may correct it — unlink is first-class UI, manual
link is CLI-only and warns (`PRODUCT.md` §6). A guessed edge would be a
fabricated fact, and every control decision downstream would inherit it.
S2's fork finding shows this rule doing real work: an implementation will
be tempted to "recover" Claude fork parentage from message-uuid overlap,
and this rule is what forbids recording it.

## D8 — Archive and delete do not rewrite lineage

Archiving removes a Session from the active surface and ends nothing.
Deleting removes Corral-owned metadata only, never provider history. A
lineage edge naming a deleted Session is kept as a recorded fact with an
unresolvable target rather than silently erased: rewriting the log to hide
it would reinterpret a Corral-owned fact.

## Not decided here

Wire exposure: PR2 is zero-wire — lineage, Runs, and their vocabulary
reach the wire only when a surface renders them (no-ghost-wire rule).
History indexing (M2); structured approval (M2); Live synchronized control
mechanics; remote `RuntimeMove` (M3).

## Acceptance

The four open questions were ruled by the founder across a three-round
grill on 2026-08-22, together with the command-receipt, fail-closed, and
projection-completeness semantics that bound the PR2 plan. Full record:
`docs/decisions/2026-08-22-pr2-resume-lineage-acceptance.md`.
