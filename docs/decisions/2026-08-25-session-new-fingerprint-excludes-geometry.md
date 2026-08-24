# Founder Decision Record — terminal geometry is not part of a command's identity

> Status: founder-accepted, 2026-08-25. Applies the Q12 fingerprint rule
> (`docs/decisions/2026-08-22-pr2-resume-lineage-acceptance.md`, restated in
> `docs/decisions/2026-08-24-pr3-durable-lifecycle-grill.md` Q2) to
> `session.new`'s concrete input set. Materialized in `corrald`'s
> `fingerprint()` and in `SessionNewParams`' documentation.

## What was asked

Q12 fixes that a command fingerprint covers "the command kind and every input
that affects the mutation", and Q2 explicitly declined to freeze that as
`argv + cwd + geometry`: whatever the final semantic inputs are, all of them
are in.

`session.new` carries `rows` and `cols`. They do affect the mutation — the pty
is created at that size — so the first implementation included them, as the
literal reading requires. Review found what that costs.

## The ruling

**Terminal geometry is excluded from `session.new`'s fingerprint.** `argv` and
`cwd` remain in it, and any later input that describes what the command *does*
joins them.

## Why

The fingerprint exists to make a lost response safe to retry. Geometry is the
one input a client cannot reliably repeat, because it comes from a terminal the
person may resize at any moment — including between the lost response and the
retry.

Including it produces the failure the mechanism exists to prevent:

```text
session.new(command_id=X, 24x80)
    ↓ response lost; a runtime is already running
person resizes the terminal
retry  session.new(command_id=X, 60x200)
    ↓ different fingerprint
CommandIdConflict
```

`CommandIdConflict`'s whole contract is that the id will never mean this
command. So the caller can neither retry nor learn what the first attempt
started — while a runtime it cannot name is running. A client that responds by
minting a fresh command id then starts the second agent this design exists to
prevent.

Geometry is also not what the command means. It is the first attaching client's
presentation preference: the daemon supplies a size when it is absent, and the
first attach reconciles it against the terminal the person actually has
(`AGENTS.md` §Client / daemon boundary — presentation-only state belongs in the
surface). A retry therefore replays, and the Session keeps the size its first
execution was given.

## The rule this leaves for later inputs

An input joins the fingerprint when it describes what the command does, and
stays out when it describes how a surface would like to render the result. When
the two readings are both defensible, the tiebreaker is whether a client can
repeat the value verbatim on a retry: an input it cannot repeat turns every
retry into a permanent conflict, which is strictly worse than a replay that
ignores it.

This does not loosen Q12. It settles which of `session.new`'s fields are
semantic inputs at all.

## Alternatives rejected

**Keep geometry in, and let clients re-send the original values.** It makes
correctness depend on every client caching what it first sent across a
connection loss, and a client that gets it wrong is punished with an
unrecoverable conflict rather than a harmless replay.

**Keep geometry in, and soften `CommandIdConflict` to a retryable answer.** It
would make one command id mean more than one command, which is the invariant
Q12 exists to hold.
