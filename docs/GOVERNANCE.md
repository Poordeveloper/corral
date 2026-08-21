# Corral Governance — document hierarchy

How the engineering operating system fits together, and which document owns
what. If two documents disagree, the higher one in this list wins and the
lower one is fixed.

```text
AGENTS.md                          canonical hard rules (law)
    ↓
docs/ENGINEERING_WORKFLOW.md       process mechanics (how we work);
                                   never duplicates rule text
    ↓
PRODUCT.md                         what Corral is / is not
ARCHITECTURE.md                    boundaries + domain glossary
ROADMAP.md                         what the current phase allows
    ↓
docs/adr/                          irreversible decisions
                                   (status + read_when frontmatter)
docs/plans/                        bounded implementation plans
                                   (done/ after landing)
    ↓
docs/references/                   evidence: architecture-benchmarks.md
                                   (settled-decision ledger) + source reports
    ↓
scripts/verify, scripts/verify-fast   the only definition of code done;
                                   CI = verify + the closed list of
                                   PR-metadata checks (Workflow Appendix A)
```

Ownership rules:

- One owner per topic. A document references other documents; it does not
  copy their text. (The v1 workflow/AGENTS duplication drifted within days —
  the ADR-trigger lists diverged before any code existed.)
- `CLAUDE.md` contains only `@AGENTS.md`.
- The canonical ADR-trigger list lives in AGENTS.md §Architectural changes.
- Settled architecture decisions live as ledger rows; reopening one requires
  reading its row and bringing new evidence (AGENTS.md §Scope discipline).
- Changes to AGENTS.md, this file, or the Workflow require explicit founder
  acknowledgement — governance is class C regardless of diff size.
- During bootstrap, `Corral_Development_Plan_v2.0_EN.md` is the source from
  which PRODUCT/ARCHITECTURE/ROADMAP are derived at PR0; after that, the
  canonical files own their topics and the plan is historical record.
