---
status: accepted
read_when:
  - deciding what the hook payload cap and its oversize marker guarantee
  - adding a provider whose payload reaches the relay by something other than stdin
  - deciding whether a caller's provider argument may be refused, and on what grounds
  - changing which surfaces of a provider a managed launch supports
---

# What the payload cap guarantees, and the two grounds for refusing a caller argument

Two rulings in one record, because they are one discovery arriving twice: ADR
0009 described a channel whose properties turn out to belong to the provider
and the operating system as much as to Corral. Each supersedes one clause of
that ADR **in part**. Everything else in it stands unchanged — D1's managed
surface and its rejections, D3's evidence vocabulary and what one event may
claim, D4's capability substitution and its trade-off.

Accepted 2026-08-31 on the founder's review of PR6 at `b0cf0cd`
(`docs/decisions/2026-08-31-pr6-review-surface-and-transport.md`, R1–R3), which
also ruled that the two may share one ADR. Measured evidence:
`docs/references/2026-08-31-pr6-codex-notify-matrix.md`, scenarios 11–13.

## D1 — The oversize marker is a promise about what the relay receives

**Supersedes in part:** ADR 0009 D2's "the 256 KiB cap with the oversize
marker", where it reads as a property of argv delivery.

`MAX_HOOK_PAYLOAD_BYTES` and `payload_omitted="oversize"` are Corral-owned
facts about a payload the relay is holding. They say nothing about whether a
provider can hand one over, and for a provider that delivers by process
argument the operating system answers first: Linux caps a single argv string at
`MAX_ARG_STRLEN` — 32 pages, about 128 KiB on a 4 KiB-page machine — and
`execve` fails with `E2BIG`. The relay process never exists. macOS bounds only
the 1 MiB total and imposes no per-string cap, which is why the same payload
that is delivered there is lost here.

So the guarantee, stated the way it can actually be kept:

> Corral's payload cap and its oversize marker govern payloads the relay
> receives. A transport between the provider and the relay may have its own
> ceiling, below Corral's; past that ceiling an event is lost with no delivery
> and no marker, because the marker is written by a process that never ran.
> Corral does not claim to make a systematic oversize visible where the loss
> happens before its own code.

The consequence is named rather than softened. On Linux, a Codex turn whose
notify payload exceeds the ceiling reports nothing at all: the fact goes stale,
and if it was the first turn the session stays unbound until a smaller one
completes. That is the same honest degradation as any undelivered event
(ADR 0004 D5), reached by a path Corral cannot instrument.

Rejected: lowering Corral's cap to sit under the OS ceiling. It would not move
where the failure happens — the payload is already in argv by then — and it
would shrink what the channel carries for every provider to describe a limit
that is one provider's and one platform's. Rejected: a second, provider-shaped
relay contract. ADR 0009 D2's own reasoning holds — two relay contracts is a
drift trap and the strictest consumer sets the bar. Rejected: wrapping the
notify program to move the payload off argv. The provider chooses argv and
appends after everything Corral wrote; there is nothing to wrap that is not
already past the ceiling.

Not decided: whether the ceiling should ever gate a launch, warn a person, or
be surfaced as a capability limit. Nothing yet shows it happening to anyone;
dogfood decides, and PR8 owns what a person is told about evidence gaps.

## D2 — A caller argument is refused on two grounds, not one

> **Superseded in part by ADR 0012 D1/D2.** The grounds below are now subsets
> inside a verified grammar: an argument whose parsing Corral has not validated
> is refused whether or not it matches any of them.
>
> **Superseded in part by ADR 0011 D1.** There are three grounds. The third is
> an argument that would attach the launch to a conversation that already
> exists — which Codex spells as a subcommand, so the surface ground below
> reached it, and Claude spells as a flag on the surface Corral manages, where
> nothing below reaches it. The original wording is kept as written.

**Supersedes in part:** ADR 0009 D5's "refuse exactly what defeats the
injection".

That criterion was written when defeating the injection was the only way a
caller argument could hurt. It is not. ADR 0009 D1 declares the interactive TUI
the whole managed surface, and the argv path let a caller step off it — most
sharply through `resume` and `fork`, which attach a second process to a
conversation a Corral-managed process may already be driving. `session.resume`
holds a per-Session continuation claim precisely to serialize that, and binding
uniqueness cannot stand in for it: that check answers when the second process
reports a completed turn, which is after both have been writing.

> A caller's provider argument is refused on either of two grounds: it would
> displace or disable Corral's own injection, or it selects a provider surface
> Corral has declared it does not manage. The two are different failures and
> are said differently to the person.

Enforcement follows the declaration rather than a judgement per subcommand: for
a provider whose managed surface is declared, every argument selecting another
surface is refused, not only the ones whose harm is currently obvious. A
subcommand-by-subcommand list would need a fresh ruling each release and would
be wrong by default for whatever arrives next.

A refusal must read the command line the way the provider's own parser reads
it. A validator that misreads it fails in both directions at once: it refuses
prompt text that merely looks like a flag, and it waves through a subcommand
hiding behind an option it mis-measured. So the refusal honours the provider's
option/value arity and its end-of-options separator, and the list is
version- **and platform**-sensitive by nature — a claim about one release on
one target, held against what the matrix records.

Rejected: refusing only `resume` and `fork`. It leaves a declared boundary
unenforced everywhere else and makes every future subcommand a new decision.
Rejected: routing a caller's `resume` to `session.resume`. Corral would be
guessing which Session a provider-side id belongs to and executing a control
operation nobody asked for. Rejected: allowing it and relying on binding
uniqueness, for the timing above.

Not decided: whether the same two grounds change what Claude refuses. Its
identical exposure is the next task by ruling R3, not a side effect of this one.

## What this does not decide

Nothing about the notify channel itself, identity, contest semantics, or the
capability substitution — ADR 0009 keeps all of it. Nothing about external
sessions or global integration (PR7). Nothing about what a person is shown when
evidence is missing rather than absent, which is the attention phase's.
