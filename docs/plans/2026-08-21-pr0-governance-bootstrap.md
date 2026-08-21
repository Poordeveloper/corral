---
status: active
class: C  # contains AGENTS/GOVERNANCE/Workflow normative surfaces; human merge
writes:
  - governance-docs        # AGENTS.md, docs/GOVERNANCE.md, docs/ENGINEERING_WORKFLOW.md
  - canonical-docs         # PRODUCT.md, ARCHITECTURE.md, ROADMAP.md, CONTRIBUTING.md
  - adr
  - verification-tooling   # scripts/, CI, workspace scaffold
reads:
  - docs/decisions/
  - Corral_Development_Plan_v2.0_EN.md
  - docs/references/
---

# PR0 — Repository governance bootstrap

Authority sources (this plan adds no decisions): Dev Plan v2.0 §15–16;
Engineering Workflow v2 §12 bootstrap checklist + Appendix A; the three
decision records in `docs/decisions/` (strategy grill, UX contract,
workflow-governance grill — its §7 is the governance-edit checklist);
`docs/GOVERNANCE.md`.

## Goal

Turn the frozen paper canon into an operating repository: git + canonical
documents + seed ADRs + verification/enforcement machinery, so PR1 executes
under the full workflow with no undocumented tribal knowledge.

## Non-goals

- No product/domain code beyond an empty workspace scaffold (PR1 owns the
  corrald walking skeleton).
- No spike execution (S1/S2/S3 run on their own schedule).
- No M2+ emergency tooling, plugin seams, packaging, Tray, or probe work.
- No new decisions: every normative sentence must trace to an accepted
  source; anything untraceable is a named Class C rider for explicit
  founder ack — never silent synthesis.

## Existing owner / architecture involved

Bootstrap creates the owners. Document hierarchy per `docs/GOVERNANCE.md`;
crate vocabulary boundary per Workflow Appendix A (`corral-protocol` wire
vocabulary vs `corral-core` domain semantics).

## Design

1. **Git bootstrap**: `git init`; baseline commit = today's tree verbatim,
   including `.claude`/`.agents`/`skills-lock.json` — `.claude/skills/*`
   stay relative symlinks into `.agents/skills/`, committed as symlinks,
   never dereferenced into copies (the only direct-to-main commit ever,
   pre-protection, so the PR0 diff is reviewable against reality);
   `.gitignore` (target/, OS noise, worktree dirs); commit identity is
   repository-scoped `Poordeveloper <catchballoon@gmail.com>` (the machine
   global identity is a different account and is left untouched); private
   GitHub repo `corral` — owner account pending founder `gh` auth; dual license
   `LICENSE-APACHE` + `LICENSE-MIT`; minimal README (name, one-line
   positioning, canon pointers, pre-release status; no normative content);
   branch `task/pr0-governance`; everything else lands via the PR.
2. **Canonical law (Class C riders; ack channel = in-conversation,
   per-edit numbered old→new blocks, order AGENTS → Workflow → GOVERNANCE;
   acked text is frozen and the PR only re-enacts it)**: AGENTS.md — the
   eight edits frozen in workflow-governance record §7, plus two
   architecture-v1 one-liners (PTYs owned by corrald only;
   heuristic-assurance evidence renders but never notifies); Workflow v2 →
   `docs/ENGINEERING_WORKFLOW.md` with its eleven §7 edits + Appendix A
   additions; `docs/GOVERNANCE.md` — add `docs/decisions/` beside
   `docs/adr/` (records = immutable acceptance evidence; ADRs materialize
   them), flip the bootstrap paragraph to historical.
3. **Canonical derivation (Class B materialization; every section cites
   its source — v2.0 § or decision-record §)**:
   - `PRODUCT.md`: product invariant + loop; capability ladder + the four
     UI verbs; five-state user model; honest M1 capability posture;
     non-goals; terminology law; provider support guarantee.
   - `ARCHITECTURE.md`: session/binding/assurance model; corrald/client
     boundary; evidence + freshness-qualified authority; protocol planes +
     compatibility posture; two stores + three clocks + storage epoch;
     hook-shim boundary (fail-open + 15 s lease); platform boundary;
     crate layout + vocabulary boundary; internal extension seams; the
     architecture-v1 invariants not carried by AGENTS.md (discovery
     idempotence via binding uniqueness; single active control-capable
     binding; PTY byte streams on a dedicated framed channel; provider
     files read-only outside named reversible operations; provider data
     untrusted — degrade, never panic); **Glossary seed** (v2.0
     vocabulary + UX-contract user-visible vs internal terms + banned
     words).
   - `ROADMAP.md`: thesis hierarchy (delay/kill/block); PR0–PR8 ladder;
     ADR schedule (0001–0004 reserved to PR1–PR4); spikes S1, S2
     (extended: safe-merge corpus), S3 (live-join census); release gate;
     kill criteria + rung-2 validity floor; dogfood-epoch plan; post-PR8
     completion list.
   - Move `Corral_Development_Plan_v2.0_EN.md` → `docs/history/`
     (historical source per GOVERNANCE).
4. **ADRs**: template + numbering README (0001–0004 reserved per
   schedule); `0005-platform-scope` (Windows deferral + re-entry trigger;
   host-OS execution domain, containers/VM/WSL2/SSH = future nodes),
   accepted; `0006-provider-hook-integration-policy` (default-install,
   disclosure, per-provider disable, fail-safe merge, Corral-owned-only
   uninstall), accepted. Both cite acceptance evidence; no re-approval.
5. **CONTRIBUTING.md**: base text fetched at implementation time from
   upstream `farion1231/cc-switch` (AI-contribution policy; provenance in
   the file header) + the governance-record external-contribution flow as
   skeleton (maintainer classifies; external PRs never autonomous-merge;
   `BLOCKED — DECISION REQUIRED` path).
6. **Workspace scaffold**: `crates/corral-core`, `crates/corral-protocol`
   as empty `forbid(unsafe_code)` lib shells (accepted names; keeps every
   gate non-vacuous); `[workspace.lints]` deny set (unwrap_used,
   expect_used, await_holding_lock, undocumented_unsafe_blocks);
   `clippy.toml` (allow-unwrap-in-tests; disallowed-methods seeded);
   `deny.toml`; committed `Cargo.lock`; `rust-toolchain.toml` pinned to
   stable 1.95.0; edition 2024.
7. **Verification & enforcement**: `scripts/verify-fast` + `scripts/verify`
   (real, green); `scripts/verify-release` (superset stub with contract
   header); `scripts/flake-probe` (minimal N-run recorder);
   dependency-direction check (cargo-metadata); risk-surface detector
   (path-category diff guard → `HUMAN_REVIEW_REQUIRED`, fail-closed,
   categories per governance record); `STORAGE_EPOCH` file = `dev` at repo
   root; CI: per-PR = `./scripts/verify` on ubuntu + the closed metadata
   checks (commit lint, LOC advisory, risk detector); macOS runs
   post-merge + scheduled and is required in `verify-release` (private
   repo bills macOS runners at 10×; local development already verifies on
   macOS); one PR template — internal fields (`Class/Reason/Triggers` +
   Evidence + Compatibility + Risk/staging) plus the external-contributor
   surface questions.
8. **Post-merge (founder console)**: branch protection on main — PRs
   only, required status checks (per-PR CI), force-push and deletion
   disabled, required approvals = 0 (solo repo: the human-merge law is
   enforced by who presses merge, not the GitHub approval bit; no bot
   account — agent commits carry the founder identity plus
   `Co-Authored-By` trailers); then dispatch the two Workflow §12
   validation tasks (one Class A, one Class B, chosen organically from
   PR0 follow-ups, agent-executed using only the repository documents).

## Interfaces or persistence changed

Nothing executable. Compatibility-relevant surfaces created: verify script
names/semantics; the closed PR-metadata check set; `STORAGE_EPOCH` marker;
ADR numbering. All are on the breaking-surface checklist from birth.

## Failure / unknown states

- Concurrent reconciliation-agent edits: this plan claims all doc owners —
  the reconciliation agent is paused for the PR0 window (founder directive
  2026-08-21); its un-landed output is absorbed during canonical
  derivation.
- Derivation drift: synthesis may strengthen/weaken a frozen sentence →
  citation discipline; founder reviews AGENTS/Workflow/GOVERNANCE diffs
  line by line (the canonical docs, not the scripts, are the review
  surface).
- Vacuous green: crate shells keep fmt/clippy/test/dep-direction real;
  detector demonstrated on a synthetic flagged diff in Evidence.
- Founder inputs (settled in the 2026-08-21 plan grill): private GitHub
  repo `corral` under the active `gh` account (`rustdesk`) unless
  redirected before creation; LICENSE = Apache-2.0 OR MIT dual;
  `.claude`/`.agents` committed. Review staffing: single PR, two
  independent fresh-context reviews, founder final review/merge.

## Tests

Scripts are the artifact. Evidence must show: verify-fast + verify green
on a clean clone; detector positive + negative demonstration; commit lint
and LOC advisory exercised on the PR itself. Automated self-tests for
tooling are follow-ups per the rule-growth law, not PR0 gold-plating.

## Definition of done

- Workflow §12 checklist and workflow-governance record §7 checklist fully
  applied; every Class C rider acked in-conversation pre-PR (per-edit,
  numbered) and re-enacted verbatim in the PR.
- CI green; founder merge; branch protection enabled.
- Both validation tasks completed post-merge; gaps they surface become
  follow-up issues before PR1 starts.
- v2.0 in `docs/history/`; this plan in `docs/plans/done/`.
