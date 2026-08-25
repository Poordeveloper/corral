---
status: done
class: B
writes: [corrald, corral-state, corral-core, corral-protocol, corral-client, corral]
reads: [docs/adr/0002-resume-lineage.md, docs/adr/0007-managed-session-lifetime.md, docs/adr/0008-managed-runtime-binding-identity.md, docs/decisions/2026-08-24-pr3-durable-lifecycle-grill.md, docs/plans/done/2026-08-24-pr3-terminal-runtime.md]
---

# PR3.1 — the durable run lifecycle corrald never wrote

ADR 0008 is accepted and this is implemented. Where the implementation
departed from the plan, the departure is recorded in §What changed on contact
with the code.

## Goal

`corrald` records the managed runs it owns. `session.new` becomes an
idempotent command; a Run's start, end, attachments and detachments become
durable facts under a managed RuntimeBinding; a daemon that restarts closes
the episodes it can no longer manage. This closes PR3's Design 7 and the
wider gap behind it — `Store::vouch` is currently corrald's only store call.

Every decision below was ruled in
`docs/decisions/2026-08-24-pr3-durable-lifecycle-grill.md` (Q2–Q11);
implementation materializes those rulings.

## Non-goals

No `session.list` change: it keeps answering *what is live in this daemon's
runtime* (Q4). No store-backed history union, recent-resumable list, or
historical rows — that is PR4's product projection decision. No materialized
attachment projection (Q11): the events are appended and nothing reads them
yet. No holder identity, ownership attribution, or enforcement on the
attachment seam (Q7). No provider bindings (PR5+). No admission or rate
control for attach churn (Q10): same-OS-user local is not a hostile boundary,
and the phase that admits untrusted clients owns that.

## Existing owner / architecture involved

`corral-state::Store` owns durable truth and already has the whole event
vocabulary. `crates/corrald/src/state.rs` owns the daemon's one handle on it.
`crates/corrald/src/runtime/` owns occurrence detection and must not learn
about the store (Q6). `crates/corrald/src/connection.rs` serves `session.new`.
ADR 0007 owns the managed session's lifetime and supplies the single point at
which a run ends.

## Design

**1. Managed RuntimeBinding (ADR 0008).** `corral-core` names the reserved
provider id once. `corral-state` enforces the direction: a `CorralCreated`
Runtime binding must carry it, a `ProviderSession` binding must not. Session
creation resolves the Session's existing control-capable Runtime binding or
mints one, once.

**2. `session.new` becomes a command (Q2).** A required `command_id` on the
request; the daemon builds `Command { id, kind: "session.new", fingerprint }`.
The fingerprint covers the command kind and every semantic input that affects
the mutation — not the wire bytes, and not a frozen list: whatever the final
input set is, all of it is in. Legal to require now because no external tagged
release exists; `command_id` becomes a baseline required field.

**3. The in-flight coordinator (Q8).** Daemon-local, keyed by `CommandId`, and
the order is load-bearing:

```text
1  compute the semantic fingerprint
2  atomically claim the CommandId's in-flight slot
3  slot exists:  same fingerprint      → join/wait for the first execution
                 different fingerprint → CommandIdConflict
4  only the slot owner consults the durable receipt
5  receipt complete → replay, never spawn
6  receipt absent   → the owner alone may enter the runtime side effect
7  result broadcast to waiters on the same command_id
8  the durable receipt is the replay authority across lost responses
```

Never *consult the receipt, then insert in-flight*: two concurrent requests
both read "absent". A slot whose owner failed without writing a receipt is
released — nothing was completed, so a later retry may execute.

The crash window is closed by ADR 0007 L6 rather than by machinery, and the
dependency is recorded in code where the coordinator lives:

> The safety of retrying an unreceipted `session.new` after daemon loss depends
> on ADR 0007 L6: managed runtimes do not survive their owning daemon. Any
> future change that lets such runtimes survive daemon loss MUST reopen the
> command crash-window design before that behaviour ships.

**4. RunId minting and the start barrier (Q3, Q9).** Exact order:

```text
RunId::mint()
→ spawn()
→ spawn confirms a concrete runtime occurrence exists
→ synchronously persist RunStarted(run_id, …)
→ only after a successful commit: start reader, screen, reaper;
  publish/register the runtime handle
```

Minting an id is not asserting that a runtime exists, so a failed spawn leaves
the id unused and no durable Run ever existed. `corral-state` changes to
`record_run_started(run_id, …)` — an ownership/API correction, not a durable
schema semantic change; if the governance detector routes it to human review
for touching the state surface, that is followed and never described as a
schema prohibition.

Ordering is structural: the producer of `RunEnded` does not exist until durable
`RunStarted` has committed. This is not queue discipline, because the failure
mode of getting it wrong is silent — `record_run_ended` on an unrecorded Run
returns `Durability::Withheld` without an error, leaving a legitimate-looking
Run open forever.

On `RunStarted` persistence failure: do not register or publish the runtime;
best-effort terminate and reap; follow the Q14 fail-closed path. Never call the
`RunEnded` path as a remedy — with no durable start there is no Run to end.

**5. The lifecycle sink (Q6).** `runtime/` reports occurrences and knows
nothing about persistence:

```text
RunExited { run_id, observed_at, evidence… }
```

A daemon/state adapter turns that into the accepted durable `RunEnded`. Frozen:
runtime owns occurrence detection, state owns durable truth, and the bridge
never makes runtime teardown wait on database latency. Enqueue is non-blocking
or bounded-fast; the reaper and the screen-retirement path never touch SQLite.

**6. Sink boundedness (Q10).** Bounded queue; teardown never blocks; lifecycle
facts are never silently dropped; writer death or capacity exhaustion means
state integrity can no longer be guaranteed and triggers fail-closed shutdown.
Queue exhaustion is an integrity failure, not backpressure. The capacity is an
initial implementation value, stated in the plan and exercised by tests — not
canon. Dedicated writer thread versus blocking pool is likewise an
implementation detail, constrained only by ordering, no silent loss, fail
closed, and no blocking of the reaper or screen.

**7. The advisory attachment seam (Q7).** `RunAttached` when an established
Corral attachment becomes active; `RunDetached` when one ends while the daemon
can observe it. No holder identity in the payload — PR3 has none to be honest
about. It supports attachment counts, detach/reattach history, and the fact
that a surface disconnecting is not a Run ending.

**8. Startup reconciliation (Q5).** For every unfinished Run **that Corral owns
as a managed-runtime episode** — a Corral-created managed RuntimeBinding, and a
Run belonging to a previous daemon-owned episode — append
`RunEnded(Unverifiable)`. Not `node_id + ended_at IS NULL`: a future Observed or
provider-owned external Run may legitimately outlive a restart.

Time semantics: the event sequence is when Corral accepted the ending fact; the
occurrence time stays unknown unless independently supported. A daemon's
startup timestamp is never presented as a process exit time. No synthetic
`RunDetached` facts are fabricated to zero an attachment count.

**9. The reducer invariant, recorded not built (Q11).** No projection is added.
For whoever builds one:

> `RunEnded` is terminal for that Run's active-attachment state. A projection
> MUST treat all still-open attachments as inactive after `RunEnded`, without
> fabricating synthetic `RunDetached` events.

## Interfaces or persistence changed

Wire: `session.new` gains a required `command_id`, and a conflict answer for
the same id with a different fingerprint. Human-gated as a compatibility-facing
commitment even though no external release exists.

Persistence: corrald begins writing. No schema change; `record_run_started`
changes signature (Q3). `corral-core` gains the reserved provider id and
`corral-state` the binding-direction refusals (ADR 0008 D3). Any `corral-state`
diff routes to human review through the schema gate regardless.

`STORAGE_EPOCH` is `dev`, so development databases violating ADR 0008 D3 may be
reset destructively rather than migrated.

## Failure / unknown states

Spawn fails → id unused, no Run, no receipt, slot released. `RunStarted` commit
fails → runtime never published, terminate and reap, fail closed. Process exits
between spawn and the commit → correct history, `RunStarted` then `RunEnded`,
because the occurrence did exist. Daemon dies before the receipt → L6 kills the
runtime with it, so a retry is a legitimate retry. Two concurrent retries → the
in-flight slot, not the receipt, decides. Sink exhausted or writer dead →
fail-closed shutdown, never a dropped fact. Daemon restart → managed episodes
close as `RunEnded(Unverifiable)`, with the canon preserved: *it closes Corral's
managed episode; it never claims the OS or provider process exited.*

## Tests

- `corral new -- true`: the instant-exit case. Must produce `RunStarted` then
  `RunEnded`, and **must not** leave a durable `RunStarted` with no eventual
  `RunEnded` merely because the process exited before lifecycle publication
  finished. This is the regression for the silent `Durability::Withheld` path
  and is load-bearing.
- A retried `session.new` with the same `command_id` and fingerprint spawns
  exactly one runtime and replays one result — asserted both for a lost
  response and for two concurrent requests on one live daemon.
- Same `command_id`, different fingerprint → `CommandIdConflict`, no spawn.
- `RunStarted` commit failure → no registry entry, no attachable session, child
  reaped, fail-closed path entered.
- Binding direction: a `CorralCreated` Runtime binding without the reserved
  provider id is refused; a `ProviderSession` binding with it is refused.
- A Session's second Run reuses the existing managed RuntimeBinding rather than
  minting a second (which the store would refuse).
- Sink: sustained normal lifecycle load does not exhaust capacity; deliberate
  attach churn can; exhaustion deterministically enters the fail-closed path;
  no event is silently dropped.
- Reconciliation closes a previous daemon's managed episodes and leaves a
  non-managed unfinished Run alone.
- Reconciliation writes no `RunDetached`, and the occurrence time of a
  reconciled `RunEnded` is not the daemon's start time.
- Attach/detach append their events; a detach that never happens because the
  daemon died leaves an unbalanced pair, and reconciliation does not invent one.

## Definition of done

- ADR 0008 accepted before implementation crosses its decision boundary.
- Design 1–9 landed; `./scripts/verify` green on the final tree.
- The "attach lease" glossary row PR3's DoD required, restated as the
  attachment seam Q7 actually delivers.
- PR3's plan carries a correction note: Design 7 did not land, and this is
  where it did.
- Human-merged: wire change, `corral-state` surface, first durable writes.

## What changed on contact with the code

**The command's four facts became one transaction.** The plan implied
`create_session`, then a binding, then `record_run_started`. Written that way,
a crash between the receipt and the Run leaves a receipt naming a Session whose
episode nothing can describe — and a retry could then only answer with an
outcome the accepted vocabulary has no variant for, which is the
`CommandOutcome::Accepted` the founder rejected. So `Store::create_session`
became `Store::start_managed_session`, writing `SessionCreated`,
`BindingAdded`, `RunStarted` and `CommandAccepted` together. The plan's own
failure list already required this — *"Spawn fails → id unused, no Run, no
receipt"* is only true if the receipt is written after the spawn. Extended
rather than added beside: two ways to create a Session under a command id would
have been the parallel abstraction AGENTS forbids.

**`record_run_ended`, `record_run_attached` and `record_run_detached` take a
`RunId`.** The plan named only `record_run_started`. The other three took a
`&Run` the runtime owner does not have and would have had to fabricate to be
allowed to speak; the store reads the Run back from its own log either way.
Same ownership correction, same semantic scope.

**No resolve-or-mint for the managed binding.** D2's *"resolution is
lookup-first"* has no caller in this phase: `session.new` always creates a
Session, and a new Session has no binding to find. Writing the lookup now would
have been a branch nothing can reach. The rule that makes reuse mandatory later
is already enforced — a second control-capable runtime binding on one Session
is refused — and that refusal is what the plan's "reuses rather than mints a
second" test asserts.

**A departing daemon waits for what it observed.** Not in the plan, and the
plan's own rule required it: facts still queued when the process ends are facts
nobody would ever write, which is the silent loss Q10 forbids. The wait is
bounded and a wait that runs out is itself reported, in the exit status.

**Advisory facts do not share the sink's fate with authoritative ones.** Q10
froze "never silently dropped" for the sink as a whole, and review found that
attach churn could therefore exhaust the queue and end the daemon. The founder
split the two rather than loosening the rule:
`docs/decisions/2026-08-25-advisory-attachment-activity.md`. The same record
carries the audit that settled what `session.new`'s answer asserts — accepted,
not executing — which needed no durable change.

**Terminal geometry is not a semantic input.** Q2 declined to freeze the
fingerprint's contents, so this phase had to settle `session.new`'s concrete
set. Geometry went in on the literal reading and came back out on the founder's
ruling, because it is the one input a client cannot repeat across a retry:
`docs/decisions/2026-08-25-session-new-fingerprint-excludes-geometry.md`.

**F1 from the PR3 review, folded in as the follow-up it was slated for.**
`execution_state`'s catch-all had flipped from `Unknown` to `Running` during
ADR 0007's change. `Running` is the one answer a fallthrough must never give:
it is the only value that asserts a process exists.

## Plan size justification

One coherent semantic scope: every item exists so that a managed run's
existence, end, and attachments are durable facts under an identity Corral owns.
The pieces are not separable — a Run cannot be recorded without a binding to
hang it on, the binding is meaningless without the identity rules, and an
idempotent create is what stops the durable record from acquiring runs nobody
asked for.
