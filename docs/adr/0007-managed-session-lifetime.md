---
status: accepted
read_when:
  - changing what starts, ends, or removes a managed session
  - adding a thread, descriptor, or buffer to a session's runtime
  - deciding what `execution_state` may claim after a screen is gone
  - deciding what a client is told when a session's terminal is no longer served
  - changing where the emulator is entered from
---

# The lifetime of a managed session: who owns its end

`ARCHITECTURE.md` §3 fixes who owns a managed terminal: `corrald` holds the
authoritative screen, one bounded emulator per session. ADR 0002 fixes the
vocabulary a Run may be described in. ADR 0003 fixes what a snapshot carries.
None of them says when a managed session *ends*, and PR3 built the runtime
without answering it — so nothing released a screen, nothing removed a thread,
and every failure path left an entry that answered questions forever.

Written during the design pass the founder ordered when four review rounds on
PR3 stopped converging. Acceptance evidence:
`docs/decisions/2026-08-24-adr7-managed-session-lifetime-acceptance.md`.

**The invariant.** Every resource a managed session creates has one owner and
one release point, and the release point is reached on every path — the ones
Corral chose and the ones it suffered. A session that Corral can no longer
serve is either an end Corral can state or a loss Corral must report; it is
never a thread still waiting for a message that cannot arrive.

## L1 — A managed session has three lifetimes, not one

They are separated because they have different owners, different costs, and
different ends. Collapsing them is what produced the defect.

**The runtime** — the child process, the process group Corral created it as,
and the PTY master. Ends when the child is reaped, or when the terminal closes
and the exit cannot be established.

**The screen** — the authoritative emulator, its scrollback, the delta stream,
its viewers, and the thread that owns all of them. Begins with the runtime and
ends with it (L2).

**The record** — Corral's session id, the run id, the launch title, the
terminal execution state, and the final screen. Outlives both. In M1 it is
process memory and ends with the daemon: PR3 persists nothing, and a durable
record is a later phase's decision, not this one's.

The registry therefore needs no removal operation. What was missing was never
`remove`; it was release of the screen.

## L2 — The screen is an actor only while its runtime produces bytes

The emulator is not `Send` — it holds raw pointers — so while bytes are
arriving it must live on exactly one thread, and everything about it is asked
by message. That is a cost paid for a reason: output must be consumed, device
queries must be answered while nobody is attached (ADR 0003 §3), and reflow
must not race a write.

Once the runtime has ended, none of those reasons survives. No further byte can
arrive, no device query can be asked, no reflow can matter. **So the end of the
runtime is the screen's last act:** the thread encodes the final screen,
publishes it with the execution fact, drops the emulator, the delta stream, the
PTY master and the writer, and returns.

A finished session's screen is a value, not an actor.

The value is one snapshot, bounded by ADR 0003 D7's budget and ceiling —
strictly less than the emulator it replaces, which held a 4 MiB scrollback that
no snapshot extent could ever have reached. It answers every question a
finished session can be asked: attach, snapshot, geometry, title. It cannot
answer resize or input, and those are refused rather than silently dropped.

This is why there is no linger, no retention timer, and no policy number here.
A finished screen stays readable for the daemon's whole life, and it costs a
snapshot.

## L3 — A terminal execution fact is final; a live claim is not

`Running` is a claim about the present, and the thread that published it is the
only thing that can extend it. If that thread is gone with `Running` still
published, the claim is about a past nobody can extend: the answer becomes
`Unknown`, because losing the ability to manage a runtime is not evidence about
a process (ADR 0002).

`Exited` and `Unknown` are terminal. Nothing can un-exit, and nothing can make
an unestablished end establishable later. Downgrading them when their publisher
retires would report "Corral cannot establish this" about an exit Corral
watched happen — the vocabulary ADR 0002 exists to prevent.

The same two facts distinguish a retirement from a loss without a third flag:
a screen thread that is gone having published a terminal fact retired; one that
is gone with `Running` published was lost. What a client is told follows from
it — "this run has ended" is a different sentence from "this session's runtime
is no longer answering", and a client that cannot tell them apart cannot tell a
finished agent from a broken daemon.

## L4 — The child's group may be signalled only before the reaper has waited

Corral tears down descendants itself, by process group, because the backend's
own kill targets one child (grill Q1). The group number is the child's pid, and
after the child is reaped that number may name something else.

There is exactly one reaper per session and it is the only party that waits, so
the rule is total: **any teardown that signals the group happens before the
reaper has waited, and none happens after.** No teardown path needs to consult
the tty, and none may.

## L5 — Containment surrounds the screen, not one call into it

The emulator is third-party code with a large unsafe surface on the path every
untrusted byte takes first (ADR 0003 D1), and a panic out of a half-modified
packed page makes even reading it unsound. Feeding it is contained for that
reason.

Reflow and snapshot serialization enter the same structure with the same
consequence, so they are contained for the same reason. A boundary drawn around
one of the three entrances is not a boundary: it decides that two identical
risks have different answers, and the difference is an accident of which one
the fuzzer reached first.

Containment is fail-closed, never repair: the screen is poisoned and nothing
reads it again — **including its destructor**. `PageList::drop` walks the same
packed page list with `Box::from_raw` on every node, so refusing to read a
half-modified structure while still letting it be dropped is the same unsound
traversal, run later and unconditionally. A poisoned emulator is therefore
forgotten rather than dropped: one session's retained scrollback is a bounded,
one-off cost, and undefined behaviour in a daemon still serving every other
session is not.

(The destructor sentence was added after the round-5 review found it; it states
the rule this section already made, applied to the one entrance that is never
written as a call.)

## L6 — The daemon's own exit ends every managed run, unverified

`corrald` stops accepting, closes established connections, and returns. The
managed children are hung up by the kernel when the last descriptor of each PTY
master closes with the process. Corral does not wait for them, does not reap
them, and therefore may not claim they exited — a daemon exit is not evidence
about a process any more than any other loss of the control plane.

This is a stated M1 limitation, not a design goal: `AGENTS.md` requires that
restarting the control plane not unnecessarily terminate managed sessions, and
handing a running child to a successor daemon needs a mechanism M1 does not
have. What this ADR fixes is that the limitation is written down and logged,
rather than being something a user discovers.

## Consequences

- Nothing accumulates per finished session except its record and one snapshot.
  Before this, each finished run permanently retained a thread and an emulator,
  bounded only by the daemon's exit — which any connected client postpones
  indefinitely.
- A screen lost while its runtime is live is no longer left as a child blocked
  forever on a PTY nobody drains: the reaper hangs up the group it still owns
  and establishes the end. That is the same ruling already made for a session
  the registry refused, applied to the same fact arriving by another route.
- `SessionGone` narrows to its true meaning. A finished session answers from
  its record; only a genuine loss reports that the runtime is not answering.

## What this does not decide

**Whether a poisoned screen is visible before you try to use it.** Poisoning is
a property of the screen, and a session's record has no field for it, so
`session.list` shows a poisoned session as an ordinary running one. Open
question, with the root cause, in
`docs/evidence/pr3-terminal-fuzz-2026-08-24.md`.

**Whether a finished session's record is durable, acknowledgeable, or
removable.** M1 has no acknowledgement concept and persists no runtime facts.
When one exists it decides how a record leaves the list; until then a record
lives as long as the daemon does.

**Whether a managed run can survive the daemon.** L6 states the current
behaviour. Changing it is a control-plane handoff mechanism and an
`AGENTS.md` §Architectural changes decision of its own.
