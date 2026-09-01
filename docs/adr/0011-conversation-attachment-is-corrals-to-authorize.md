---
status: accepted
read_when:
  - deciding whether a caller's provider argument may be refused, and on what grounds
  - adding a provider whose CLI can attach a fresh launch to an existing conversation
  - changing what `session.resume` is the only authorized path for
---

# Attaching to an existing conversation is Corral's to authorize

**Supersedes in part:** ADR 0010 D2's "either of two grounds". There are three.
Everything else in ADR 0010 stands, including D1's transport ceiling and D2's
rule that a refusal must read the command line the way the provider's own
parser reads it.

Accepted 2026-09-01 (`docs/decisions/2026-09-01-claude-conversation-attachment.md`,
R1–R2), raised by PR6 review ruling R3. Measured evidence:
`docs/references/2026-09-01-claude-2.1.251-attachment-matrix.md`.

## D1 — The third ground

> **Amended by ADR 0012.** This ground still says what it says, but it no
> longer carries the safety weight alone: a managed launch now passes only
> arguments whose parsing Corral has verified, so a spelling missing from the
> list below costs a refused launch rather than a silent attachment.

> A caller's provider argument is refused when it would attach the launch to a
> provider conversation that already exists. `session.resume` is the only path
> authorized to do that.

`session.resume` holds a per-Session continuation claim and walks the
eligibility ladder — sufficient assurance, a Confirmed identity, no live Run,
an established exit — precisely so that two provider processes cannot drive one
conversation. A fresh launch carrying the provider's own attach argument reaches
the same conversation with neither. Binding uniqueness is not a substitute: it
answers when the second process first reports an identity, which is after both
have been writing, and its answer is to leave the *new* Session unidentified
rather than to stop anything.

The ground is provider-neutral, and it had to be. Codex spells attachment as a
subcommand, so ADR 0010 D2's surface ground reached it; Claude spells it as a
flag on the surface Corral manages, with the injection intact and hooks
reporting normally. A ground written around either spelling would miss the
other, and the harm is the same one both times.

It is about *existing* conversations only. A launch that creates a new
conversation is what a managed launch is for, and nothing here narrows it.

## D2 — What it does not extend to

This ground says nothing about which provider surfaces are managed. Claude has
no declared managed surface and does not acquire one here: its subcommands
other than the attaching one stay the caller's, and whether Claude should get a
declaration like ADR 0009 D1's is a pending decision, deliberately not taken
(R2).

Nor does it reach an argument that merely moves execution — a cloud or
remote-control session that starts a *new* conversation somewhere else. Where a
single argument can do either, and the provider decides from the value's shape
at runtime, the argument is refused: Corral cannot tell the two apart without
interpreting a value it has no business interpreting, and refusing is the
direction whose failure is loud and recoverable.

## Why not the alternatives

Rejected: broadening ADR 0010 D2's "surface" to cover conversations. It keeps
two grounds by making a word mean more than ADR 0009 D1 declared, and the next
reader inherits the confusion.

Rejected: routing a caller's attach argument into `session.resume`. Corral
would be picking which Session a provider-side id belongs to and performing a
control operation nobody requested.

Rejected: allowing it and relying on binding uniqueness, for the timing above.

Rejected: allowing it because the person asked for it explicitly. M1 offers no
override for a second native resume of a session that may still be live
(ADR 0007's grill Q7), and an argument is a weaker request than a command —
nothing about it says the person knows another process holds the conversation.

## What this does not decide

Whether Claude gets a declared managed surface, and with it a refusal of its
remaining subcommands. Whether an argument that relocates execution — Claude's
`--bg`, `--remote-control`, Codex's `--remote` — should be refused, degraded, or
left alone; that is an execution-location question and belongs with the phase
that owns remote nodes.
