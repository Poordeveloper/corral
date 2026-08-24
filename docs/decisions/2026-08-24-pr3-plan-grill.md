# Founder Decision Record — the PR3 plan grill

> Status: founder-accepted, 2026-08-24. Materialized by the revision of
> `docs/plans/2026-08-24-pr3-terminal-runtime.md` landing with this
> record. Ruled across a two-round grill of the PR3 plan's own calls and
> its silently assumed policies; ADR 0003 and ARCHITECTURE §3 were
> already accepted and were not reopened.

Four rulings modified the drafting agent's recommendation (Q3, Q5, Q7,
Q8) and one was replaced outright (Q5). Several accepted rulings had
their *reasoning* rejected while the choice stood — those are recorded,
because a decision defended by a bad argument gets re-litigated on the
argument.

No durable schema or event acceptance is involved: Q9 is an explicit
capability check confirming PR2's accepted vocabulary already expresses
every fact PR3 produces.

## Round 1

**Q1 — PTY backend → `portable-pty`, with the ownership argument
rejected.** The choice stands: portable-pty's Unix spawn path performs
setsid + TIOCSCTTY itself and maps master-read EIO to EOF, so Corral
needs no unsafe PTY crate and keeps `forbid(unsafe_code)`. Rejected
reasoning: *"controlling-terminal / EIO / process lifecycle all belong to
upstream."* Not all of it.

    portable-pty owns the PTY platform mechanism.
    Corral still owns managed-runtime lifecycle semantics.

Frozen boundary — portable-pty: PTY allocation, controlling-terminal
setup, spawn plumbing, PTY I/O, resize, platform EIO/EOF mechanics.
Corral runtime: process-group identity capture, child/reaper
bookkeeping, descendant teardown policy, detach semantics,
daemon-shutdown policy, Run lifecycle truth. `Child::kill` is a
child-process operation (SIGHUP, then kill) and is not equivalent to
Corral's descendant/process-group teardown contract; portable-pty
exposes `process_group_leader` and rustix offers a safe
`kill_process_group`, so Corral owns that policy without unsafe.

**Implementation gate, not a later bug.** "Production-used" does not
admit the dependency. The WezTerm Unix spawn path has an open
exec-failure report — pre_exec fd cleanup can interfere with Rust's
exec-error pipe, so the parent may not receive a normal spawn error.
Before the dependency is relied on, a focused compatibility test must
cover nonexistent executable, non-executable target, invalid cwd, normal
executable, child exit, resize, process-group identity, on Linux and
macOS. Load-bearing assertion: *Corral must be able to distinguish
"command did not successfully exec" from "command started and later
exited."* If the current release cannot: pin a fixed upstream revision,
add a bounded workaround, or reopen the backend decision. Silently
accepting wrong spawn semantics is not an option.

**Q2 — terminal data channel → accepted, with boundaries frozen.**
`terminal.attach` → one-time token → second connection to the same
canonical endpoint → bootstrap hello declares terminal-data role + token
→ binding → the connection transitions permanently into terminal binary
framing.

1. The second connection is not a second daemon namespace.
2. It starts in the existing bootstrap framing; after the role/token
   hello succeeds the transition is one-way. That fd never carries
   ordinary RPC methods again, never interleaves JSON frames, never
   returns to control mode. No generic multiplexing framework.
3. The token is a local bearer authorization/correlation capability
   resting on the canonical UDS same-user filesystem boundary. Rejected
   reasoning: *"it leaves an isomorphic seam for future remote
   authentication."* Remote is out of PR3 scope; whether remote reuses
   this is remote architecture's decision, not a justification available
   today.
4. It binds to `CorralSessionId` **and** the concrete Run / terminal
   runtime identity — never Session alone, because a Session outlives a
   Run and a stale token must never attach to a different terminal after
   resume. It does **not** bind snapshot epoch: epochs change normally
   under resize/resync and are not runtime identity.
5. High-entropy, short-lived, single-use, atomic redemption.
6. Redemption consumes. If the initial snapshot then fails to
   produce/send, the attach attempt fails and the token stays consumed —
   the client requests a fresh `terminal.attach`. No half-consumed
   rollback branch.
7. Early death of the control RPC connection does not invalidate an
   issued token; the token's own TTL and single use are the authority.
   Otherwise two independent connections are covertly re-coupled into a
   lifecycle dependency.

This is a compatibility-facing wire commitment (`terminal.attach`, the
ClientHello role/token semantics, the framing transition), so it is
`HUMAN_REVIEW_REQUIRED` and this record is its acceptance evidence. The
plan may no longer claim PR3 touches no protocol human gate.

**Q3 — `session.list` shape → MODIFIED; `unverifiable` rejected as an
execution state.** Assurance/knowledge condition is a different
dimension from execution state, attention state, and freshness; those
were frozen apart and are not re-merged to save a field.

    SessionListItem { session_id, title, execution_state }

`session_id`: `CorralSessionId`, the identity field. `title`: optional,
non-authoritative display label; a client must never parse it or use it
for identity or control. `execution_state`: `Running` | `Exited` |
`Unknown`, where Unknown means *Corral cannot currently make a reliable
execution-state claim* — the execution dimension's own value, not PR8
attention Unknown smuggled in, and not spelled "Unverifiable execution
state". Reconciliation with a durable record of a live Run and no
reliable current claim yields Unknown, never Exited. A future assurance
surface is an additive separate field or capability, never assurance
vocabulary pushed into the execution enum.

**Title, hard rule: PR3 title must not be full argv concatenation.**
argv routinely carries `--token`, passwords, URLs, paths, and customer
identifiers; a list needing one line of text is no reason to spread the
whole command line into `session.list`, UIs, logs, and screenshots.
Default: executable basename (`claude`, `codex`, `bash`). Title is
permanently defined as *a human-readable, non-authoritative display
label chosen by Corral*, so user naming or provider-derived titles can
change the source later without changing the field's meaning.
Compatibility-facing: `HUMAN_REVIEW_REQUIRED`.

**Q4 — detach chord → `Ctrl-\` (0x1C), with both arguments rejected.**
M1: unconditional interception by the interactive attach client, never
forwarded to the PTY, no literal escape, no keybinding configuration.
Recorded as a conscious limitation: *a literal 0x1C cannot be sent to
the child through Corral's M1 interactive terminal attachment.*
Rejected reasoning: "SIGQUIT is nearly unused" (not a correctness
argument — this is a UX tradeoff we choose to pay) and "screen/telnet
users migrate for free" (inaccurate and unnecessary). Required tests:
ordinary typed 0x1C detaches; pasted/input-stream 0x1C still detaches;
detach while the child is in raw mode; the child continues after detach;
no 0x1C reaches the PTY writer. Later configurability is client-side
evolution and changes no terminal wire contract.

**Q5 — orphan policy → all three options rejected; model D.** The
recommendation (end the Run unverifiable, leave the process, report the
pid) was rejected along with killing the process group and adding an
orphan state. It collides with a frozen invariant: *loss of contact /
loss of runtime ownership does not equal process death.* After a corrald
crash the known fact is that PTY ownership is gone; that the process
exited is **not** known.

    1. the terminal/runtime binding is lost/ended
    2. the Run's execution outcome becomes Unknown / unverifiable
    3. Corral does NOT kill the surviving process
    4. Corral does NOT claim Exited/Ended
    5. the session stays honestly representable as having lost its
       managed runtime

Whether PR2's accepted vocabulary can express *ownership permanently
lost while process death is unknown* is a capability check that must be
performed, not assumed — the plan's "zero durable schema/event diff" was
not yet a proven fact. If it cannot: STOP; PR3 has found a durable
semantic gap, which is a Class C durable-state decision requiring human
acceptance before PR3 continues — not a place to make `Ended` stand in
to save ceremony. Separately: a last-known pid is not a trustworthy
entry point for "the user can kill it themselves" — pids are reused
across a restart. Frozen invariant: *loss of PTY ownership terminates
Corral's ability to manage the runtime; it does not prove that the OS
process exited.*

**Q6 — multiple viewers → (a), with an anti-feedback invariant.** One
managed terminal may carry several attached data channels: each attach
takes an independent initial snapshot and joins the same authoritative
delta stream; a slow viewer is handled by the bounded subscriber policy
and must never stall the PTY or runtime authority. All attached clients
may submit input; the advisory lease is never a server-side correctness
gate. Geometry is shared authoritative session/runtime state, not
per-viewer: last **explicit** resize wins. The invariant the
recommendation lacked:

> A client sends a resize only because its own local desired geometry
> changed (or on initial attach). It must never automatically echo a
> resize merely because it received a new server geometry/epoch.

Without it, two viewers of different sizes reassert forever. Each
authoritative geometry change follows ADR 3's epoch semantics and all
viewers eventually converge. The imperfect experience — the
last-operated size wins between two active differently-sized viewers —
is accepted for M1; no smallest-common-geometry, primary viewer, resize
ownership, or enforced control lease until real need appears.

**Q7 — `corral new` → accepted, title corrected.** `corral new --
<cmd>` creates and immediately attaches; no `--detached`, no `--name` in
PR3 — the walking skeleton needs one complete producer→attach path, not
background session-management UX. The session id prints to stderr before
interactive mode; stdout carries no control metadata. Title never
concatenates argv (Q3): `corral new -- claude
--dangerously-skip-permissions` yields title `claude`. Scope guard:
this CLI is a managed-runtime walking skeleton and does not define the
final M1 "New Session" product UX, which may still be provider +
working directory + advanced args. Corral's long-term product model is
not to be derived from this temporary harness.

**Q8 — large-geometry fixture → approved, rationale corrected.** 500
columns × 140 rows, per-cell varying truecolor foreground/background,
attribute variation, wide-character coverage, through the actual
snapshot encoder, asserting the actual encoded wire payload < 8 MiB,
kept in the permanent regression suite. Rejected reasoning: *"500×140 is
the upper bound of real hardware."* It is not — virtual displays,
extreme resizes, and automation can produce larger geometry. It is an
approved representative extreme stress case proving *this
representative extreme remains comfortably below the hard ceiling*, not
that all legitimate viewports are mathematically below that size. The
ceiling's other half needs its own test: a viewport-only encoded payload
over 16 MiB yields an explicit SnapshotTooLarge, no partial-success lie,
and a healthy daemon. Healthy-extreme evidence and failure-path evidence
are different jobs.

## Round 2

**Q9 — capability check → YES; not Class C.** PR2's accepted vocabulary
suffices and the zero durable schema/event diff stands. The reason is
not that `RunEnded` is a name we can just about live with: the accepted
semantics already cover this case. `RunEnded(Unverifiable)` means *the
managed Run episode is closed, and corrald could not establish that the
OS runtime exited* — never *the process exited*. Evidence: ADR 0002 D2
("when `corrald` cannot establish that it exited, the Run ends as
unverifiable — never assumed exited"); the accepted `RunEnded` event
documented as "ended, **or could not be established to have ended**";
and `RunEnd::Unverifiable` distinguished in PR2 from
`Exited(ExitCause::Unknown)`, which is the different fact of an observed
exit with an unobserved cause.

PR3 crash reconciliation therefore maps legally:

    PTY/runtime ownership irrecoverably lost
            ↓
    the managed Run episode can no longer continue
            ↓
    RunEnded(Unverifiable)          [Run lifecycle]
    execution_state = Unknown       [user-visible claim]

The two facts do not conflict, and the visible claim is never Exited. No
`Orphaned` or `OwnershipLost` durable discriminant is added: the
accepted vocabulary expresses the fact losslessly, and a new one would
manufacture a second representation of it. Required in the plan, in
code comments, and in test terminology, verbatim:

> `RunEnd::Unverifiable` closes Corral's managed episode; it never
> claims that the OS process exited.

Test invariant: daemon crash + a previously managed Run with no observed
process exit + restart reconciliation → `RunEnd::Unverifiable` **and**
`session.list` `execution_state == Unknown`, never Exited.

**Q10 — pid → not persisted, not probed.** `last_known_pid` is a runtime
diagnostic only: process-memory of the live corrald that spawned the
process, never persisted, never reconstructed after restart, never
post-restart identity evidence, never exposed as a safe kill target.
Restart reconciliation reads no persisted pid, calls no `kill(pid, 0)`,
inspects no `/proc` by stale pid, and infers neither survival nor death.
Persisting a pid would add durable schema merely to preserve weak
evidence, and even a still-existing pid is not the same process without
stronger identity evidence. After a restart, a previously live Run with
no observed exit is `RunEnd::Unverifiable`, period. A future phase
needing robust post-crash recovery must introduce an identity mechanism
strong enough for the control action it wants; PR3 does not prepay for
it with a persisted pid.

**Mechanism defaults — accepted with sharpened semantics.**

*Attach token*: 128-bit CSPRNG; TTL 30 s from issuance; bound to
`CorralSessionId` and the concrete Run/terminal runtime identity; not
bound to snapshot epoch or the issuing RPC connection's lifetime.
Redemption is one atomic operation — validate token, validate TTL,
validate runtime binding, mark consumed, transition the connection into
the terminal-data role — never check-then-later-mark, which would let
two clients both validate. Consumption is final (Q2.6).

*Per-subscriber delta queue*: 4 MiB of encoded queued data **per
subscriber**, not a shared session pool. Overflow behaviour, stated
precisely: *never drop an interior terminal delta and continue
pretending the stream is valid.* Over budget → mark the subscriber
desynchronized → close that data channel → the client re-attaches →
fresh snapshot on the current stream. Not "drop the oldest delta and
keep sending the newest", which would synthesize a plausible-looking but
wrong screen. A slow subscriber never backpressures the PTY reader,
never backpressures authoritative VT updates, and never slows other
subscribers — all three belong in tests.

*`TERM=xterm-256color`*: policy for the spawned PTY child's environment,
explicitly not the daemon's own environment identity — daemon
identity/lifetime must not depend on spawner-local environment, while a
managed Run's execution environment legitimately does. Not `TERM=ghostty`,
which would oblige Corral to guarantee that terminfo is available.
Revisit only if the emulator measurably emits beyond the
xterm-256color contract.

*Scheduled fuzz*: `scripts/fuzz-terminal` owns target/config semantics;
the CI scheduled job only calls it and invents no rules in YAML.
Nightly, 30-minute bound: evidence amplification only, never a normal
merge gate; failure → P1 triage; minimized reproducer → the permanent
deterministic corpus; material findings archived under the
`docs/evidence/` admission rules; successful nightly runs commit
nothing.

*Spawn compatibility gate*: accepted as a prerequisite **before Design 1
is built on**, not a mid-implementation check (Q1).

## Classification correction

The plan's original claim — "no new ruling, no human-gated surface" — is
withdrawn.

    Base class:              high-consequence Class B
    Human-gated surfaces:    new third-party dependency (portable-pty);
                             terminal.attach / data-channel protocol
                             additions; ClientHello terminal-role/token
                             extension; session.list's first concrete
                             compatibility-facing shape
    Founder decisions:       resolved by this grill
    Durable schema/event:    NO CHANGE (Q9 capability check)
    Merge:                   HUMAN_REVIEW_REQUIRED → human merge

Still not Class C — the protocol decisions are ruled here rather than
left open — but PR3 may not claim autonomous merge. The fresh
reconciliation review checks whether the rulings were faithfully
materialized; it does not re-grill them.

## Postscript

Q5 vindicated a piece of PR2: the slightly odd disjunctive semantics of
`RunEnded(Unverifiable)` were not surplus complexity. They are exactly
what caught the case where a daemon crash makes process death
unprovable.
