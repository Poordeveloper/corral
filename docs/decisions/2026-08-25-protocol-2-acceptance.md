# Founder Decision Record — protocol 2, and what a version is for

> Status: founder-accepted, 2026-08-25. Class C. Implemented under
> `docs/plans/2026-08-25-protocol-2.md`.

```text
Decision:
Increment protocol version because an existing method schema changed.

Rationale:
Capability negotiation is reserved for additive optional features.
Required request schema changes require protocol version change.

Compatibility:
No released clients exist.
Both client and daemon must move together.
```

## The ruling

```text
PROTOCOL_VERSION            = 2
MIN_COMPATIBLE_PEER_VERSION = 2
```

## Why, stated correctly

The reason is **not** "no tagged release exists, so the version may be changed
freely". It is that **protocol 1 can no longer meet the compatibility promise
it froze.**

The failure is not a client that does not know a new capability. It is two
peers declaring each other compatible in the handshake while understanding the
same method differently:

```json
old:  session.new { command }
new:  session.new { command, command_id }
```

With both declaring `protocol_version = 1`, the hello has already lied. That is
a protocol contract failure, and the version field is what exists to express it.

## Why not a capability

Capabilities answer one question:

```text
does this peer support feature X?
```

A missing `terminal.stream.v1` means the peer does not have that incremental
ability. That is additive and optional, and absence is a complete answer.

Here the parameter contract of `session.new` itself changed. Same method,
different meaning — not an optional feature absent. Routing it through a
capability would leave `protocol_version = 1` plus `session.new` meaning
nothing definite, and every client carrying a method-version matrix, a
capability gate and a fallback schema for one method. That is more complexity
than the version field costs.

**Capability remains for additive optional features only.**

## Why now is not too late

Wire permanence begins at the first external tagged release. That rule protects
a wire contract **already published to external consumers**. It does not say
that an unreleased version number may never be corrected.

Today there is no external release, no third-party client, and no persisted
external protocol history. So this is *correcting an unpublished contract*, not
*breaking a public one*.

## Two corrections to how this was described

**`command_id` is not "a new field".** Describing it as an optional field
addition is misleading. It is a **request identity semantics change**: it
changes retry semantics, command deduplication, and receipt lookup. That is a
protocol behaviour change, which is exactly why it needs a version and not a
capability.

**The floor is not permanently equal to the version.** `MIN_COMPATIBLE_PEER_VERSION = 2`
is right today. It must not harden into `version N implies floor N`. A future
protocol 5 whose every change since 3 was additive has a floor of 3. The
division stands: **version governs breaking change, capability governs additive
evolution.**

## Scope note

This is the last real wire gate before PR3's line of work is closed.
