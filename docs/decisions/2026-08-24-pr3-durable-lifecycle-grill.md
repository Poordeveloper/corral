# Founder Decision Record — durable run lifecycle grill

> Status: founder-ruled across three rounds, 2026-08-24. Materialized by
> ADR 0008 (Q1, Q12), the implementation plan
> `docs/plans/2026-08-24-pr3-durable-run-lifecycle.md` (Q2–Q11), and this
> record's own provenance. No implementation preceded it.

The grill exists because a post-merge check of PR3's Definition of Done found
Design 7 — *"corrald records the attach holder, reports it, enforces nothing.
Attach/detach append `RunAttached`/`RunDetached`"* — unimplemented, and the gap
wider than the line: `corrald` performs no durable write at all. `Store::vouch`
is its only store call. `record_run_started`, `SessionCreated`, `RunEnded` have
no callers; the crash reconciliation the plan describes has no code, because
nothing is written for it to reconcile.

The plan contradicted itself — Design 7 says events are appended, §Interfaces
says *"Persistence: none"* — so the founder ruled (b): close the gap, and grill
it first because closing it properly turned out to need decisions PR2 had left
to *"the phase that serves the first mutating RPC"*.

Four rounds of review on PR3 never caught this. A review compares a diff to a
goal, and a design item that was never written is not in the diff.

## Facts established before the questions

- `record_run_started` requires a **Runtime `BindingId`** and mints its own
  `RunId` internally. PR3 mints `RunId` itself and creates no binding.
- `BindingKey = (node, kind, provider, external_id)`; `ExternalId` is a bounded
  opaque string documented as *"a provider session id, a runtime handle, a
  history file identity"*.
- `Assurance::Deterministic` is documented as *"corrald spawned and owns the
  runtime; identity holds by construction"* — the model already has a place
  for managed sessions.
- At most **one control-capable Runtime binding per Session**
  (`refuse_second_control_capable_runtime_binding`), and the store has no
  unlink.
- `CommandKind` is documented as *"PR2 owns the mechanism, not the catalogue:
  the phase that serves the first mutating RPC names its own commands."*
- `CommandOutcome` has exactly one variant, `SessionCreated(CorralSessionId)`:
  there is no "accepted, outcome unknown" receipt state.
- `record_run_ended(&Run, …)` reads the Run back and, when it is not recorded,
  returns `Ok(Written::nothing_to_record(Durability::Withheld))` — **a silent
  discard, not an error**.
- `STORAGE_EPOCH = dev`: development databases are disposable, no migration
  obligation.
- No git tag exists, so no external tagged release: wire permanence has not
  begun and `session.new`'s request shape may still change.

The founder ruled explicitly that the last two answer compatibility and
migration cost only. They do not answer whether a decision is architecture
truth, and so are never a reason to skip an ADR.

## Round 1

**Q1 — managed-runtime binding identity → (a), with `external_id` pinned.**
Materialized in ADR 0008. The frozen shape and the four things the identity is
*not* live there; the reasoning that produced it is here: `ExternalId` is not a
process handle and not a Run handle. It names *"Corral's control-capable
managed-runtime binding for this Session"*. Concrete runtime occurrence is
expressed by `Run`, always. Rejected: pid or pid+start-time (identity rule and
grill Q10 both forbid it); `CorralSessionId` as the external id (works, but
visibly circular); no Runtime binding at all (Runs are the vocabulary ADR 0002
accepted).

**Q2 — `session.new` gains a required `command_id` → (a), hardened.** The
failure being closed is real: a lost response makes a client retry and two
agents run, the second unknown to anyone. The founder's hardening is that a
receipt in the database is not by itself a fix —

> Command receipt/deduplication MUST be consulted before a second runtime side
> effect is allowed.

— and the forbidden order is named: never *spawn first, then discover this
command id was already completed*. Fingerprint content is not frozen as
`argv + cwd + geometry`; it is *whatever the final semantic inputs to the
mutation are*, per the already-frozen Q12 rule, so a later input that affects
the mutation must join it.

**Q3 — who mints `RunId` → recommendation overturned; (c′).** The
recommendation was spawn-then-let-the-store-mint. The founder identified a real
race, not a theoretical one:

```text
spawn true
    ↓ process has already exited
    ↓ reaper already holds the exit
    ↓ store has not returned a RunId yet
```

`RunEnded` needs a `RunId` to name and does not have one. The ruling separates
two things the recommendation had fused: **minting an id is not asserting that
a runtime exists.** The caller pre-mints; `RunStarted` is persisted only after
`spawn()` confirms a concrete runtime occurrence; a failed spawn simply leaves
the id unused and no durable Run ever existed — which is exactly Q9's rule that
no Run fact may be written without a concrete runtime occurrence.

`corral-state` therefore changes to `record_run_started(run_id, …)`. The
founder ruled this an ownership/API correction rather than a durable schema
semantic change, and required that if the governance detector routes it to
human review for touching the state surface, that is followed — but never
described as *"schema forbids it"*.

**Q4 — historical sessions in `session.list` → (a).** PR3's `session.list`
keeps answering *"what managed sessions are actionable/live in this daemon's
runtime?"*. No store union, no recent-resumable list, no historical Exited or
Unknown rows. The store may be written without becoming a list read source.
The founder's reason: what a list means is a product projection decision, and
it belongs to PR4, where a person first looks at one — *"不应该由 PR3
persistence integration 偷偷决定"*.

**Q5 — startup reconciliation → (a), scope narrowed.** Not *"every unfinished
Run on this node"*. The predicate must come from accepted ownership facts —
a Corral-created/managed RuntimeBinding, and a Run belonging to a previous
daemon-owned managed episode — because a future Observed or provider-owned
external Run may legitimately outlive a `corrald` restart. `node_id +
ended_at IS NULL` is not the test.

Time semantics: the event sequence is when Corral accepted the ending fact;
the occurrence time stays unknown unless independently supported. A daemon's
startup timestamp is never dressed up as a process exit time.

And no fabricated detaches: if `RunEnded` terminates a Run's attachment state,
the projection follows from it. Corral does not write individual
`RunDetached` facts it never observed.

**Q6 — where `RunEnded` is written → (a)'s owner direction, contract
sharpened.** `runtime/` does not know the store: correct. But the seam is not
named after persistence. Runtime reports an occurrence —

```text
RunExited { run_id, observed_at, evidence… }
```

— and a daemon/state adapter turns it into the accepted durable `RunEnded`.
Hard requirement: the reaper and the screen-retirement path never block on
SQLite. Enqueue must be non-blocking or bounded-fast. Unrecoverable enqueue or
writer failure takes the Q14 fail-closed path; it must never be
`queue full → silently drop RunEnded → keep serving`. Frozen:

> runtime owns occurrence detection; state owns durable truth; the bridge must
> not make runtime teardown wait on database latency.

**Q7 — the advisory lease seam → (a), and renamed.** PR3 has no stable
client/holder identity, so *"who holds the lease"* cannot be implemented
honestly. What can is **attachment lifecycle**. Design 7 is restated as an
*advisory attachment seam*: `RunAttached` means an established Corral
attachment became active, `RunDetached` means one ended while the daemon could
observe it. No holder name, no client identity, no ownership attribution, no
enforcement — those wait for a phase with a consumer for them.

## Round 2

**Q8 — the two windows in `session.new` → (c), with the order frozen.** The
crash window and the concurrency window are different problems. ADR 0007 L6
closes the crash window for free: a daemon's managed runtimes die with it, so
retrying an unreceipted `session.new` against a new daemon is a legitimate
retry and not a duplicate live runtime. The concurrency window — two retries
arriving at one live daemon, both reading "no receipt" before either commits —
L6 does not touch, and a daemon-local in-flight table closes it.

The founder froze the order, because the obvious arrangement still races:

```text
1  compute the semantic fingerprint
2  atomically claim the CommandId's daemon-local in-flight slot
3  a slot already exists:
     same fingerprint      → join/wait for the first execution
     different fingerprint → CommandIdConflict
4  only the slot owner consults the durable receipt
5  receipt complete   → replay, never spawn
6  receipt absent     → the owner alone may enter the runtime side effect
7  the result is broadcast to waiters on the same command_id
8  the durable receipt is the replay authority across lost responses
```

Forbidden: *check the durable receipt → not found → then insert in-flight*.
Two concurrent requests both see "not found".

And the dependency is written down rather than assumed:

> The safety of retrying an unreceipted `session.new` after daemon loss depends
> on ADR 0007 L6: managed runtimes do not survive their owning daemon. Any
> future change that lets such runtimes survive daemon loss MUST reopen the
> command crash-window design before that behaviour ships.

**Q9 — the `RunStarted` ordering barrier → (a), fully accepted.** Combined with
Q3 the order is exact:

```text
RunId::mint()
→ spawn()
→ spawn confirms a concrete runtime occurrence exists
→ synchronously persist RunStarted(run_id, …)
→ only after a successful commit: start reader, screen, reaper;
  publish/register the runtime handle
```

so that **the producer of `RunEnded` does not exist until durable `RunStarted`
has succeeded**. Structure, not queue discipline. The founder's reason for
insisting: a writer queue that preserved order 99.9% of the time would, in the
remaining case, produce a durable Run that looks legitimate and stays open
forever — the most dangerous failure mode available, and the one the silent
`Durability::Withheld` path hands out for free.

`corral new -- true` is confirmed correct history: `RunStarted` then
`RunEnded`, because the runtime occurrence did exist.

On `RunStarted` persistence failure: the runtime must not enter the registry or
become externally usable; best-effort terminate and reap; the state failure
follows Q14. It must **not** call the `RunEnded` path as a remedy — with no
durable `RunStarted` there is no Run in the durable model to end.

**Q10 — lifecycle sink boundedness → (b)'s semantics, capacity not frozen.**
Frozen: the queue is bounded; teardown never blocks on DB latency; lifecycle
facts are never silently dropped; writer death or capacity exhaustion means
state integrity can no longer be guaranteed and triggers fail-closed shutdown.
Queue full is an integrity failure, not ordinary backpressure. **Not** frozen:
4096, 8192, 1024 — capacity is measured implementation policy, and the plan
states an initial value with tests rather than canon.

The founder also scoped the threat model: this is same-OS-user local, which is
not a hostile security boundary. Whether attach churn needs admission control
belongs to the phase that admits remote or untrusted clients — never to
dropping durable events now.

**Q11 — attachment projection → (a).** Append `RunAttached`, `RunDetached`,
`RunEnded`; build no materialized attachment projection, because Q4 left no
reader. The reducer invariant is recorded for whoever builds one:

> `RunEnded` is terminal for that Run's active-attachment state. A projection
> MUST treat all still-open attachments as inactive after `RunEnded`, without
> fabricating synthetic `RunDetached` events.

So `Attached A, Attached B, Ended` projects to zero active attachments while
the durable log still contains only those three facts. A projection may
summarize truth; it may not invent it.

**Q12 — reserved `ProviderId("corral")` → (a), strengthened.** Not a global
refusal of the string — the Corral-owned RuntimeBinding needs it. The invariant
is directional and lives in ADR 0008.

**Q13 — how these are recorded → (b), widened.** One small ADR covering **Q1
and Q12 together**, because they are one decision — *how Corral names and
assures the managed runtime identity it owns* — and Q12 is the enforcement that
keeps Q1's durable meaning from being broken by PR5/PR6, not ordinary
validation. Everything else is execution contract for already-accepted
semantics and belongs in the implementation plan.

## What the founder rejected outright

- A durable `CommandOutcome::Accepted` written before spawning (Q8 (b)): it
  adds an outcome variant for a guarantee L6 already provides, and a claim
  that is never upgraded becomes a receipt naming no session.
- A published `retired`-style third flag anywhere in this design.
- Reconciliation fabricating `RunDetached` facts to zero a counter.
- Any arrangement in which `spawn` happens before the command's completion
  status is known.
