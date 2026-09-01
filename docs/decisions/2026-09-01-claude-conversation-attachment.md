# Claude conversation attachment — founder rulings

> Acceptance evidence for ADR 0011, 2026-09-01. Raised as PR6 review ruling R3
> (`docs/decisions/2026-08-31-pr6-review-surface-and-transport.md`), which left
> Claude's identical native-resume exposure as the immediately following task
> rather than widening PR6.

## The gap that made this a decision rather than a repair

PR6 closed the bypass for Codex under ADR 0010 D2's second ground: `resume` and
`fork` are *subcommands*, and ADR 0009 D1 declares the interactive TUI the whole
managed surface, so an argument selecting another surface is refused.

Claude has no such declaration, and its equivalent is not a subcommand.
`claude --resume <id>` runs the same interactive Claude Code, on the same
surface, with Corral's `--settings` injection intact and reporting normally.
Neither of ADR 0010 D2's grounds reaches it. What it does is attach a *fresh*
managed launch to a conversation that already exists — walking around the
per-Session continuation claim and the eligibility ladder that `session.resume`
holds, which binding uniqueness cannot substitute for because it answers only
after the second process has reported a completed turn.

So the harm PR6 named as "the sharpest case" of ground two turns out to be its
own ground, and the agent could not tell whether the accepted design covered it
(`docs/ENGINEERING_WORKFLOW.md` §2.2).

## R1 — A third ground, provider-neutral

Ruled: add the ground rather than stretch the second one.

> A caller argument that would attach the launch to a provider conversation
> that already exists is refused. `session.resume` is the only path authorized
> to do that.

Rejected in the same breath: redefining "surface" to mean "surface or
conversation". It keeps the count of grounds at two by making the word mean
something ADR 0009 D1 did not, and a later reader would misread how far that
declaration reaches.

## R2 — Only conversation attachment this round

Claude's other eighteen subcommands — `mcp`, `update`, `doctor`, `agents` and
the rest — are **not** refused by this task. Declaring a managed surface for
Claude, the way ADR 0009 D1 does for Codex, would refuse them; that is a
separate decision and is recorded as pending, not taken.

The reason is the one that kept it out of PR6: it is a user-visible change to a
provider this work was not commissioned to widen, and the conversation-attachment
harm stands on its own without it.

## What follows from R1 that was not asked about

Ground three is provider-neutral, so it also becomes the honest reason Codex's
`resume` and `fork` are refused. Nothing changes in the Codex code — they are
refused either way — but the reason recorded against them stops being the
surface declaration alone.
