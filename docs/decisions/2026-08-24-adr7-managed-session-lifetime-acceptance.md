# Founder Decision Record — ADR 0007 acceptance

> Status: founder-accepted, 2026-08-24, as proposed and without
> modification. Materialized by ADR 0007 flipping to `accepted`, the
> `Final screen` glossary row and the terminal-section rule in
> `ARCHITECTURE.md`, and the closing sentence on benchmark row 3 — the
> implementation landed ahead of acceptance on `task/pr3-terminal-runtime`
> and is unchanged by this record.

Not a grill. ADR 0007 came out of a design pass the founder ordered after
four `/code-review` rounds on PR3 returned 15/15/14/15 findings without
converging: *先做一次会话生命周期的设计过程*. The pass found that about
eight of those findings were symptoms of one thing nobody had designed —
what ends a managed session — and that the loop's own comment described a
retirement the code could not reach, because the registry held an `Ask`
sender for the daemon's whole life.

The decision is on the `AGENTS.md` §Architectural changes list twice —
runtime/PTY ownership and execution-state authority — which is why an
agent could not accept it and why the ADR was written rather than the
repair being made quietly inside a bug-fix commit.

No durable-state acceptance is involved. PR3 persists nothing, and a
finished run's record is process memory that ends with the daemon.

## What was accepted

L1–L6 as written in `docs/adr/0007-managed-session-lifetime.md`. The
load-bearing pair:

**L2 — the screen is an actor only while its runtime produces bytes.** The
end of the runtime is the screen thread's last act. It publishes the final
screen and returns, releasing the emulator, the delta stream, the pty
master and the writer. A finished run's screen is a value, not an actor.

**L3 — a terminal execution fact is final; a live claim is not.** `Exited`
and `Unknown` survive their publisher retiring; `Running` does not, and
becomes `Unknown` when the thread that could extend it is gone. The same
two facts distinguish a retirement from a loss with no third flag.

## Alternatives the design pass rejected, recorded so they do not return

**A retention timer (a "linger").** The first design kept a finished
session's screen thread alive for a bounded window so a person could still
read what an agent left, retiring it once no viewer remained. It was
dropped once L2 was stated properly: if no byte can arrive, the screen is
a value, and a value needs no thread to hold it. The timer would have
bought nothing and cost a policy number to rule on, a clock in the screen
thread, and a window in which a finished screen becomes unreadable.

**Adding `remove` to `ManagedSessions`.** The obvious reading of the
defect — nothing removes a session — is the wrong one. The record is
cheap (two ids, a title, a state, one snapshot) and is what `session.list`
is for; the expensive thing is the screen. What was missing was never
removal, it was release. The registry still has no removal operation, and
a future phase that owns acknowledgement decides how a record leaves the
list.

**A published `retired` flag.** Rejected as a third representation of
something two existing facts already say. Adding it would have created a
state that can disagree with the execution value beside it.

**Consumer-side inference in the terminal channel.** The channel could
have read `execution_state` after a `SessionGone` to work out whether the
run had ended. Rejected under `AGENTS.md` §Scope discipline: the handle
owns the distinction, so it returns it — `InputRefused::RunEnded` versus
`RuntimeGone` — rather than leaving each consumer to re-derive it.

## Scope this record does not cover

Three questions stay open and are not resolved by this acceptance:
whether a poisoned screen is visible before you try to use it (open in
`docs/evidence/pr3-terminal-fuzz-2026-08-24.md`), whether a finished
session's record becomes durable or acknowledgeable, and whether a managed
run can survive the daemon — the last being a control-plane handoff
mechanism and an architectural decision of its own.
