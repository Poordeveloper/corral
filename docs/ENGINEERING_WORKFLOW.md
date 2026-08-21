# Corral Engineering Workflow

> Status: canonical process mechanics for Corral M0/M1.
> Supersedes the pre-PR0 drafts `Corral_Engineering_Workflow_v1` and `v2`,
> whose lineage remains in git history.
> Rules live in `AGENTS.md` (canonical law); this document is process
> mechanics only and never duplicates rule text. Document hierarchy:
> `docs/GOVERNANCE.md`. Founder acceptance for the governance model
> materialized here: `docs/decisions/2026-08-21-workflow-governance-grill.md`.

## 1. Purpose and reading order

Corral is developed heavily with coding agents. Code is cheap to produce;
understanding, review, integration, and long-term coherence are expensive.

> AI lowers the cost of writing code. It does not lower the cost of reviewing
> or owning it.

Reading order for any non-trivial task:

```text
AGENTS.md                      always (canonical hard rules)
nearest scoped AGENTS.md       if one exists for the touched subtree
this document                  process for the change class at hand
PRODUCT / ARCHITECTURE / ROADMAP / ADRs / benchmarks ledger
                               the sections routed to by the task
```

Benchmark basis: `docs/references/architecture-benchmarks.md` §15 records
which engineering practices come from which reference projects, with evidence.
This document does not restate it.

## 2. Unit of work and change classes

```text
one task -> one explicit goal -> one coherent change
         -> one verification story -> one reviewable PR
```

File count is not scope; semantic purpose is scope. Scope rules: AGENTS.md
§Scope discipline.

### 2.1 Classes

Class follows whether the work **produces a new decision**, not which
directory it touches. Implementing an already-accepted architecture is Class
B even when the topic appears on the AGENTS.md architectural list.

| Class | Definition | Ceremony |
|---|---|---|
| A — Bounded | localized implementation or fix; changes no accepted contract and touches no high-risk surface | read → reproduce or identify the violated contract → smallest coherent fix → focused tests → verify → review diff |
| B — Substantive | substantive implementation inside the accepted architecture envelope: cross-module work, provider features, runtime behavior, new surfaces | written plan in `docs/plans/` **before implementation**; no repeated founder approval while the work stays inside the envelope |
| C — Decision | creates or changes an architectural, durable, security, compatibility, or ownership decision | explicit human acceptance of the decision → plan → staged implementation → compatibility/migration tests |

### 2.2 Escalation triggers

Reclassify upward when any of these appears, at any point in the task:

```text
the change semantically contradicts or alters an accepted invariant
durable schema or event contract changes
wire or public compatibility changes
the unsafe boundary expands
a security or trust boundary changes
ownership authority changes
a new architecture-level concept is introduced
the agent cannot tell whether the accepted design covers the choice
```

These are semantic signals, not mechanical equivalences. A new crate or a new
persistent field is a signal to look, not an automatic Class C: adding a
planned crate that follows accepted layering is B; adding a sidecar crate to
route around an existing owner is C.

On a trigger, stop before crossing the boundary — never finish first and let
review decide. Uncertainty resolves upward.

### 2.3 Classification authority

The implementing agent classifies first; the reviewer verifies. The PR body
carries:

```text
Class:                          A | B | C
Reason:
Applicable escalation triggers: none | <list>
```

The founder may override a classification. Omission of a class in a task
request does not default the work to C.

Misclassification handling:

- boundary not yet crossed → reviewer requires the upgrade; add the missing
  plan or human gate and continue;
- a Class C boundary crossed without acceptance → governance finding; the
  work returns to the decision gate. Existing code is never grounds for
  acceptance.

### 2.4 Plans

Class B and C work is preceded by a written plan (AGENTS.md §Scope
discipline). A plan is a thinking boundary, not a word count: a small fix
gets a small plan; never pad one to fill the template. Hard cap: one page /
~60 lines.

```text
---
status: active | blocked | done
class:  A | B | C
writes:                         # owner boundaries this task will modify
  - <owner>
reads:                          # owner boundaries it only reads or tests
  - <owner>
---

Goal
Non-goals
Existing owner / architecture involved
Design
Interfaces or persistence changed
Failure / unknown states
Tests
Definition of done
```

A plan that implements accepted architecture, creates no new founder
decision, changes no accepted invariant, and touches no human-gated surface
proceeds without founder acceptance; it enters review together with the diff.

## 3. Preflight routing

Guidance, not ceremony. The two enforceable cores are:

1. **Search before create** — find the existing owner/concept before adding a
   module, trait, state enum, protocol message, persistent field, or helper
   (checked in review).
2. **Inspect real dependency contracts** — read current source/types/docs of
   third-party APIs instead of inferring from memory or wrappers (checked in
   review when dependencies are involved).

Also: for bugs, reproduce or identify the exact violated observable contract
before editing whenever practical. Read the complete owning module and the
relevant caller/callee path, not just the search hit.

## 4. Worktree, claims, and multi-agent protocol

Hard rules: AGENTS.md §Git / worktree safety.

Conventions:

```text
shared integration checkout:  ./corral (or the main clone)
task worktrees:               ../corral-worktrees/<task-slug>
task branches:                task/<id>-<slug>
```

- Use a dedicated worktree when another implementation is in progress, the
  task is substantive, or multiple agents edit concurrently.
- Never create nested worktrees. Edit and test in the selected worktree
  consistently.
- Do not follow absolute worktree paths copied from another agent without
  verifying the active checkout.

### 4.1 Owner claims

One active writer per owner boundary by default; readers and tests are
unrestricted. The claim lives in the plan frontmatter (`writes:` / `reads:`)
— there is no second registry to maintain.

- Before writing in an owner, check the active plans for a claim on it.
- Class A work need not pre-claim, but must stop and coordinate before
  modifying an owner under an active B/C claim.
- Discovering mid-task that the fix belongs in another owner: check that
  owner's claim first (§5.1).

### 4.2 Stale claims

A claim without observable activity — branch commit, PR update, explicit
renewal — for **2 maintainer working days** is marked STALE. Stale is not
abandoned.

Automatic takeover is permitted only when all hold: the claim is stale, there
is no open PR, the relevant worktree is clean, no newer activity exists, and
no other active claim conflicts. The taking agent records the predecessor and
acquires the claim. Dirty or ambiguous state requires human coordination.

Takeover releases the write lease only. It never deletes a branch or worktree.

## 5. Implementation discipline

Rules: AGENTS.md §Scope discipline (including owner repair and the
fail-closed containment exception), §Existing concepts, §Comments, §Rust.

Numbers (review pressure, not lints):

- Modules: prefer ≤ ~500 production LoC; if a central file is at ~800, put
  substantial new behavior in a focused module instead. Do not split merely to
  satisfy a number if it destroys cohesion.
- Diffs (non-mechanical): complex logic normally < ~500 changed lines; at
  ~800, an explicit staging check is required in the PR. A larger coherent
  invariant is allowed when splitting would be unsafe and the PR says why.

### 5.1 Cross-owner root causes

Priority: **correct owner > preserve owner concurrency > single-PR
convenience.** Repairing the owner does not mean the discovering agent puts
the producer fix in its own PR.

Absorb the fix into the current PR only when all hold:

```text
no active writer in that owner
no audit debt in that owner
the fix is local restoration of an accepted invariant
no owner contract changes
no new architecture / durable / wire / security decision
reviewable as the same violated invariant
```

Split into a prerequisite PR when any of these appears: an active writer, an
`AUDIT_PENDING` in that owner, a contract change, a need for separate
regression characterization, or a review surface that becomes two independent
implementation problems. Claim and audit-debt state are repository-visible
facts, so this split is forced mechanically rather than argued.

There is no line-count threshold for this decision. Owner contention,
contract surface, and review coherence decide.

Prerequisite-fix authorship, in order: the active owner writer; the
discovering agent if the owner is unoccupied; maintainer dispatch when
priority conflicts. The discoverer contributes the minimal failing
regression or reproducer and states the producer invariant.

## 6. Testing

Rules: AGENTS.md §Tests. Evidence mapping:

| Change | Required evidence |
|---|---|
| pure domain logic | unit tests |
| provider/history formats | real-format fixture/contract tests |
| session binding/assurance | scenario/invariant tests |
| session lifecycle, provider integration, runtime behavior, attention/status | integration tests (MUST add/extend) |
| protocol changes | compatibility + future-input tests |
| runtime/PTY lifecycle | integration + failure/lifecycle tests (detach, disconnect, restart, crash, handoff, unverifiable) |
| user-visible TUI behavior | insta snapshot coverage (mandate activates at PR7) |

Mechanics:

- Mock-provider harness (wiremock-style recorded/synthetic responses) is the
  sanctioned way to test agent behavior without networks or paid tokens; no
  test may call a real provider API.
- `corral-test-support` (established at PR2) is the sanctioned home for shared
  test helpers; do not proliferate per-crate copies. Test-only helpers stay
  out of production modules.
- Regression tests are named `<issue-number>-<short-slug>` and must fail on
  the pre-fix implementation; the PR states the observed pre-fix failure.
- Unit tests live in sibling `*_tests.rs` files included via `#[path]`.
- Do not test statically defined values; do not add negative tests for
  removed logic.

### 6.1 Flake evidence

A verification failure is never a licence to retry until green. A diagnostic
rerun is not a verification success.

Proving a pre-existing flake requires nondeterminism at the immutable
base/merge-base commit: the same test, the same runner class, both PASS and
FAIL observed. One rerun that passes establishes *suspected* flake only. If
only the PR commit fails while the base is repeatedly stable, the failure is
treated as a regression of that PR.

`scripts/flake-probe` is the sanctioned diagnostic experiment — it repeats a
targeted test and records commit, runner/platform, pass/fail counts, failure
signature, and timing evidence. It is an experiment, not a merge gate lottery.

While a flake is unproven and unquarantined, the PR that hit it stays blocked:

```text
failed canonical verify
  → one diagnostic rerun + targeted probe + base check
  → record in the PR body, open or link the flake issue
  → prove pre-existing flake → human-approved quarantine
  → rerun canonical verify under the explicit quarantine contract
  → merge
or
  → repair the flake → canonical verify green → merge
```

The second green run never erases the first red one.

### 6.2 Quarantine

Quarantine is a Class B process change. It always carries
`HUMAN_REVIEW_REQUIRED` and never merges autonomously: an agent proposes a
quarantine, never approves its own. Human acceptance records the flake
evidence, the invariant that temporarily loses its hard gate, the owner, the
deadline, and whether the gap is release-critical
(`RELEASE_CRITICAL_GAP` when this is the only effective coverage of a key
invariant).

Ownership is a tracked flake issue plus the subsystem owner boundary — never
an ephemeral agent identity. Repair falls, in order, to the active owner
writer, a dispatched fix task, or a dedicated flake-repair task.

Lease: **3 maintainer working days.** The first renewal is allowed with
recorded investigation, blocker, and next step. The second renewal is the
last ordinary one. At the third expiry, mechanical renewal is forbidden and
one of these must be chosen:

```text
A. dedicated P1 repair, started now
B. replacement by equivalent or stronger stable coverage
C. human-recorded accepted known gap
```

Option C never unblocks a release-critical invariant. An expired lease
without action costs the owner its autonomous-merge privilege;
release-critical gaps continue to block release. Silent auto-renewal is
forbidden.

Quarantined tests keep running in CI, non-blocking, separately reported, with
visible history. A silent `#[ignore]` is not a quarantine. Quarantine buys
development continuity, not release confidence.

### 6.3 Timeout widening

Widening a test timeout requires measured evidence that the test is correct
and its budget unrealistic:

```text
the failure mode is timeout-only, not semantic divergence or deadlock
slow executions eventually complete correctly
timing was measured on CI-like runners
the distribution is recorded, not guessed
  (>= 50 targeted runs or equivalent CI history; p50 / p95 / tail)
no owner bug exists that should be repaired instead
```

The new budget is derived from the measured tail plus a documented margin.
Widening is forbidden when slow cases show deadlock, missed wakeups, leaked
processes, or scheduler-sensitive correctness bugs — those are owner repairs.

## 7. Verification and CI contract

Three entry points with distinct jobs (AGENTS.md §Verification):

```text
./scripts/verify-fast     iteration gate, target p95 < ~3 min:
                          cargo fmt --check
                          clippy with the workspace deny set
                          focused/inexpensive tests
                          never merge evidence

./scripts/verify          THE merge gate, target p95 <= ~20 min:
                          verify-fast
                          full workspace tests
                          required integration / lifecycle tests
                          cargo-deny (advisories / licenses / duplicates)
                          protocol future-input tests
                          dependency-direction check
                          disallowed-methods boundary lints

./scripts/verify-release  release gate, strict superset of verify:
                          supported provider/version matrix
                          packaging / install / uninstall
                          multi-platform release checks
                          migration verification
                          zero release-critical quarantines
```

`verify` is merge-ready; `verify-release` answers "can we release" and is
never a second definition of done. Repository scripts own verification
semantics: CI may have several jobs, but CI configuration never re-implements
test selection, quarantine rules, compatibility logic, or release logic — it
calls the scripts.

Per-PR CI runs `./scripts/verify` on Linux plus the declared PR-metadata
checks (Appendix A). macOS coverage runs post-merge and on schedule, and is
required by `verify-release`; local development already verifies on macOS.

Scheduled jobs may only amplify evidence — stress, fuzz, soak, repeated flake
probes, compatibility breadth too expensive per PR. No merge-critical
invariant may be covered only there. A scheduled failure never retroactively
invalidates merged history; it produces a finding, may freeze the affected
owner's autonomous merge pending triage, and follows the P1 process if it is
a real correctness defect.

### 7.1 Verification budget

Budgets are review pressure, never a licence to delete coverage:

```text
verify-fast   target p95 <= 3 minutes
verify        target p95 <= 20 minutes on the canonical CI runner
```

Exceeding a budget does not block correct code. It creates
`VERIFY_BUDGET_DEBT`, owned by the repository tooling boundary, when the
rolling p95 persistently exceeds the target or a single PR adds ≥2 minutes to
the median. The PR then explains the cost, and the preferred repairs are
parallelization, caching, fixture splitting, harness optimization, and
deterministic sharding. Moving unique correctness coverage out of the merge
gate is not an available repair. A human reviewer decides between optimizing
before merge and accepting temporary debt.

## 8. Merge authority and audit

Merge permission derives from class, machine risk detection, verification,
independent review, and audit-debt state — never from the implementing
agent's own assurance that the change is safe.

### 8.1 Tiers

| Class | Conditions for autonomous merge |
|---|---|
| A | `verify` green; ≥1 fresh-context review with no material findings; no machine-detected risk surface |
| B — ordinary | plan existed before implementation; work stayed inside the accepted envelope; `verify` green; **1** fresh-context review, no material findings; no machine risk flag; no classification uncertainty; no unresolved P1/P2; no audit-debt conflict |
| B — high-consequence | same, with **2 independent** fresh-context reviews |
| C | never — explicit human acceptance of the decision plus human merge |

High-consequence owner boundaries (default):

```text
corral-core / session identity / lifecycle
corrald runtime ownership
PTY / process
discovery / binding / identity
provider integration
attention derivation / routing
protocol
durable state
security / trust boundary
```

Ordinary owner boundaries (default): GPUI presentation and layout, TUI
presentation, tray presentation, non-canonical docs, tooling that does not
change verification semantics. When the tier is unclear, use
high-consequence.

Independence means the writer's context does not review, each reviewer reads
the plan, diff, and relevant architecture on its own, and no reviewer merely
confirms a verdict it was shown.

Any machine-detected high-risk surface requires human merge regardless of the
claimed class, and at minimum the high-consequence review policy.

### 8.2 Risk-surface detector

A repository check inspects each diff and sets `HUMAN_REVIEW_REQUIRED` when
it touches:

```text
SQLite schema or migrations
durable event definitions
protocol schema or compatibility fixtures
unsafe-enabled crates or the unsafe boundary
security / trust configuration
ownership boundary declarations
dependency-direction rules
provider configuration mutation code
a new third-party dependency, a major-version upgrade of one, a newly
  enabled feature that materially expands capability/security/build
  surface, or a new native/FFI footprint
accepted ADRs or architecture-invariant documents
glossary entries that are modified or deleted
quarantine state
detection-manifest schemas (once introduced)
```

Internal workspace crate edges that follow the accepted dependency direction
do not trigger the flag; ordinary lockfile transitive churn does not trigger
it. Agents cannot remove the flag. Detector uncertainty fails closed to human
review.

### 8.3 Post-merge audit

Every autonomous Class B merge produces `AUDIT_PENDING`, closed by a human by
the end of the next maintainer working day.

| Owner tier | Audit reads |
|---|---|
| critical (the high-consequence list) | plan, review findings, verification evidence, and the complete non-generated diff |
| surface / presentation | Goal and non-goals, plan, findings, machine-risk result, verification evidence, and a targeted diff browse |

At least one light-audit PR per week is additionally chosen at random for a
full-diff audit. A material miss found by sampling escalates that owner to
full-audit policy until later audits restore confidence.

The audit answers: was the class underestimated; was the accepted envelope
silently crossed; is the owner boundary correct; did both reviewers share a
wrong premise; did a new contract escape the detector.

The audit object is the merge-time immutable diff and commit — never a later
HEAD. A root-cause fix may modify the same code before its audit closes;
audit and review may be coalesced into one human session, but the two
responsibility records stay separate and are both answered.

### 8.4 Audit-debt fence

While an owner boundary holds an unaudited autonomous Class B merge, no
further Class B merges autonomously in that owner. Development, review, and
PRs continue; the merge either waits for the audit or goes through a human.
Unrelated owners are unaffected.

### 8.5 Findings after merge

A P1 finding freezes autonomous merge in the affected owner. A human chooses
between revert and fix-forward:

```text
revert first when
  a security or trust boundary was violated
  durable data is at risk
  schema or event semantics are wrong
  an architecture invariant was violated
  wrong-target control is possible
  downstream work does not yet depend on it
  correctness cannot be restored by a small obvious fix

fix forward when
  the defect is narrow and understood
  the invariant remains correct
  no data or security risk exists
  the fix is smaller and safer than the revert
  downstream commits already legitimately depend on the change
```

The resolution gets a fresh review and verification. An agent never elects to
keep an architecture or security P1 because downstream work already exists.
A P2 finding becomes a follow-up and may pause autonomous merge in that area
when it suggests systemic risk; P3 is a follow-up only.

## 9. Review protocol

Rules for review content/output: AGENTS.md §Review. Severities:

```text
P0  catastrophic / security / severe data loss
P1  correctness, state loss, protocol/architecture violation
P2  meaningful bug or maintainability issue worth fixing now
P3  optional improvement; omit unless it materially helps
```

Review staffing follows the merge tiers in §8.1: one fresh-context review for
Class A and ordinary Class B, two independent fresh-context reviews for
high-consequence Class B, and for Class C review plus explicit human
acceptance.

For B/C, the reviewer builds the **evidence map** and states any missing cell
as a gap instead of guessing:

```text
changed surface | entry point | owner boundary | one caller + one callee |
invariant-sharing siblings | existing tests | current main behavior
```

Verify the premise before treating an apparent gap as unfinished work:
`git log -p -S <symbol>` — deleting intentional design is the most common
AI-review failure.

### 9.1 Reviewing canonical prose

Canonical documents get fresh-context review like code, but the evidence map
is replaced by a documentation checklist. For each normative claim:

```text
1. what is the source of authority — accepted ADR, founder decision,
   architecture invariant, current implementation contract?
2. is this materialization, clarification, or semantic change?
3. does it conflict with PRODUCT, ARCHITECTURE, accepted ADRs, the
   benchmark ledger, or current code and tests?
4. does it smuggle in a new noun, ownership, capability, scope, or
   precedence?
5. does it remove or weaken an existing constraint?
6. would a future agent reading only this reach a different conclusion
   than the current design?
```

## 10. PR and commit discipline

- Commits: Conventional Commits (`feat|fix|refactor|docs|test|chore(scope):`),
  linted in CI.
- One task → one focused PR; no unrelated refactors.
- Issue-first policy: NOT required for internal (founder/agent) work — the PR
  body's Goal/Non-goals is the task statement. Required for external
  contributions and for new product-scope features. Re-evaluate at ≥3 regular
  contributors.
- The PR body is the durable explanation — updated by editing, not buried in
  comment threads. Template:

```text
Class             # A | B | C
Reason            # why this class
Escalation triggers  # none, or which applied
Goal
Non-goals (if useful)
Evidence          # verification command + result summary line (mandatory);
                  # for regressions: the observed pre-fix failure
Compatibility     # which external surfaces are touched, or "none"
Risk / staging    # required when the diff approaches the staging threshold
```

- Breaking-change surface checklist — any diff touching one of these gets
  compatibility review:

```text
wire protocol (methods / fields / discriminants / opcodes)
durable event types and store schema
CLI commands, flags, exit codes
hook-shim contract and env vars
session/resume file paths
verify script names and semantics
detection-manifest schemas (once introduced; compatibility-sensitive
  external surfaces)
```

- The author/agent must be able to explain the entire change. Raw AI output is
  never review-ready because it compiles.
- Schema/durable-event diffs additionally require the human-approval marker
  (Appendix A) per AGENTS.md §Durable state.

### 10.1 Governance changes carry their transition

Merge-time law is authoritative: a rule in force when a PR merges applies to
that PR, whether or not it existed when the work started. In-flight work is
not re-implemented — it satisfies the new law's requirements for the
remaining merge path (for example, adding a second review).

Every PR that changes AGENTS.md, this document, or `docs/GOVERNANCE.md`
carries:

```text
## Transition

Effective:        <on merge>
Affected active plans:
  - <plan> : no change | reclassify | add review | add human gate |
             stop before boundary | grandfather
```

Active plans are enumerable from their frontmatter, so the law's author
identifies the impact rather than every in-flight agent re-reading the law
each day. Work that already crossed a newly gated boundary must be ruled on
explicitly — grandfathered by name with a reason and human acceptance, or
stopped and reworked before merge. A task never grandfathers itself. Merged
history is not retroactively non-compliant unless the new rule states a
remediation.

## 11. Docs and decision lifecycle

- ADRs: `docs/adr/NNN-<slug>.md` with frontmatter `status:
  proposed|accepted|superseded-by-NNN` and `read_when:` triggers. ADRs are
  never edited into a different decision — supersede them.
- Decision records: `docs/decisions/YYYY-MM-DD-<topic>.md` — immutable
  founder acceptance evidence. ADRs materialize them by reference; neither
  copies the other.
- Plans: `docs/plans/YYYY-MM-DD-<topic>.md`; move to `docs/plans/done/` when
  the work lands.
- Benchmarks ledger: `docs/references/architecture-benchmarks.md` — one row
  per settled subsystem decision; new reference research merges into it in the
  same PR that uses it; consultation duty per AGENTS.md §Scope discipline.
- Glossary: `ARCHITECTURE.md` owns the domain vocabulary (Session, Run,
  Binding, Evidence, AttentionItem, NeedsInputRequest, Endpoint, …) including
  banned synonyms; new domain nouns land in the glossary in the same change.
- Terminology: Corral has a *durable semantic event log*; do not describe it
  as event sourcing.

### 11.1 Documentation class map

Documentation class follows semantic authority, not file type. Materializing
accepted truth is not the same as changing truth.

| Change | Class |
|---|---|
| AGENTS.md / GOVERNANCE / this document — any normative rule added, removed, changed in meaning, or reordered in precedence | C |
| the same files, pure editorial (typo, link, formatting, no semantic change) | A/B, detector-flagged; the reviewer confirms it is editorial |
| glossary entry added, naming a concept the accepted architecture already contains | class of the implementing PR (usually B) |
| glossary entry modified or deleted, wording-only | B + human merge |
| glossary entry whose meaning, boundary, or relations change | C |
| a new glossary entry that introduces a new architecture concept | C, even when the diff is add-only |
| ARCHITECTURE / PRODUCT prose synchronizing an accepted decision | B |
| ARCHITECTURE / PRODUCT prose creating or changing an invariant or scope contract | C |
| plans | class of the task |
| new ADR proposal | C |
| ADR materializing an already-accepted decision, citing its evidence | B |
| ADR whose decision changes | C |
| ADR evidence, links, implementation status | B |
| references / benchmark ledger: new evidence, versions, sources | B |
| references whose new evidence overturns or reopens a settled decision | C decision path |
| README, comments, ordinary docs | A/B by scope |

The detector is a conservative guardrail, not a semantic classifier: it flags
modified or deleted glossary entries and canonical-law files. Add-only
entries are not flagged, so the agent and the fresh reviewer must still ask
whether the entry names an existing concept or creates a new one, failing
closed to C when unsure. No hidden canon: an ordinary README or comment never
establishes a rule future agents are expected to follow.

### 11.2 Drift

Two layers.

**PR-local.** Any PR that changes normative behavior answers: which canonical
documents, ADRs, and tests own this behavior, and are they still consistent?
Canon is normally updated by the same PR that changes the owned behavior —
deferring documentation to a later reconciliation task is the drift source
itself.

**Scheduled reconciliation scan.** Weekly during M1 and before each
phase/milestone boundary, plus manually after a cluster of Class C decisions
or a major landing. It targets ADRs accepted since the last pass, changed
owner boundaries, changed public/durable/protocol contracts, Class C
decisions, existing `DOCUMENTATION_DEBT`, and canonical documents last
touched before those decisions. Its output is a drift report or PR — never a
silent automatic canon rewrite.

Severity:

- **Canonical P1 drift** — documentation and accepted or current truth give
  contradictory instructions about the same invariant, enough to make a
  future agent implement the wrong behavior. This creates
  `DOCUMENTATION_DEBT`: the affected owner loses autonomous Class B merge
  until reconciled. Existing code is not automatically wrong; determine the
  source of authority, reconcile canon, ADR, and code, take a fresh review,
  and clear the debt. If which side is true cannot be determined, it is a
  Class C conflict for human decision.
- **Canonical P2 drift** — omissions, stale examples, unsynchronized
  implementation status. Tracked issue; no freeze.

A reconciliation task is a role, not a privileged office: it claims owners,
plans, opens PRs, and is reviewed like any other work.

## 12. External contributions

External contributors contribute code and evidence. They do not inherit the
repository's autonomous-merge authority and are not expected to operate the
internal governance machinery. Policy text lives in `CONTRIBUTING.md`; the
mechanics are:

- No external PR ever merges autonomously. A human maintainer merges all of
  them.
- Agent review runs first as evidence amplification, at the same staffing as
  internal work (§8.1), then the human review and merge.
- The maintainer, or maintainer-agent triage, assigns the class. Contributors
  answer only surface questions in the PR template — protocol? schema?
  runtime ownership? provider integration? security? architecture? — and a
  contributor's misjudgement is never a governance violation.
- A patch that reaches a Class C boundary is marked
  `BLOCKED — DECISION REQUIRED`. The architectural question is extracted into
  an issue or ADR and decided by a human; the PR then continues, is modified,
  is split, or is rejected. Existing contributor effort never lowers the
  decision bar.
- External changes to AGENTS.md, `docs/GOVERNANCE.md`, or this document are
  issue-first, and require explicit maintainer acceptance plus human merge.

## 13. Emergency path

There is **no emergency bypass during M1 / dogfood.** Release-deadline
pressure, preserving a dogfood streak or metric, maintainer inconvenience,
slow CI, a small fix, and a known fix are explicitly not emergencies. A
metric never justifies lowering an engineering gate.

The path below is legislated in advance and stays dormant until Corral has
external users, so that it is never drafted during an incident.

Valid triggers:

```text
external users are materially unable to use Corral
active data-loss or corruption risk
active security / trust-boundary incident
widespread wrong-target control or dangerous behavior
an upstream/provider change causing production-wide unavailability
```

Only a human maintainer declares an emergency; an agent never self-declares.

Under an emergency, obligations move from before-merge to immediately
after-merge — they are never removed:

```text
machine risk gates are never bypassed
a regression test exists when the failure is reproducible
verify runs at least the mandatory subset relevant to the incident
at least one fresh-context review precedes merge
the second review and full audit may be deferred
deferred obligations close within 24 hours or by the next maintainer
  working day
the merge creates audit debt; the owner loses autonomous merge until cleared
```

An emergency Class C decision needs immutable pre-merge evidence in the PR
body:

```text
## Emergency Class C Decision
Decision:
Reason:
Alternatives rejected:
Compatibility / migration consequence:
Approved by:
Timestamp:
```

The ADR materializes within 24 hours or by the next maintainer working day —
later only while the incident is still active — citing the PR, the decision
block, and the root cause. A missed ADR deadline is a governance P1 and puts
the owner in `DOCUMENTATION_DEBT`: no Class B autonomous merge in that owner,
no new Class C merge in that decision area except another active emergency,
cleared only when the ADR lands and a fresh reviewer confirms code and
documentation agree.

Durable-truth law holds during emergencies: migrate, introduce a new
representation, or obtain approval for a destructive reset — never a silent
reinterpretation of persisted facts.

## 14. Rule growth

AGENTS.md §Rule growth is canonical: automate first; prose law only for what
automation cannot own; add rules only after an observed failure with durable
cost. When a rule is added, pair it with a regression test, a lint/check, a
scoped `AGENTS.md`, or an ADR.

## 15. PR0 bootstrap checklist

```text
Files
  AGENTS.md (canonical law)              CLAUDE.md = "@AGENTS.md"
  PRODUCT.md / ARCHITECTURE.md (+ Glossary seed) / ROADMAP.md
                                         derived from the Development Plan
                                         and the decision records
  CONTRIBUTING.md                        external policy incl. AI-PR rules
  README.md                              orientation only; no canon
  LICENSE-APACHE + LICENSE-MIT
  docs/ENGINEERING_WORKFLOW.md           this file
  docs/GOVERNANCE.md                     document hierarchy
  docs/adr/  (template with status + read_when; ADR 5 and 6 accepted)
  docs/decisions/                        founder acceptance evidence
  docs/plans/  + docs/plans/done/
  docs/references/                       ledger + reports
  docs/history/                          retired source documents
  STORAGE_EPOCH                          dev

Enforcement (Appendix A implemented, except rows marked "not yet
implemented" there)
  scripts/verify-fast, scripts/verify, scripts/verify-release
  scripts/flake-probe
  workspace [workspace.lints]: clippy deny set incl. unwrap_used,
    expect_used, await_holding_lock, undocumented_unsafe_blocks
  clippy.toml: allow-unwrap-in-tests; disallowed-methods seeded
  deny.toml (cargo-deny); Cargo.lock committed; pinned rust-toolchain
  dependency-direction check script
  risk-surface detector
  CI: workflows calling the repository scripts — per-PR verify + declared
    PR checks, plus scheduled evidence amplification (§7)
  PR template with Class/Evidence/Compatibility sections
  commit lint; LOC advisory job; schema-gate guard (scripts/check-schema-gate)
  branch protection on main: PRs only, required checks, no force-push

Validation
  one representative class-A task and one class-B task completed by a
  coding agent using only these documents — no undocumented tribal
  knowledge required
```

## Appendix A. Verification map (rule → enforcement)

Build/test truth lives in `scripts/verify*`. PR-metadata checks are the only
permitted CI additions and are closed-listed here.

| Rule | Mechanism | Owner |
|---|---|---|
| formatting | `cargo fmt --check` | verify-fast |
| no unwrap/expect outside tests; no await-holding-lock | workspace clippy deny set | verify-fast |
| unsafe policy (`forbid` default; SAFETY comments) | `forbid(unsafe_code)` + `clippy::undocumented_unsafe_blocks` | verify-fast |
| ownership boundaries (only state module opens DB; only runtime spawns PTYs; grows with owners) | `clippy.toml disallowed-methods` | verify-fast |
| dependency direction (surfaces ↛ corral-core) | cargo-metadata check script | verify |
| dependency hygiene (advisories/licenses/dupes) | cargo-deny | verify |
| protocol additive evolution | future-input fixture tests | verify |
| full test truth | workspace test suite | verify |
| release breadth (provider matrix, packaging, migrations, no release-critical quarantine) | verify-release | release |
| conventional commits | commit lint | CI PR check |
| change-size thresholds | LOC advisory comment (production vs test split; flags >500 complex / >800 total) | CI PR check (advisory) |
| schema/durable-event human gate | diff guard (`scripts/check-schema-gate`) on schema/migration/durable-event paths requiring the `DURABLE-APPROVED-BY:` marker in the PR body | CI PR check |
| risk-surface human gate (§8.2 list) | risk-surface detector setting `HUMAN_REVIEW_REQUIRED`; fails closed | CI PR check |
| quarantine is human-approved and leased | quarantine registry check (owner, deadline, release-critical flag) | CI PR check |
| flake evidence | `scripts/flake-probe` records | diagnostic tooling |
| stale owner claims | claim staleness check over active plan frontmatter | scheduled (not yet implemented — lands with the first concurrent-claim usage) |
| canonical drift | reconciliation scan | scheduled (not yet implemented — manual scan until then, §11.2) |
| audit debt / documentation debt fences | debt tracking over merged PRs and owners | scheduled + review (tracking not yet implemented — review carries it) |
| verification budget | rolling p95 measurement | scheduled (not yet implemented — measure when verify has real cost) |
| verification evidence present | PR template + review | review |
| integration-test MUST for behavior changes | review checklist | review |
| glossary conformance | review checklist | review |
| evidence-map for B/C review | review protocol §9 | review |
| canonical-prose checklist | review protocol §9.1 | review |

Crate-vocabulary boundary for the dependency-direction check:
`corral-protocol` owns wire vocabulary, protocol schemas, and
compatibility-facing representations; `corral-core` owns domain semantics
and invariants. Surfaces depend on `corral-protocol`, never on
`corral-core` — a type appearing on the wire does not move its business
semantics into the protocol crate.

Rows marked "review" are the honest non-mechanical remainder; if one recurs
as a failure, promote it to automation per §14.
