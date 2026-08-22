# Founder Decision Record — Surface sequencing and the Desktop bar

> Status: founder-accepted, 2026-08-22. Materialized by the `ROADMAP.md` §3
> resequencing that lands with this record. Changes ordering only: no PR's
> content changes, no architectural invariant moves, and the M1 scope in
> §4 is untouched.

## The problem

Nothing in PR0–PR3 is usable by a person. The founder cannot hold the
product, so every design judgement until PR7 would be made without a single
day of real use.

That is not only uncomfortable, it is a schedule fact. `ROADMAP.md` §5
makes **14 consecutive days of normal dogfood use** and **≥100 trusted
Needs You transitions** release gates, and the evidence window counts only
after `STORAGE_EPOCH` reaches `dogfood`. The earliest dogfoodable build
therefore sits on the critical path: every phase that delays it delays M1
by that phase plus fourteen days.

## What was actually blocking it

Not the Desktop's position. The TUI — `list / new / attach / switch`,
already in the §4 M1 surface list — was bundled into PR7 with the Attention
Engine, and it does not depend on the Attention Engine. It depends on
session identity (PR2) and runtime ownership (PR3).

## D-S1 — The TUI is its own phase, after PR3

A minimal TUI becomes PR4; everything downstream shifts by one. The first
build a person uses daily arrives three phases earlier.

What PR4 honestly delivers: See and Control. Sessions launched through
`corral new` appear, can be attached, switched between, and survive the
terminal closing. What it does not deliver is Know — without attested
evidence every session reads Unknown, which is a first-class state and the
correct answer, not a hole to be filled with a guess.

Rejected: thinning PR2 to reach PR3 sooner. S5(e) of the activation grill
requires the first application-mutating RPC to land with the command
identity, replay and idempotency semantics its phase needs, and the first
such command is `corral new`. Moving the durable log out of PR2 moves it
into PR3 rather than shortening the path.

## D-S2 — The Desktop bar is restated, not removed

Replaced:

    The core loop must be demonstrable at PR7 through CLI/TUI, before any
    Desktop work.

With: no Desktop work begins before session identity, runtime ownership,
terminal streaming, and control are demonstrable in the TUI, and before
attested attention evidence exists (PR5).

The bar protects exactly one thing — the daemon's semantic model must not
be shaped by a graphical surface, and `AGENTS.md` forbids surface-specific
Session identity outright. A TUI already exercising identity, streaming and
control proves that as well as one that additionally renders attention, so
the bar does not need to wait for the Attention Engine.

It does still wait for PR5. A Desktop opened onto a screen of Unknown would
be rebuilt as each later phase adds meaning to render; waiting one phase
for hooks costs one phase and saves a round of graphical rework. Both
surfaces gain the five-state rendering at PR8: the TUI pays that extension
once and cheaply, and the Desktop pays it once rather than at every phase.

Rejected: starting the Desktop straight after PR3 or PR4. Nothing
technically prevents it — Desktop and TUI are two clients of one protocol,
and the daemon cannot tell them apart — but GPUI is the largest single
piece of work in the schedule and would be rebuilt against a still-moving
model at hooks, discovery and attention in turn.

## What this decision does not change

PR contents, M1 scope (§4), the release gate (§5), the five-state model, or
any accepted ADR. Ordering only.
