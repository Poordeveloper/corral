<!--
External contributors: fill in Goal and the surface questions at the bottom,
and leave Class blank — a maintainer assigns it, and a wrong guess is never
held against you (CONTRIBUTING.md).
-->

**Class:** <!-- A | B | C -->
**Reason:** <!-- why this class -->
**Escalation triggers:** <!-- none, or which applied (Workflow §2.2) -->

## Goal

<!-- One explicit goal. What observable behaviour or contract changes. -->

## Non-goals

<!-- Optional. What a reviewer might expect here but will not find. -->

## Evidence

<!--
The verification command and its result. Mandatory.
For a regression fix: the failure observed before the fix, and why the new
test fails on the pre-fix implementation.
-->

```
./scripts/verify
```

## Compatibility

<!--
Which external surfaces this touches, or "none":
wire protocol · durable events and store schema · CLI commands, flags, exit
codes · hook-shim contract and env vars · session/resume file paths · verify
script names and semantics · detection-manifest schemas
-->

## Risk / staging

<!-- Required when the diff approaches the staging threshold (Workflow §5). -->

---

<!-- Contributors: answering these is enough; the maintainer classifies. -->

Does this change touch:

- [ ] the protocol or anything on the wire
- [ ] storage, schema, or durable events
- [ ] runtime or PTY ownership
- [ ] provider integration or the user's provider configuration
- [ ] security, trust, or authorization
- [ ] an architecture invariant or an accepted ADR
