# Founder Decision Record — attachment activity is advisory; and what `session.new`'s answer asserts

> Status: founder-accepted, 2026-08-25. Two rulings from the same review of
> `task/pr3-durable-run-lifecycle`. Neither changes durable state or the
> accepted event vocabulary. Materialized in `corrald`'s occurrence sink,
> `SessionNewResult`'s documentation, and two invariants added to
> `ARCHITECTURE.md` §3.

## 1. Attachment activity is advisory

### What was asked

The durable-lifecycle grill's Q10 froze the occurrence sink's rule as *lifecycle
facts are never silently dropped; capacity exhaustion means state integrity can
no longer be guaranteed and triggers fail-closed shutdown*, and scoped the
threat model out: *same-OS-user local is not a hostile boundary*.

Review found what that leaves open. Every terminal channel reports one
`RunAttached` and one `RunDetached`, and both share the queue with run endings.
A client that connects, redeems a token and disconnects in a loop fills the
queue faster than one SQLite transaction per fact can drain it — and the sink's
answer to a full queue was to end the daemon, taking every managed agent's
control plane with it. `AGENTS.md` §Security says the same OS user is not
sufficient proof of an authorized actor, so "not a hostile boundary" was not an
answer.

### The ruling

Two things were being conflated:

```text
subscriber / attach activity
        ↓
daemon lifetime decision
```

Attaching is an **observer's** behaviour. It is not runtime ownership.

> Attachment activity is advisory.
> Managed runtime ownership is authoritative.

Concretely, daemon lifetime is decided by established terminal subscribers,
managed runs, and other accepted ownership roots. Attach and detach *events*
may inform diagnostics, short-term buffer cleanup, and what a surface shows.
They may not change lifecycle truth — not by manufacturing liveness, not by
extending a daemon indefinitely, and not by racing it into a wrong idle
verdict.

The data channel already has the right shape one layer down: a viewer that
overflows its budget is desynchronised and dropped, and resyncs. The same shape
applies here:

> A subscriber that fails the data-channel contract loses its subscription,
> not the daemon lifecycle.

A slow client is dropped; a broken client is dropped; a churning client is
advisory noise. The daemon neither exits because of it nor lives forever
because of it.

**No durable change.** No new event, no new state, no new persisted field.

### What it took in code

`RunOccurrence` gained a `Weight`: an ending is `Authoritative`, an attachment
is `Advisory`. Three things follow, and the third is the one that makes the
first two more than words:

- an advisory fact that cannot be enqueued or cannot be written is logged and
  costs nothing else;
- a shutdown waits only for authoritative facts to land;
- advisory facts are bounded by a share of the queue well below the whole, so
  churn exhausts its own budget and never the room an ending needs. Without
  that reservation, "advisory" would still have let an observer starve the
  fact the daemon must account for.

## 2. What `session.new`'s answer asserts

### What was asked

When the runtime registry cannot be consulted after a command has durably
committed, the caller is answered with the session and run the command created,
for a Run that is already ending. Review named the gap as a missing
`CommandOutcome` variant — *executed, then ended*.

### The ruling

Do not add one. Audit the existing wording first, because a new variant may be
re-fusing two layers that are already correctly separate:

```text
Command acceptance      accepted / rejected
Run lifecycle           Running / Exited / Unverifiable
```

The audit condition was stated as: if the accepted vocabulary already means
*accepted*, no durable change; if it claims *execution*, a Class C decision is
required.

### The audit

It means accepted.

- `CommandReceipt` is documented as *"the durable record that a command was
  **accepted** and what it produced"*.
- The event is named `CommandAccepted`.
- `CommandOutcome::SessionCreated(CorralSessionId)` names what was created, not
  what is running.
- `SessionNewResult` carries two identities and no state field. Execution state
  is reported only through `session.list`'s `execution_state`, whose values are
  `running` / `exited` / `unknown`.
- ADR 0002 D6 already holds the two layers apart: event sequence is when Corral
  accepted a fact, and a Run's own facts carry when it happened.

**No Class C. No durable change.** What was wrong was wording inside the
daemon, not the vocabulary: the internal outcome was named `Started`, which
invites exactly the collapse this ruling forbids. It is `Accepted` now, and
`SessionNewResult` states what it does and does not assert.

## Why this is not a smaller version of the rejected variant

`CommandOutcome::Accepted` was rejected in the durable-lifecycle grill (Q8) as
a durable receipt written *before* spawning — a claim that is never upgraded
and ends up naming no session. Nothing here writes such a receipt. The receipt
is still written only with the Run it produced; this settles what reading one
back means.
