---
status: accepted
read_when:
  - considering Windows support, WSL2, containers, VMs, or SSH targets
  - deciding whether a session that Corral cannot see is a bug or out of scope
  - adding platform-conditional code to a core crate
---

# Platform scope: macOS and Linux in M1, Windows deferred

M1 targets macOS and Linux on the **host OS execution domain**. Containers,
VMs, WSL2, and SSH targets are future *nodes* — documented out of scope
rather than blind spots. Inside that domain, for supported provider versions,
every live session must be discovered regardless of terminal host including
tmux; a systematic gap there is a release blocker, while failing to see a
session inside a container is not.

Windows is deferred with an explicit re-entry trigger: user-demand evidence
or a cohort that requires native support. The first Windows step is
WSL2-as-a-node, reusing the Unix runtime; native ConPTY ownership comes
after. The Windows continuity model is pre-decided from Herdr's production
evidence — job-object child lifecycle, no live handoff, with upgrades and
crashes recovering through snapshot restore plus provider-native resume. Do
not attempt an FD-style ConPTY handoff.

## Why

Corral's differentiating claim is that it sees sessions it did not launch.
That claim needs a bounded, provable domain: "every session on this machine's
host OS" is auditable, while "every session anywhere" silently includes
execution domains that need a node story, remote transport, and trust — all
of which belong to M3 and later. Drawing the line at the host OS keeps the
release gate honest without narrowing the product's eventual reach.

Windows deferral follows the same logic in the other direction: the runtime
work is real (ConPTY ownership, a different process-lifecycle model) and
would land in the milestone whose purpose is proving a loop, not portability.
Pre-deciding the continuity model now prevents a future agent from attempting
the FD-passing design that does not exist on Windows.

## Consequences

- Container, VM, WSL2, and remote users see nothing in M1. Accepted and
  documented; routed to the node roadmap.
- Nothing Unix-shaped may leak into the protocol or the domain model:
  endpoints, not sockets or file descriptors, are the wire-level concept, and
  platform behavior stays behind platform modules.
- The discovery coverage audit is scoped to host-OS terminal hosts, which is
  what makes "systematic blind spot" operationally testable.

Acceptance evidence: `docs/decisions/2026-08-21-m1-decision-grill.md` §1.
