---
status: proposed
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
(`docs/references/architecture-benchmarks.md` row 44).

**The invariant.** A Session's identity never depends on any process that
ran it. A Run is one process episode of a Session. Nothing about a Run —
its pid, its terminal, its start time, its provider-side id — is ever
promoted into Session identity, and no Run is ever created from evidence
too weak to control on.

## D1 — A Run is a record with a Corral-minted id

`RunId` (UUID), owned by Corral, unique across Sessions. Runs carry an
ordinal within their Session for display only.

Durable events and runtime bindings must name a specific episode, and a
display ordinal cannot serve: correcting a wrong binding renumbers it, and
a renumbered reference is a rewritten fact. Rejected: an ordinal index; a
`(session, started_at)` composite, which is not unique under clock
adjustment and encodes a timestamp as identity.

## D2 — What begins and ends a Run

A Run begins when Corral holds **Deterministic or Attested** evidence that
a process is executing the Session's provider session:

- managed — `corrald` spawned the process, so identity holds by
  construction;
- observed — a hook event or equivalent provider-native signal carries the
  provider session identity and is corroborated by an observed process.

A Run ends when the process is observed to have exited, recording the cause
when determinable. When `corrald` cannot establish that it exited, the Run
ends as **unverifiable** — never assumed exited (AGENTS.md §Runtime truth).

**Open for the grill.** Does a Session known only through heuristic
correlation have a Run at all? The draft says no: it is a Session with
bindings and no Run, because a Run is what control operations act on and
heuristic evidence may never enable control. The cost is that the recent-
resumable list must render a Session with no Run at all.

## D3 — NativeResume: same Session, new Run

Recognition is binding resolution, not inference. Binding uniqueness on
`(node, provider, external_id, kind)` resolves a seen provider session to
its existing Session; a new Run opens under it. Heuristic correlation may
never create this edge — at most it proposes one for manual confirmation.

**Blocking unknown.** This assumes a provider keeps its session id stable
across resume. If a provider mints a new id, NativeResume cannot be
recognised from identity alone and needs a provider-supplied continuity
signal (for Claude, `transcript_path`). Spike S2 verifies this first-party
against current CLI versions and gates this decision; PR2 must not guess
which shape is true.

## D4 — ContextHandoff: a new Session with a recorded lineage edge

Handing context into a fresh provider session produces a **new Session**,
with a durable Corral-owned edge to its predecessor. Both can be live and
independently actionable at once, and `PRODUCT.md` §6 requires one row per
independently actionable branch.

The edge is a Corral-owned fact between two Sessions, not a binding:
bindings relate a Session to an external identity. Rejected: modelling a
handoff as a new Run of the same Session, which would put two live agents
behind one row and make "interrupt this Session" ambiguous.

## D5 — RuntimeMove: same Session, new Run, new runtime binding

The process changes, so the Run changes; the provider session does not, so
the Session does not. At most one control-capable runtime binding is active
per Session (`ARCHITECTURE.md` §1), so the previous binding ends or is
explicitly superseded before the new one is acquired.

## D6 — What is recorded durably

Added to the accepted event set: `RunStarted`, `RunEnded`,
`SessionForkedFrom`. Alongside the already-accepted `SessionCreated`,
`BindingAdded`, `BindingConfirmed`, `RunAttached`, `RunDetached`,
`CommandAccepted`.

`RunStarted`/`RunEnded` are the process episode; `RunAttached`/`RunDetached`
are a runtime binding becoming available or not. They are different facts:
a Run can outlive every attachment to it, which is the whole point of
"closing a surface never terminates managed work".

Never recorded: PTY bytes, raw hook events, provider transcripts, derived
status. Live runtime state stays runtime-owned.

**This expands a durable event set and therefore needs explicit
acceptance** (AGENTS.md §Durable state).

## D7 — Unlinked is not the same as unrelated

When Corral cannot establish whether a newly seen provider session
continues a previous one, it records no lineage. The Session exists,
unrelated, and a person may correct it — unlink is first-class UI, manual
link is CLI-only and warns (`PRODUCT.md` §6). A guessed edge would be a
fabricated fact, and every control decision downstream would inherit it.

## D8 — Archive and delete do not rewrite lineage

Archiving removes a Session from the active surface and ends nothing.
Deleting removes Corral-owned metadata only, never provider history. A
lineage edge naming a deleted Session is kept as a recorded fact with an
unresolvable target rather than silently erased: rewriting the log to hide
it would reinterpret a Corral-owned fact.

## Not decided here

Whether lineage appears on the wire in PR2 — under the frozen no-ghost-wire
rule it does not until a surface renders it. History indexing (M2);
structured approval (M2); Live synchronized control mechanics; remote
`RuntimeMove` (M3).

## Open questions for the grill

1. Does a heuristic-only Session have a Run? (D2)
2. Is the provider session id stable across resume for Claude and Codex,
   and what is the continuity signal if not? (D3, gated by Spike S2)
3. Is the `RunStarted`/`RunEnded` versus `RunAttached`/`RunDetached` split
   right, or is one pair enough? (D6)
4. Does PR2 serve any of this on the wire, or is PR2 daemon-internal with
   `session.list` gaining fields only when the TUI renders them? (D6, D9)

Acceptance evidence: to be recorded when the founder rules the open
questions. Status flips to `accepted` then.
