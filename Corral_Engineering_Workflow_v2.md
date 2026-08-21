# Corral Engineering Workflow v2

> Status: approved engineering-process baseline for Corral M0/M1.
> Date: 2026-08-21
> Supersedes v1. Rules live in `AGENTS.md` (canonical law); this document is
> process mechanics only and never duplicates rule text. Document hierarchy:
> `docs/GOVERNANCE.md`. At PR0 this file becomes `docs/ENGINEERING_WORKFLOW.md`.

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

| Class | Examples | Required ceremony |
|---|---|---|
| A — Bounded | clear bug with known owner; parser edge case; small flag; localized UI fix | read → inspect flow → reproduce/identify contract → smallest coherent fix → focused tests → verify → review diff |
| B — Feature | new provider integration; Tray menu; discovery source; new control operation | plan in `docs/plans/` **accepted by a human before implementation** |
| C — Architectural | anything on the canonical list in AGENTS.md §Architectural changes | ADR accepted → plan → staged implementation → compatibility/migration tests |

Plan template (hard cap: one page / ~60 lines):

```text
Goal
Non-goals
Existing owner / architecture involved
Design
Interfaces or persistence changed
Failure / unknown states
Tests
Definition of done
```

Plan acceptance may be as light as the founder committing the plan file or one
approving comment — but implementation does not start before it exists.

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

## 4. Worktree and multi-agent protocol

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

## 5. Implementation discipline

Rules: AGENTS.md §Scope discipline, §Existing concepts, §Comments, §Rust.

Numbers (review pressure, not lints):

- Modules: prefer ≤ ~500 production LoC; if a central file is at ~800, put
  substantial new behavior in a focused module instead. Do not split merely to
  satisfy a number if it destroys cohesion.
- Diffs (non-mechanical): complex logic normally < ~500 changed lines; at
  ~800, an explicit staging check is required in the PR. A larger coherent
  invariant is allowed when splitting would be unsafe and the PR says why.
- Repair the owner, not the symptom: no speculative retries, larger timeouts,
  weaker assertions, test-only behavior, duplicate fallbacks, or consumer-only
  guards when the producer owns the invalid state.

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
- Flaky policy: AGENTS.md §Tests (P1, quarantine with owner + deadline, no
  CI retry loops).

## 7. Verification and CI contract

Two entry points, the only definition of code done (AGENTS.md §Verification):

```text
./scripts/verify-fast     iteration gate, target < ~2 min:
                          cargo fmt --check
                          clippy with the workspace deny set
                          focused/inexpensive tests

./scripts/verify          completion gate:
                          verify-fast
                          full workspace tests
                          cargo-deny (advisories / licenses / duplicates)
                          protocol future-input tests
                          dependency-direction check
                          disallowed-methods boundary lints
```

CI = `./scripts/verify` + the declared PR-metadata checks in Appendix A —
nothing else. Adding a CI job outside those two categories is a violation.

## 8. Review protocol

Rules for review content/output: AGENTS.md §Review. Severities:

```text
P0  catastrophic / security / severe data loss
P1  correctness, state loss, protocol/architecture violation
P2  meaningful bug or maintainability issue worth fixing now
P3  optional improvement; omit unless it materially helps
```

Class scaling:

| Class | Review requirement |
|---|---|
| A | findings-first review of the final diff; fresh context optional |
| B | fresh-context review (separate agent session or human) mandatory |
| C | fresh-context review mandatory + explicit human acknowledgement |

For B/C, the reviewer builds the **evidence map** and states any missing cell
as a gap instead of guessing:

```text
changed surface | entry point | owner boundary | one caller + one callee |
invariant-sharing siblings | existing tests | current main behavior
```

Verify the premise before treating an apparent gap as unfinished work:
`git log -p -S <symbol>` — deleting intentional design is the most common
AI-review failure.

## 9. PR and commit discipline

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

## 10. Docs and decision lifecycle

- ADRs: `docs/adr/NNN-<slug>.md` with frontmatter `status:
  proposed|accepted|superseded-by-NNN` and `read_when:` triggers. ADRs are
  never edited into a different decision — supersede them.
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

## 11. Rule growth

AGENTS.md §Rule growth is canonical: automate first; prose law only for what
automation cannot own; add rules only after an observed failure with durable
cost. When a rule is added, pair it with a regression test, a lint/check, a
scoped `AGENTS.md`, or an ADR.

## 12. PR0 bootstrap checklist

```text
Files
  AGENTS.md (v2, canonical law)          CLAUDE.md = "@AGENTS.md"
  PRODUCT.md / ARCHITECTURE.md (+ Glossary seed) / ROADMAP.md
                                         derived from the Development Plan
  CONTRIBUTING.md                        external policy incl. AI-PR rules
                                         (CC Switch text as base)
  docs/ENGINEERING_WORKFLOW.md           this file, moved
  docs/GOVERNANCE.md                     document hierarchy
  docs/adr/  (template with status + read_when)
  docs/plans/  + docs/plans/done/
  docs/references/                       ledger + reports (exists)

Enforcement (Appendix A implemented)
  scripts/verify-fast, scripts/verify
  workspace [workspace.lints]: clippy deny set incl. unwrap_used,
    expect_used, await_holding_lock, undocumented_unsafe_blocks
  clippy.toml: allow-unwrap-in-tests; disallowed-methods seeded
  deny.toml (cargo-deny); Cargo.lock committed
  dependency-direction check script
  CI: one workflow calling ./scripts/verify + declared PR checks
  PR template with Evidence/Compatibility sections
  commit lint; LOC advisory job; schema-gate guard

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
| conventional commits | commit lint | CI PR check |
| change-size thresholds | LOC advisory comment (production vs test split; flags >500 complex / >800 total) | CI PR check (advisory) |
| schema/durable-event human gate | diff guard on schema/migration/durable-event paths requiring approval marker in PR body | CI PR check |
| verification evidence present | PR template + review | review |
| integration-test MUST for behavior changes | review checklist | review |
| glossary conformance | review checklist | review |
| evidence-map for B/C review | review protocol §8 | review |

Crate-vocabulary boundary for the dependency-direction check:
`corral-protocol` owns wire vocabulary, protocol schemas, and
compatibility-facing representations; `corral-core` owns domain semantics
and invariants. Surfaces depend on `corral-protocol`, never on
`corral-core` — a type appearing on the wire does not move its business
semantics into the protocol crate.

Rows marked "review" are the honest non-mechanical remainder; if one recurs
as a failure, promote it to automation per §11.
