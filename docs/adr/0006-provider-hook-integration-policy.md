---
status: accepted
read_when:
  - writing code that reads or writes a user's provider configuration
  - designing hook installation, merge, versioning, or uninstall
  - deciding what Corral may do when it cannot prove a safe merge
  - questioning whether integration should be opt-in
---

# Provider hook integration is default-installed, disclosed, and fail-safe

Corral's provider hooks are **core infrastructure, installed and enabled by
default** with the normal installation and transparently disclosed at first
run. There is no separate consent step. Settings offer a per-provider
*Disable Integration*, which enters an explicitly degraded awareness mode.

The permanent ban is **undisclosed or destructive mutation**, not default
installation:

- existing user and third-party hooks are preserved;
- merge ambiguity fails safe — never overwrite, degrade honestly, and ask
  the user to resolve;
- writes are atomic same-directory tempfile plus rename with mode
  preservation, comment-preserving structured editing, and
  backfill-before-overwrite;
- uninstall removes only Corral-owned changes, with no byte-for-byte restore
  promise.

Managed sessions (PR4/PR5) use launch-scoped injection only and never mutate
global agent configuration. Global hook configuration arrives at PR7 with
lock and owner identity; if safe coexistence cannot be proven, discovery
degrades to read-only heuristics rather than risking the user's setup.

## Why

An earlier framing required prominent onboarding consent before installing
hooks. That was reversed because the semantic *Know* of externally launched
sessions is part of the M1 thesis: if supported external sessions normally
lack semantic status, M1 has failed. An integration that most users never
enable cannot carry a thesis, and a consent dialog at first run buys
protection against a risk — configuration damage — that the safety rules
above already own directly.

The residual risk is honest and different: Corral's shim sits in the hot path
of every agent run for every user. That is why the fail-open budget is law
(`AGENTS.md` §Runtime truth) rather than a quality goal.

## Consequences

- The fail-open guarantee is a P0 quality bar, not a nice-to-have.
- Some configuration-sensitive users will be lost; mitigated by disclosure
  and one-click disable. Accepted.
- No byte-for-byte uninstall restore — honesty over an unkeepable promise.
- The A-thesis experiment must exclude users who disabled integration and
  users degraded by merge failure, and must track disable rate and
  merge-failure rate separately as delivery health (`ROADMAP.md` §6).
- PR7 concentrates the discovery-coverage and safe-coexistence release gates
  and is the schedule's highest-risk point.

Acceptance evidence: `docs/decisions/2026-08-21-m1-decision-grill.md` §1
(hook integration policy; supersedes the earlier opt-in framing recorded in
the same grill).
