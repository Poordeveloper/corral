# Founder Decision Record — PR4's minimal TUI

> Status: founder-ruled, 2026-08-25, in one round over a prepared frontier.
> Materialized by `docs/plans/done/2026-08-25-pr4-minimal-tui.md`. Two of the
> seven rulings corrected the recommendation they answered; both corrections
> are recorded with the reasoning, because in each case the recommendation was
> wrong in a way that would have shipped.

## Q1 — What "Open" does → (a), takeover

```text
TUI list
→ Open
→ suspend/leave the list UI
→ run the existing terminal attach loop full-screen
→ Ctrl-\ detach
→ restore the list UI
→ immediately refresh session.list
```

PR4 does no terminal pane composition.

The reason is not that panes are Desktop's privilege. It is that the client
architecture has no local terminal model to compose with. Composition would
force PR4 to introduce, early: a second client-side VT emulator, per-visible-session
terminal state, pane geometry ownership, shared-session resize interaction, and
terminal rendering into arbitrary rectangles. None of that is needed to prove
PR4's session navigation and control loop.

And PR3 already froze that terminal geometry is **shared runtime state** with
**last explicit resize wins**. So a TUI pane resize is not presentation: it
changes the authoritative PTY geometry every viewer sees. PR4 does not solve
that early for a list UI.

> **PR4 Open reuses the existing full-terminal attachment semantics; it does
> not introduce composed terminal rendering.**

## Q2 — What the list shows → recommendation corrected

The recommendation was "every session's main status reads Unknown". That is one
step too far. `Exited` is already one of the frozen five, and it is the one that
needs no provider semantic evidence: reliably knowing the runtime ended is
enough.

```text
execution_state = Running   → primary Unknown, secondary runtime fact Running
                              rendered "Running · Status unknown"
execution_state = Unknown   → primary Unknown, secondary neutral runtime wording
execution_state = Exited    → primary Exited        (never "Exited · Status unknown")
```

Once Corral reliably knows the runtime is over, `Exited` is the strongest and
safest main claim available.

So the invariant to freeze is not "a main status never comes from execution
state". It is:

> **Execution state may establish `Exited`, or secondary runtime truth. It must
> never manufacture Working / Needs You / Ready.**

The difference matters: the weaker wording would have shown a session that
plainly exited as Unknown for every phase up to PR8.

## Q3 — `session.list` ordering → started-at descending, daemon-owned

Newest started session first; ties broken by a deterministic `CorralSessionId`
order. No `started_at` wire field is added for sorting alone: ordering is
`session.list`'s current product projection, and the producer decides it once
so that CLI, TUI and Desktop do not each invent a default.

Scope is exact. This orders **the current daemon-visible session list**. It is
not a history ordering contract, not resumable-history product ranking, and not
attention ranking; PR8's recent/resumable list may own richer semantics.

The ordering is observable behaviour, but the initial default is adjustable.
"started_at descending" is not written down as a wire compatibility invariant.

## Q4 — Refresh → polling at 1 Hz

A client refresh policy, not a wire contract. PR4 introduces no session
subscription, no generic event subscription, and no server push for list
refresh.

Refresh happens on the regular poll, immediately on returning from Open, and
immediately after a local operation known to affect the list — the UI does not
look stale for up to a second after something the person just did. Polls do not
overlap: a second is not queued while the first is in flight.

On RPC failure the list enters an explicit disconnected/unavailable
presentation rather than continuing to show an old snapshot as current truth.
Retry and backoff are implementation detail and need no protocol decision.

> PR4 uses polling because its only live-list need does not justify defining
> the later semantic event stream.

## Q5 — TUI framework → hand-rolled, recommendation corrected

The recommendation leaned to `ratatui`, at low confidence. Rejected, and the
reason follows from Q1: once Open is a full-screen takeover, the TUI needs only
session rows, selection, keyboard navigation, status and runtime-fact
rendering, a footer, redraw, and restore around the takeover. It is not a
terminal composition UI, and pane layout, widget composition and complex cell
rendering — the reasons to take a framework — are exactly what Q1 removed.

`ratatui`'s default backend is `crossterm`; the default can be turned off and
other backends exist, so it is not true that taking it forces `crossterm` to
own raw mode. But having to route around the default backend is itself the
signal that a session picker has not earned the dependency.

PR4 keeps the existing `rustix` raw-mode ownership and adds small Corral-owned
ANSI drawing helpers — only the primitives this list needs, and deliberately
not a general TUI framework. When multiple panes, richer lists or forms,
scrollable structured views, a modal system, or a large reusable widget surface
actually appear, `ratatui` gets compared again against real requirements. PR9
and Desktop are not bound by this.

It also avoids a new third-party dependency and its human gate now.

## Q6 — `STORAGE_EPOCH` → advanced separately, after PR4 merges

PR4 does not advance the epoch. After it merges:

```text
PR4 merge
→ normal verify
→ the maintainer actually runs the loop: list, open, interact, detach,
  reopen, exit and failure behaviour
→ the maintainer decides this is the start of real product-evaluation data
→ a separate commit: dev → dogfood
```

"PR4 is dogfood-capable" and "from this moment these data are compatibility
evidence" are not the same fact, and the advance must be an explicit,
repository-visible change. No additional mechanical hour or day count is
imposed as a gate; this is the already-frozen human epoch decision.

After it: non-rebuildable Corral-owned facts gain migration obligations, a
destructive reset requires explicit approval, and any release-evidence window
whose evidence is destroyed restarts. Never advanced by an agent.

## Q7 — A poisoned terminal is user-visible → accepted, and reclassified

Accepted as a **secondary capability fact** — and this is not a TUI
implementation detail. `session.list` cannot express it today, so it is a
compatibility-facing wire addition with an explicit human gate. A plan that
says "PR4 has no wire change" is wrong.

Forbidden:

- mapping a poisoned terminal to `execution_state = Unknown`; the process may
  still be reliably Running;
- inventing another primary product status for it.

It is a different dimension: **Corral's ability to present or control this
terminal**.

The wire increment is minimal and does not expose the internal word
"poisoned":

```text
terminal_access:
  available
  unavailable
```

The only thing that produces `unavailable` in PR4 is an authoritative terminal
screen that cannot be safely served. Presented as, or equivalently to:

```text
Running · Status unknown
Screen unavailable
```

Never as a main status reading `Poisoned`, `Broken` or `Error`: it is neither
an agent semantic state nor a runtime-death claim.

When `terminal_access = unavailable`: Open is disabled or predictably rejected,
the UI explains why before the click rather than after it, the session stays
visible, and execution state remains independently truthful.

PR4 builds no extensible reason taxonomy. If the only consumer needs to know
"this cannot safely be attached right now", the minimal availability contract
is enough; the cause goes to diagnostics and logs. Additive extension waits for
several attach-failure reasons a person actually needs to tell apart.

> **Terminal readability and attachability is a Corral capability fact, not an
> agent semantic status and not evidence of process death.**
