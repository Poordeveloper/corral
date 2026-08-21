# M1 Founder Decision Record — Strategy Grill

> Status: founder-accepted decisions from the 2026-08-21 strategy grill.
> Newer than, and where in conflict superseding, older statements in
> `Corral_Development_Plan_v2.0_EN.md`, pending reconciliation.
> Companion record: `2026-08-21-m1-ux-contract.md`.
> Routing: hook integration policy requires an ADR at PR0/PR6 (canonical
> AGENTS.md list item: mutation of the user's provider/agent configuration);
> release/kill criteria → `ROADMAP.md`; non-goals → `PRODUCT.md`.

## 1. Founder decisions

### Thesis hierarchy

- Managed runtime (B) = mandatory foundation. Failure ⇒ M1 incomplete: delay
  and fix, never a kill signal.
- Observed-session aggregation (A) = the primary differentiated product bet.
  The only kill-class item.
- Attention fidelity (C) = trust/quality bar. Failure ⇒ do not ship.
- M1 thesis: "Corral must reliably manage sessions it launches, and its
  reason to exist is that it also sees sessions it did not launch."

### A launch gate

- M1 "local" = the host OS execution domain. For supported Claude/Codex CLI
  versions, every live session must be discovered regardless of terminal
  host, including tmux. Systematic blind spots are release blockers.
- Containers / VM / WSL2 / SSH = future nodes (ADR-5 logic): documented out
  of scope, not blind spots.
- Supported provider guarantee = latest stable + previous tested release,
  carried by a supported-version matrix + fixtures + integration tests,
  maintained by Corral.
- Observed Know is part of the thesis: if externally launched supported
  sessions normally lack semantic Know, M1 has failed. Safe hook coexistence
  is therefore release-critical (this makes PR6 the highest-risk PR).

### Hook integration policy (final; supersedes any earlier opt-in framing)

- Hooks are core infrastructure: installed and enabled by default with the
  normal Corral installation. No separate consent step; installation is
  transparently disclosed.
- Settings provide per-provider Disable Integration; disabling enters an
  explicit degraded-awareness mode.
- The permanent ban is undisclosed/destructive mutation — not default
  installation.
- Existing user / third-party hooks must be preserved. Merge ambiguity ⇒
  fail safe: do not overwrite, degrade honestly.
- Uninstall removes only Corral-owned changes; no byte-for-byte restore
  promise.

### Notification gate and release gate (C)

- Heuristic/unverified evidence never generates Needs You notifications.
  Silent waiting for unhooked sessions is accepted over speculative
  interruption.
- Measured at user-visible outcome: known provider noise must be
  normalized/suppressed; the standard is zero avoidable false Needs You
  notifications.
- Release gate: 14 consecutive days of normal dogfood; ≥100 trusted
  Needs You transitions across Claude + Codex; zero avoidable false
  notifications; systematic missed states in supported hooked flows are
  release blockers.

### Herdr posture

- Vendor/refactor the code Corral explicitly needs (Apache-2.0, NOTICE /
  THIRD_PARTY provenance stating source and scope), then self-maintain.
- No upstream data (manifests or otherwise) in the production correctness
  path. Herdr remains a research/reference source, not a runtime dependency
  and not an upstream requiring sync.
- External posture: transparent attribution; neither hidden nor marketed.

### A-experiment design

- The kill test measures: under the normal full See + Know + Control
  experience, do users keep using externally launched sessions?
- Per provider, the full-experience cohort excludes users who actively
  disabled integration and users degraded by safe-merge failure.
- Two delivery-health metrics are tracked separately and never mixed into
  the behavioral kill criterion: active-disable rate and merge-failure rate.
  If either is high enough that the full experience cannot generally be
  delivered, that is an independent integration/delivery-model failure.
- Control validity floor (from the UX grill): the A verdict is valid only if
  the user's primary provider actually delivered at least rung-2 centralized
  control (Needs You resolvable inside Corral) during the window. Rung-3-only
  delivery voids the verdict and triggers an S3/provider-integration
  re-examination, not a kill. "Rung 3 is good enough to avoid a dead end;
  rung 2 is the minimum to judge whether Corral deserves to exist; rung 1 is
  the experience we actually want."

## 2. Accepted costs

1. Default-installed hooks put Corral's shim in the hot path of every agent
   run for every user ⇒ the fail-open guarantee is a P0 quality bar.
2. Default installation will lose some configuration-sensitive users;
   mitigated by transparent disclosure + one-click disable. Accepted.
3. Notification gate: unhooked/degraded sessions may wait silently —
   false-negatives accepted where assurance is low; precision first.
4. "Every" narrowed: container/VM/remote users see nothing in M1. Accepted
   and documented; routed to the node roadmap.
5. Version policy ±1: sessions from older CLIs may be invisible or degraded.
   Accepted with a documented matrix.
6. No byte-for-byte uninstall restore — honesty over an unkeepable promise.
7. PR6 concentrates two release gates (discovery coverage + safe
   coexistence) and becomes the schedule's highest-risk point.
8. Zero-avoidable-false-positives + provider-noise suppression = a
   permanently Corral-owned noise catalog and its maintenance cost.
9. Vendored Herdr code is self-maintained — maintenance cost accepted to
   keep competitor data out of the critical path.
10. The kill verdict depends on 5 qualified external users experiencing the
    full product — recruiting and qualification are real prerequisite work.

## 3. M1 non-goals

- Existing v2.0 §14 list stands: history UX, artifact browser, tmux-class
  splits/workspaces, plugin runtime/marketplace, Tailscale, SSH onboarding,
  mobile/web, cloud relay, enterprise permissions, universal semantic
  approvals.
- Newly ruled (strong scope barriers):
  1. Fleet/worktree orchestration (one-click N agents / N worktrees / task
     fleets) — non-goal.
  2. Agent-to-agent orchestration/pipelines (artifact passing, chained
     triggers, automated review pipelines) — non-goal.
  3. Generic notification hub — the Attention Engine serves coding-agent
     sessions only; no Slack/email/CI/GitHub/calendar sources.
  4. Transcript analysis/summaries/analytics beyond the minimal semantic
     processing attention requires: no session summaries, quality scoring,
     token/cost analytics, daily reports, productivity analytics.
- Items 1–2 are identity boundaries: M1 proves a session-first coding-agent
  control plane, not an agent operating system. Reopening them requires a
  fresh product case; the existence of session/event/control primitives is
  not a reason.
- Boundary non-goals: container/VM/WSL2/SSH sessions (future nodes);
  Windows (ADR 5).
- Posture: if A fails, no silent pivot into a Herdr competitor.

## 4. Kill / reconsider criteria

- Kill-class (A only): cohort = 5 qualified external daily coding-agent
  users on the full experience; window = 4 weeks post-M1. A is unproven if
  fewer than 3/5 users repeatedly use Observed-session actions
  (open/jump/attach/adopt-continue/resume) on at least 3 separate working
  days.
- A fails while B is healthy ⇒ stop and re-underwrite the "Herdr competitor"
  thesis separately. Healthy B usage is evidence, not permission to move
  the goalposts.
- Delay-class (B): unreliable managed sessions ⇒ M1 incomplete; fix before
  shipping.
- Block-class (C): release gate unmet ⇒ do not ship.
- Independent delivery failure: disable-rate or merge-failure-rate high
  enough that the full experience cannot generally be delivered ⇒ re-examine
  the integration/delivery model; A is not thereby falsified.
- Anti-masking clause (v2.0 §14): if the loop is not valuable, do not hide
  the problem behind history/remote/mobile features.

## 5. Remaining evidence questions (spike/measurement, not founder judgment)

1. S1 (scheduled): VT emulator serialization choice.
2. S2 (scheduled; scope extended): hook payload verification, plus
   safe-merge semantics — a real-world settings corpus (including other
   tools' hooks), a merge-ambiguity taxonomy, and the fail-safe trigger set.
3. Discovery ground truth: a coverage-audit harness (spawn sessions across
   Ghostty/Terminal/VS Code/iTerm/tmux and assert discovery) — the
   operational definition of "systematic blind spot".
4. Provider noise catalog: baseline of known false-signal patterns (the
   definition of "avoidable"), collected during the 14-day dogfood.
5. Delivery-health baselines: real disable and merge-failure rates
   (thresholds deliberately unset until data exists).
6. tmux/multiplexer process-tree attribution fidelity.
7. Coexistence durability: do Corral-owned hook entries survive providers
   rewriting their own config files?
8. Gate feasibility: does normal dogfood produce 100 trusted transitions in
   14 days? If not, extend the period (rule fixed, data pending).
9. Cohort recruiting: sourcing and qualifying the 5 external users; must
   start before the kill window.
