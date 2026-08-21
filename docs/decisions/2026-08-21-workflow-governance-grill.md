# Engineering Workflow Governance — Founder Decision Record

> Status: founder-accepted decisions from the 2026-08-21 workflow/governance
> grill (third grill; companions: `2026-08-21-m1-decision-grill.md`,
> `2026-08-21-m1-ux-contract.md`).
> Scope: engineering workflow and governance only.
> Materialized into canon at PR0 — ten amendments to `AGENTS.md` and the
> full `docs/ENGINEERING_WORKFLOW.md`, each founder-acked per edit before
> the change landed. The workflow draft it supersedes is superseded in git
> history rather than kept as a second document.
> Per the in-grill ruling on documentation classes: materializing these
> decisions into canon is documentation materialization (Class B, citing
> this record as acceptance evidence); any normative deviation from this
> record is Class C.

## 1. Workflow founder decisions

### Two gates, not one (plan vs decision)

- Architecture/decision acceptance and per-PR implementation planning are
  separate gates. Founder approves decisions, not every implementation plan.
  Agents own implementation planning, not architectural authorization.
- Frozen canon (Dev Plan v2.0 + founder decision records) is accepted
  architecture intent. PR0 materializes the ADR-grade frozen decisions into
  formal `status: accepted` ADRs citing the original acceptance evidence —
  normalization, not re-approval.
- Every Class B/C PR still requires a written implementation plan before
  code, but a plan that implements accepted architecture, creates no new
  founder decision, changes no accepted invariant, and touches no
  human-gated surface proceeds without founder ack. Plan enters review with
  the diff.
- Hard STOP before crossing any new decision boundary: changed accepted
  ADR/invariant; durable schema/event semantics; wire compatibility; unsafe
  boundary; security/trust boundary; runtime/state ownership; session
  identity/lifecycle semantics; provider-config mutation policy; reopening a
  settled benchmark-ledger decision. STOP → proposal → explicit human
  acceptance → continue. Never "finish and let review decide."
- AGENTS.md replacement text (frozen): "Implementation must be preceded by a
  written plan for Class B/C work. A new or changed architectural decision
  requires explicit human acceptance before implementation crosses that
  decision boundary. Work that implements already-accepted architecture may
  proceed from an unblocked implementation plan without repeated founder
  approval."
- A plan is a thinking boundary, not a word count: tiny fixes get tiny
  plans; Class A has no plan tax; never pad to fill the template.

### Class system (decision-based, not surface-based)

- Class A = localized implementation/fix; no accepted-contract change, no
  high-risk surface. Class B = substantive implementation inside the
  accepted architecture envelope. Class C = creates/changes an
  architectural, durable, security, compatibility, or ownership decision
  requiring human acceptance.
- Touching a topic on the AGENTS.md architectural list does not make work C;
  changing a decision does. Implementing accepted `CorralSessionId` is B.
- Agent first-classifies; reviewer verifies; PR body carries
  `Class: / Reason: / Applicable escalation triggers:`. Founder may
  override; omission does not default everything to C.
- Escalation triggers forcing reclassification: semantic contradiction with
  an accepted invariant; durable schema/event contract change; wire/public
  compatibility change; unsafe-boundary expansion; security/trust-boundary
  change; ownership-authority change; new architecture-level concept; agent
  cannot determine whether accepted design covers the choice. Triggers are
  semantic signals, not mechanical path matches ("new crate" / "new
  persistent field" alone is a signal, not automatic C). Uncertainty → up.
- Misclassification: boundary not yet crossed → reviewer upgrades, add
  missing gate, continue. Class C boundary crossed without approval →
  governance finding; "the code is already written" is never grounds for
  acceptance; back to the decision gate.

### Merge authority and audit

- Class A: verify + 1 fresh-context review, no material findings, no
  machine-detected escalation surface → autonomous merge.
- Ordinary Class B: plan existed before implementation; inside accepted
  envelope; verify; 1 fresh-context review; no machine risk flag; no
  classification uncertainty; no unresolved P1/P2; no audit-debt conflict →
  autonomous merge + mandatory next-working-day human audit.
- High-consequence Class B (default owners: corral-core / session
  identity / lifecycle; corrald runtime ownership; PTY/process;
  discovery/binding/identity; provider integration; attention
  derivation/routing; protocol; durable state; security/trust): same, but
  TWO independent fresh-context reviews. Unsure which tier → high.
  Ordinary-tier defaults: GPUI/TUI/tray presentation, non-canonical docs,
  tooling not changing verification semantics.
- Independent reviews: writer context excluded; reviewers read plan + diff +
  architecture independently; no confirming a prior reviewer's verdict.
- Class C: explicit human acceptance of the decision + human merge. Always.
- Machine-detected high-risk surface → human merge regardless of claimed
  class; at minimum the high-consequence review policy applies.
- Mechanical risk-surface detector (PR0): diff touching SQLite
  schema/migrations; durable event definitions; protocol
  schema/compatibility fixtures; unsafe-enabled crates; security/trust
  configuration; ownership-boundary declarations; dependency-direction
  rules; provider-config mutation code; new third-party dependency;
  accepted ADR / architecture-invariant docs; detection-manifest schemas
  (once introduced) → `HUMAN_REVIEW_REQUIRED`. Agents cannot remove the
  flag. Machine uncertainty fails closed to human review.
- Dependency flag scope: new third-party dependency (crates.io, git,
  system/native library, build/runtime tool), existing-dependency
  major-version upgrade, newly enabled feature that materially expands
  capability/security/build surface, new native/FFI footprint. Internal
  workspace crate edges following accepted dependency direction do not
  trigger; ordinary Cargo.lock transitive churn does not trigger. Early-PR
  human-merge concentration is an accepted cost.
- Audit (`AUDIT_PENDING`, close by end of next maintainer working day):
  critical owners — read plan, review findings, verification evidence, and
  the complete non-generated diff; surface/presentation owners — Goal/plan/
  findings/machine-risk/verification + targeted diff browse, plus at least
  one random full-diff audit of a light-audit B per week; a material
  sampling miss escalates that owner to full-audit until trust is restored.
  Audit answers: class underestimated? envelope silently crossed? owner
  boundary correct? shared wrong premise across reviewers? new contract
  missed by the detector?
- Audit-debt fence: an owner boundary with an unaudited autonomous B merge
  accepts no further autonomous B merges (develop/review freely; merge waits
  or goes human). Unrelated owners unaffected.
- Audit object is the merge-time immutable diff/commit, never later HEAD.
  Audit/review coalescing is allowed (one human session covers old audit +
  new fix review) but responsibility records stay separate.
- P1 found in audit: freeze autonomous merge in the affected owner; human
  chooses revert vs fix-forward. Revert-first when security/trust or durable
  data at risk, wrong schema/event semantics, architecture invariant
  violated, wrong-target control risk, little downstream dependence, or no
  small obvious fix. Fix-forward when scope is narrow and understood,
  invariant intact, no data/security risk, fix smaller/safer than revert,
  or downstream legitimately depends on it. Resolution gets fresh review +
  verify. P2 → follow-up (may pause area if systemic); P3 → follow-up only.

### Durable state: three clocks

- Decision clock: additive schema change implementing accepted design
  (additive only; no change to existing field/event meaning, identity/key
  semantics, migration guarantees, ownership/source-of-truth; no
  reinterpretation of recorded facts) = Class B + detector flag + human
  merge, no new decision ceremony. Any semantic change to recorded
  facts/keys/discriminants/migration guarantees/ownership, any discard of
  non-rebuildable Corral-owned facts, anything uncovered by accepted
  architecture = Class C. Reviewer uncertainty fails closed to C.
- Storage compatibility clock: `storage_epoch = dev | dogfood | released`,
  a committed repository marker; dev→dogfood only by human maintainer
  commit. Before dogfood: dev DBs disposable; no migration tax. After:
  rebuildable derived state may still be wiped+rebuilt (tested rebuild,
  clear source of truth); Corral-owned non-rebuildable facts
  (acknowledgements, durable semantic event log, manual corrections/unlinks)
  require migration or explicit maintainer-approved destructive reset.
  Release-gate evidence windows (14-day / 100-transition) count only after
  the dogfood epoch and restart if their data is wiped.
- Public compatibility clock: wire discriminant/opcode permanence starts at
  the first external tagged release exposing that contract; pre-release
  renumbering is legal with tests/fixtures updated in the same change.
  Durable persisted event semantics harden earlier — at first write into
  non-rebuildable storage after the dogfood epoch: no silent
  reinterpretation; migrate, tombstone + new discriminant, or approved
  destructive reset with evidence-window restart.

### Cross-owner root cause, claims, and containment

- Priority: correct owner > preserve owner concurrency > single-PR
  convenience. "Repair the owner" does not mean the discoverer stuffs the
  producer fix into its own PR.
- Absorb into the current PR only when all hold: no active writer in that
  owner; no audit debt there; fix is local restoration of an accepted
  invariant; no owner-contract change; no new
  architecture/durable/wire/security decision; reviewable as the same
  violated invariant. Any of — active writer, AUDIT_PENDING, contract
  change, separate regression characterization needed, review becomes two
  independent problems — forces a split prerequisite PR. No LOC threshold;
  owner contention + contract surface + review coherence decide. Claim and
  audit-debt facts are repository-visible and force the split mechanically.
- Prerequisite-fix authorship: active owner writer > discovering agent (if
  owner unoccupied) > maintainer dispatch. Discoverer contributes the
  minimal failing regression/reproducer and the stated producer invariant.
- Consumer law (AGENTS.md replacement, frozen): "Repair the owner, not the
  symptom. Consumer-side normalization or permanent guards must not
  substitute for an owner fix. A consumer may fail closed temporarily when
  invalid upstream state would otherwise cause unsafe behavior, but it must
  not silently repair or reinterpret that state." Containment may reject
  unsafe input (degrade to Unknown / refuse control); must link the
  root-cause prerequisite; is removed when the owner fix lands.
- Task claims live in plan frontmatter (`writes:` / `reads:` owner lists +
  status); one active writer per owner boundary by default; readers/tests
  unrestricted. Class A need not pre-claim but must stop and coordinate
  before modifying an owner under an active B/C claim. Claim lifecycle:
  plan active → done on merge/abandon. Staleness: 2 maintainer working days
  without observable activity (branch commit, PR update, renewal,
  heartbeat) → STALE. Automatic takeover only when stale + no open PR +
  clean worktree + no newer activity + no conflicting claim: mark
  abandoned, record predecessor, acquire. Dirty/ambiguous → human
  resolution. Takeover releases the write lease only; never deletes
  branches/worktrees.

### Flakes, quarantine, and verification truth

- Principles: verification failure ≠ permission to retry until green;
  diagnostic rerun ≠ verification success; quarantine ≠ test removal.
- Flake proof: nondeterminism demonstrated at the immutable base/merge-base
  commit (same test, same runner class, both PASS and FAIL). One rerun pass
  = suspected only. PR-commit-only failure with stable base = treated as a
  regression. `scripts/flake-probe` is a sanctioned explicit diagnostic
  experiment (recorded commit, runner, counts, signature, timing); it is
  not merge-gate retrying.
- Innocent-PR path: failed canonical verify stays FAILED; one diagnostic
  rerun + targeted probe + base check allowed; record in PR body; open/link
  flake issue; PR blocked until root fix or formal quarantine; then rerun
  canonical verify under the explicit quarantine contract.
- Quarantine: Class B process change; HUMAN_REVIEW_REQUIRED; never
  autonomous merge; agent proposes, never self-approves. Human accepts
  evidence, the invariant losing its hard gate, owner, deadline,
  release-critical flag (`RELEASE_CRITICAL_GAP` when the only effective
  coverage of a key invariant).
- Owner = tracked flake issue + subsystem owner boundary (never an
  ephemeral agent identity). Repair assignment: active owner writer >
  dedicated dispatched fix task > dedicated flake-repair task. Lease: 3
  maintainer working days. First renewal allowed with recorded
  investigation/blocker/next step; second renewal is the last ordinary one;
  at the third expiry choose: dedicated P1 repair now, replace with
  equivalent-or-stronger stable coverage, or human-recorded accepted known
  gap (which never unblocks a release-critical invariant). Lease expiry
  without action: owner loses autonomous merge; release-critical gaps keep
  blocking release; silent auto-renewal forbidden.
- Quarantined tests keep running in CI, non-blocking, separately reported,
  history visible. Release-critical quarantine blocks shipping M1 unless
  repaired or replaced with human-confirmed equivalent coverage.
  Quarantine buys development continuity, not release confidence.
- Verification layers: `verify-fast` (iteration; not merge evidence; target
  p95 ≤ 3 min) · `verify` (the ONE merge gate; all deterministic
  correctness coverage; 15–25 min accepted in M1; target p95 ≤ 20 min) ·
  `verify-release` (strict superset; provider/version matrix, packaging,
  zero release-critical quarantines, migration verification; answers "can
  we release", never a second merge definition). Scheduled/nightly jobs may
  only amplify evidence (stress, fuzz, soak, repeated probes, breadth); no
  merge-critical invariant may live only there. CI may have multiple jobs
  but repository scripts own verification semantics; CI YAML never
  re-implements selection/quarantine/compat/release logic. Nightly red
  never invalidates past merges; new failure → triage, possible owner
  freeze; real P1 → P1 process.
- Verify budget is review pressure (`VERIFY_BUDGET_DEBT`), never a license
  to delete coverage: rolling p95 breach or a single PR adding ≥2 min
  median → explain cost; prefer parallelize/cache/split/optimize; human
  reviewer decides optimize-before-merge vs accepted temporary debt.
- Timeout widening requires measured evidence: timeout-only failure mode;
  slow runs complete correctly; CI-like measurement; recorded distribution
  (≥50 targeted runs or equivalent CI history; p50/p95/tail); no owner bug
  that should be repaired instead. Forbidden when slow cases show
  deadlock/missed wakeup/leaks/scheduler-sensitive correctness. "A timeout
  must never be widened to conceal uncertainty about whether the system
  eventually makes progress."

### Canon, drift, and law-in-flight

- Documentation class follows semantic authority, not file type:
  materializing/clarifying accepted truth ≠ creating/changing normative
  truth. AGENTS.md/GOVERNANCE/Workflow normative changes = C (pure
  editorial fixes A/B but detector-flagged; reviewer must confirm
  editorial). Glossary: add-only entry naming an accepted concept travels
  with its implementation PR (autonomous-eligible); modifying/deleting an
  existing entry = human-gated (wording-only B + human merge; semantic
  change C); a new entry introducing a new architecture concept = C even if
  add-only; uncertainty fails closed to C. Detector is a conservative
  guardrail (modify/delete lines flag; add-only does not), not a semantic
  classifier. ARCHITECTURE/PRODUCT prose: sync of accepted decisions = B;
  new/changed invariant or scope contract = C. Plans = process artifacts
  following task class. ADR: new proposal C; materializing an accepted
  decision B (citing acceptance evidence); changing an accepted decision C;
  evidence/status updates B. References/benchmark ledger: B, unless new
  evidence overturns/reopens a settled decision → C path. README/comments:
  A/B; no hidden canon — ordinary docs must not quietly establish
  normative rules.
- Attention-evidence precedence clarification (to materialize in
  ARCHITECTURE/AGENTS): source authority applies only to evidence still
  fresh enough to support its claim; a stale high-authority signal is
  invalidated/degraded by newer contradictory evidence (recompute, possibly
  Unknown); a low-authority source does not thereby inherit the right to
  assert the target state unless it is allowed to assert it.
- Reconciliation agent: task role, no constitutional privilege — worktree,
  plan, claim (`architecture-docs` etc. for the task's lifetime only),
  PR, review, merge policy like anyone. Canon is normally updated by the
  same PR that changes the owned behavior; reconciliation tasks exist for
  historical debt, cross-ADR consistency, periodic drift repair, and
  consolidation — not as the everyday docs entrance.
- Canonical prose review checklist (replaces evidence map for prose): for
  each normative claim — source of authority; materialization vs
  clarification vs semantic change; conflicts with
  PRODUCT/ARCHITECTURE/ADRs/ledger/code+tests; smuggled new
  nouns/ownership/capability/scope/precedence; removed/weakened
  constraints; would a future agent reading only this reach a different
  conclusion than current design?
- Drift: two layers. PR-local — any PR changing normative behavior answers
  "which canonical docs/ADR/tests own this behavior; still consistent?".
  Scheduled reconciliation scan — weekly during M1 and before
  phase/milestone boundaries (plus manual trigger after C accumulation or
  major landings), scanning ADRs since last pass, changed owner boundaries,
  changed public/durable/protocol contracts, C decisions,
  DOCUMENTATION_DEBT, and canon last touched before those; output is a
  drift report/PR, never auto-merged canon edits. Canonical P1 drift
  (contradictory instructions capable of producing wrong implementation) =
  DOCUMENTATION_DEBT: affected owner loses autonomous B merge until
  reconciled; code is not automatically wrong — determine authority,
  reconcile, fresh review, clear; undecidable → C. Canonical P2 drift
  (omissions/stale examples) = tracked issue, no freeze.
- Law-in-flight: merge-time law is authoritative. Every governance change
  PR carries a `## Transition` block: effective point, enumerated affected
  active plans (from the machine-readable plan registry), required action
  per plan (none / reclassify / add review / add gate / stop before
  boundary / explicit grandfather). No silent effect on work that already
  crossed a newly gated boundary — the transition must rule grandfather or
  stop/rework. Grandfathering is explicit, named, human-approved; never
  self-claimed. Merged history is not retroactively non-compliant unless
  the new law mandates remediation. Law authors own transition impact;
  in-flight tasks only execute stated transition requirements.

### Founder-binding and external contributors

- Gates bind the founder: no direct push to main, by anyone, ever. Founder
  minimums — A: verify + 1 fresh agent review + normal merge path; B: plan
  before substantive implementation + verify + review + machine gates +
  normal merge path; C: founder's own decision is immediate (founder IS the
  human authority) but the code still gets fresh review + verify + no
  direct push. Role asymmetry is decision authority only, never
  review/verify exemption.
- External PRs: never autonomous merge — human maintainer merges all.
  Agent review machinery runs as evidence amplification first (A/ordinary
  B ≥1 fresh review; high-consequence B: 2), then human final review/merge.
  Maintainer (or maintainer-agent triage) assigns the class; contributors
  answer simple surface questions in the template and are never guilty of
  misclassification. External patch at a Class C boundary: mark
  `BLOCKED — DECISION REQUIRED`, extract the decision to issue/ADR, human
  decides, then continue/modify/split/reject — the decision bar never drops
  because code already exists; direct external changes to
  AGENTS.md/GOVERNANCE/Workflow normative rules are issue-first + explicit
  maintainer acceptance + human merge. External contributors contribute
  code and evidence; they do not inherit internal autonomous-merge
  authority or the duty to operate the internal governance machinery.

## 2. Rule precedence (frozen chains)

1. Human approval of irreversible/high-cost decisions > machine-enforced
   risk boundaries > fresh-context review > autonomous throughput.
2. Safety / data integrity / user availability > machine-enforced invariant
   gates > fresh independent review > documentation timing > normal process
   latency.
3. Correct owner > preserve owner concurrency > single-PR convenience.
4. Accepted architecture → agent-authored plan → autonomous implementation
   inside the envelope → new decision boundary → STOP + human acceptance.
5. Merge-time law governs; the governance-change PR owns the transition.
6. Canonical authority follows semantics, not file extension.
7. A failed canonical verify remains failed until fixed or formally
   quarantined; metrics and deadlines never lower engineering gates.

## 3. Human approval gates (exhaustive)

- Class C decisions: substantive acceptance + human merge (founder's own C
  work: decision immediate, code still reviewed).
- Every machine `HUMAN_REVIEW_REQUIRED` flag (detector list in §1).
- Every durable schema/event diff, including additive Class B.
- Quarantine approval, renewals, third-expiry strategy choice, and
  equivalent-coverage confirmation for release-critical gaps.
- `storage_epoch` transitions; destructive reset of non-rebuildable facts.
- Emergency declaration (M2+; maintainer only) and emergency-C decision
  blocks.
- Grandfather clauses in governance transitions.
- P1 audit findings: revert vs fix-forward.
- All external PR merges.
- Normative changes to AGENTS.md / GOVERNANCE / Workflow.

## 4. Agent autonomy boundaries

Agents may, without a human: classify (reviewer-verified, default-up);
author plans and implement inside the accepted envelope; merge Class A and
flag-free ordinary/high-consequence Class B per the review tiers; add-only
glossary entries for accepted concepts; materialize accepted decisions as
Class B docs; acquire claims and auto-recover clean stale claims; propose
quarantines and run flake probes; add internal workspace dependency edges.

Agents may never: approve Class C decisions; remove machine flags;
self-declare emergencies; push directly to main; merge into an owner with
audit debt, documentation debt, or a P1 freeze; normalize/reinterpret
invalid upstream state or persisted facts; retry verification to green;
self-approve their own quarantine; autonomous-merge external PRs; treat a
prior agent's output as human approval.

## 5. Exceptions / emergency path

- M1/dogfood: NO emergency bypass. Explicit non-emergencies: release
  deadline pressure; dogfood streak/metric preservation; maintainer
  inconvenience; slow CI; "the fix is small"; "I already know the fix."
  The 14-day metric can never justify lowering gates; streak accounting is
  a metric-definition question.
- M2+ skeleton (legislated now): valid triggers — external users materially
  unable to use Corral; active data-loss/corruption; active
  security/trust incident; widespread wrong-target control; provider change
  causing production-wide unavailability. Maintainer-only declaration.
  Machine risk gates never bypassed; regression test when reproducible;
  verify's relevant mandatory subset runs; ≥1 fresh-context review before
  merge; second review/full audit deferrable but must close within 24h /
  next maintainer working day; emergency merge auto-creates audit debt and
  freezes the owner's autonomous merge. Emergency defers process
  obligations from before-merge to immediately-after; it never removes
  verification.
- Emergency × Class C: immutable decision evidence pre-merge (PR-body
  `Emergency Class C Decision` block: decision, reason, alternatives
  rejected, compat/migration consequence, approver, timestamp). ADR
  materialization within 24h / next working day (later only while the
  incident is active), citing PR + decision block + root cause. Missed
  deadline = governance P1 → `DOCUMENTATION_DEBT`: no Class B autonomous
  merge in the owner, no new C merge in the decision area except another
  active emergency, cleared only when the ADR lands and a fresh reviewer
  confirms code/docs agree. Durable-truth law holds even in emergencies:
  migrate, new discriminant, or approved destructive reset — never silent
  reinterpretation.
- Narrow standing exception to owner-repair: temporary consumer fail-closed
  containment (§1), linked to the root-cause prerequisite.

## 6. Rules removed or simplified (deltas to previously written law)

1. "Feature-class and architectural work requires an accepted plan or ADR
   before implementation begins" → replaced by the three-sentence
   decision/plan split (§1).
2. Class definitions: surface/topic-based → decision-based; implementing
   accepted architecture is B, not C.
3. "CI runs exactly `./scripts/verify` … no other definition of done" →
   one definition of MERGE-ready (`verify`) + `verify-release` + legal
   evidence-amplification scheduled jobs; CI YAML owns no semantics.
4. Absolute consumer-guard ban → fail-closed containment exception (reject,
   never normalize).
5. "A shipped wire discriminant is permanent" → clock-qualified: public
   permanence at first external tagged release; durable persisted semantics
   harden at first post-dogfood-epoch write.
6. Uniform double review for all B → tiered (1 ordinary / 2
   high-consequence).
7. Uniform full-diff founder audit → tiered by owner risk + weekly random
   full-diff sampling of light-audit PRs.
8. New-dependency human gate scoped to third-party deps, major upgrades,
   capability-expanding features, FFI; internal workspace edges and lock
   churn exempt.
9. Plan template hard cap read as a thinking boundary, not minimum
   ceremony; no padding tiny fixes into essays.
10. No LOC thresholds for cross-owner absorb-vs-split decisions
    (contention/contract/coherence decide).
11. Quarantine renewal: bounded (two ordinary renewals, then forced
    strategy change) instead of indefinite 3-day ceremonies.

## 7. Final PR0 governance changes (materialization checklist)

AGENTS.md (normative edits; acceptance = this record):
- §Scope discipline: three-sentence replacement (§1).
- §5-referenced implementation rule: consumer-law replacement text (§1).
- §Verification: three-layer structure; "one definition of merge-ready."
- §Durable state: three clocks; `storage_epoch`; rebuildable vs
  non-rebuildable categories.
- §Protocol: permanence clocks for wire vs persisted semantics.
- §Tests: quarantine-lease pointer (flake law mechanics live in Workflow).
- §Git: "no direct push to main — humans included."
- §Runtime truth: freshness-qualified authority sentence.

Engineering Workflow:
- §2: decision-based class definitions; escalation triggers; plan
  template + frontmatter claims (`writes:/reads:/status`); PR body
  `Class:/Reason:/Applicable escalation triggers:`.
- New section: merge-authority tiers; risk-surface detector contract;
  AUDIT_PENDING process + tiered audit + sampling; audit-debt fence; P1
  revert/fix-forward criteria.
- §4: claim protocol, staleness (2 working days), automatic-recovery
  conditions.
- §6: flake evidence standard; `scripts/flake-probe`; quarantine lease +
  renewal cap; quarantined-tests-keep-running; timeout-widening evidence
  standard.
- §7: verify / verify-release / scheduled-amplification layers; budgets
  (p95 3 min / 20 min) + `VERIFY_BUDGET_DEBT`; CI-calls-scripts-only rule.
- §8: review tiers with default high-consequence and ordinary owner lists;
  canonical-prose review checklist.
- §9: governance-change `## Transition` block requirement.
- §10: documentation class map; drift layers; weekly reconciliation scan;
  DOCUMENTATION_DEBT semantics.
- New: external-contribution flow (maintainer triage classification,
  review amplification, `BLOCKED — DECISION REQUIRED`); M2+ emergency-path
  skeleton (dormant until external users exist).

Appendix A additions: risk-surface detector; quarantine gate (human merge +
lease tracking); claim staleness check; drift scan; AUDIT_PENDING tracking;
verify budget monitoring; flake-probe.

PR0 also materializes the frozen ADR-grade decisions as `status: accepted`
ADRs (Class B materialization citing the three decision records).

Below the governance line (implementation detail, no further founder
decision required): detector path lists; flake-probe implementation; plan
registry/claims format; budget measurement mechanics; CI job wiring;
audit/debt tracking representation (labels vs files).
