---
status: ready
class: C
writes: [corral-tui, corral, corral-protocol, corrald, scripts-ci]
reads: [docs/decisions/2026-08-25-pr4-tui-grill.md, docs/decisions/2026-08-22-surface-sequencing.md, docs/adr/0003-terminal-snapshot-format.md, docs/adr/0007-managed-session-lifetime.md, PRODUCT.md, ROADMAP.md]
---

# PR4 — the minimal TUI, and the first surface a person uses

Every design item below was ruled in
`docs/decisions/2026-08-25-pr4-tui-grill.md` (Q1–Q7). This plans their
implementation and nothing else.

**Class C, and why.** Design 5 adds a compatibility-facing field to
`session.list`. The decision is accepted — the grill is the acceptance
evidence — so implementation proceeds, but the wire surface is real and the
merge is human-gated. An earlier draft of this plan said "no wire change";
that was wrong and is recorded here so the claim is not repeated.

## Goal

`list / new / open / switch`, over the daemon that already exists. The first
build a person can run daily, and the first that can be dogfooded — which is a
separate decision, taken separately (Design 8).

## Non-goals

No terminal pane composition (Q1). No session subscription or server push
(Q4). No TUI framework and no general widget layer (Q5). No attention states:
Working, Needs You and Ready arrive with the evidence that entitles them, in
PR8. No history, resumable list, or ranking beyond Design 3's ordering (Q3).
No reason taxonomy behind `terminal_access` (Q7). No epoch advance (Q6).

## Existing owner / architecture involved

`corral-client` owns activation and the RPC calls. `corral/src/terminal.rs`
owns raw mode, the attach loop, snapshot and delta application, and the
`Ctrl-\` detach — Design 1 reuses it whole rather than reimplementing it.
`corrald`'s `ManagedSessions::describe` owns what a session looks like from
outside; `runtime/session.rs` owns what the screen thread publishes for
readers that cannot ask it.

## Design

**1. Open is a takeover (Q1).** Selecting a session leaves the list UI,
restores the terminal to the state the attach loop expects, runs the existing
full-screen attach, and on `Ctrl-\` rebuilds the list and refreshes it
immediately. "Switch" is therefore navigate-then-Open; it needs no mechanism
of its own.

The invariant, recorded where the code lives:

> PR4 Open reuses the existing full-terminal attachment semantics; it does not
> introduce composed terminal rendering.

**2. The state projection (Q2).** One function, owned once, used by both the
TUI and `corral list`:

```text
Running → primary Unknown, secondary "Running"   → "Running · Status unknown"
Unknown → primary Unknown, secondary neutral runtime wording
Exited  → primary Exited                          (never "Exited · Status unknown")
```

and the invariant it exists to hold:

> Execution state may establish `Exited`, or secondary runtime truth. It must
> never manufacture Working / Needs You / Ready.

`corral list` today prints `execution_state` as a bare status column, which
under this ruling shows "running" as a main state. It moves onto the same
projection: one surface contradicting the other would be worse than either.

**3. Ordering (Q3).** `ManagedSessions::describe` sorts by start time,
newest first, ties broken by the existing deterministic id order. No wire
field is added for it — `SessionHandle` already holds `started_at`.

Scope stated at the call site: this orders the current daemon-visible list. It
is not history ordering, resumable ranking, or attention ranking, and the
default is adjustable rather than a wire compatibility invariant.

**4. Refresh (Q4).** A 1 Hz poll of `session.list`, plus an immediate refresh
on returning from Open and after any local operation known to affect the list.
No overlapping polls. An RPC failure puts the list into an explicit
unavailable presentation rather than showing the last snapshot as current.

**5. `terminal_access` (Q7).** `SessionListItem` gains:

```text
terminal_access: "available" | "unavailable"
```

Absent or unrecognized means unknown, never a known negative — so a client
that cannot read it still offers Open and reports whatever refusal comes back,
rather than disabling an action on a value it did not understand.

The daemon answers it without a round trip to the screen thread, the way
`execution_state` and geometry already are: `Published` gains a flag the screen
thread sets when it poisons. Unavailable when the screen is poisoned, or when
the screen thread is gone without having published a final screen — which is
exactly "a snapshot cannot be served", the question the field answers.

Presentation: a secondary line, never a main status, and never the word
poisoned. When unavailable, Open is refused before the keystroke rather than
after, the row stays in the list, and its execution state is unchanged.

> Terminal readability and attachability is a Corral capability fact, not an
> agent semantic status and not evidence of process death.

**6. The crate (Q5).** `corral-tui`, already named in
`scripts/check-dependency-direction`'s client-side list. A library crate, with
`corral tui` as the subcommand that launches it: the daemon already resolves as
`corral`'s sibling, and a third installed binary would add a second resolution
path for no gain. Implementation choice, not a ruling.

Hand-rolled drawing over the existing `rustix` raw mode: rows, selection,
keyboard navigation, the two-line status rendering from Design 2, a footer, a
prompt for `new`, redraw, and save/restore around the takeover. Only those
primitives — a general widget layer is what Q5 declined.

**7. `new` (Q1, Q4).** A prompt for the command, then `session.new` with a
freshly minted `command_id`, then straight into Open. The list refreshes on
return.

**8. The epoch is not advanced here (Q6).** `STORAGE_EPOCH` stays `dev` in
this PR. The advance is a separate, human-only commit after the maintainer has
actually run the loop.

## Interfaces or persistence changed

Wire: `SessionListItem` gains `terminal_access`. Additive — a result field an
older peer ignores and a newer peer treats absence as unknown — so no protocol
version change: version governs breaking change, capability governs additive
evolution (`docs/decisions/2026-08-25-protocol-2-acceptance.md`). Human-gated
regardless, because the protocol surface is touched.

Persistence: none. Terminal availability is live runtime state and is never
persisted as fact.

## Failure / unknown states

Daemon unreachable at start: the TUI reports it and exits, as `corral` already
does; it never starts a daemon differently from the CLI. Daemon lost while
running: the list shows unavailable and keeps offering a retry, and never
presents its last snapshot as current. Open on a session that ended between
the poll and the keystroke: the attach answers from the final screen, which is
the point of ADR 0007 L2. Open on `terminal_access = unavailable`: refused with
the reason already on screen. Open on an unknown `terminal_access`: attempted,
and whatever the daemon answers is reported.

## Tests

- The projection: Running renders Unknown with Running beside it; Exited
  renders Exited and nothing about status; Unknown renders Unknown with
  neutral runtime wording. Table-driven, and the regression is that no input
  produces Working, Needs You or Ready.
- `corral list` and the TUI produce the same primary and secondary text for
  the same session — one projection, asserted from both callers.
- Ordering: two sessions started in a known order list newest first; equal
  start times fall back to the deterministic id order.
- `terminal_access` is `unavailable` for a poisoned screen and for a lost
  screen thread, and `available` for a live one and for a finished session
  whose final screen is serviceable. Driven through the daemon, with the
  corpus's poisoning input rather than a test-only seam.
- A `SessionListItem` without `terminal_access` decodes, and the client treats
  it as unknown rather than unavailable — the absence-is-not-a-negative rule.
- An unrecognized `terminal_access` value decodes and is treated as unknown.
- Open, detach, and the list is refreshed on return rather than a second
  later.
- Polls do not overlap: a slow `session.list` does not accumulate a queue of
  requests behind it.
- An RPC failure moves the list into the unavailable presentation and out of
  it again when the daemon answers.

## Definition of done

- Designs 1–8, and `./scripts/verify` green on the final tree.
- Human-merged: `corral-protocol` is touched, so the risk-surface gate applies
  whatever the class.
- `PRODUCT.md` §8's terminology law holds in every string the TUI renders:
  Session is the only exposed domain noun, and Run, Binding, Assurance and
  Evidence do not appear.
- The plan moves to `done/`, and the epoch advance is *not* part of this PR.

## Plan size justification

One surface, one loop: see the list, open a session, come back. Every design
item exists so that a person can do that honestly — the projection so the list
does not overclaim, the ordering so it is predictable, the refresh so it is
current, `terminal_access` so a row does not silently refuse the one action it
offers.
