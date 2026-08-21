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
docs/decisions/                    founder acceptance evidence, immutable;
                                   ADRs materialize them by reference
docs/plans/                        bounded implementation plans
                                   (done/ after landing)
    ↓
docs/references/                   evidence: architecture-benchmarks.md
                                   (settled-decision ledger) + source reports
    ↓
scripts/verify-fast                iteration feedback, never merge evidence
scripts/verify                     the one definition of merge-ready;
                                   CI = verify + the closed list of
                                   PR-metadata checks (Workflow Appendix A)
scripts/verify-release             release gate; strict superset of verify
```

Ownership rules:

- One owner per topic. A document references other documents; it does not
  copy their text. (The v1 workflow/AGENTS duplication drifted within days —
  the ADR-trigger lists diverged before any code existed.)
- `CLAUDE.md` contains only `@AGENTS.md`.
- The canonical ADR-trigger list lives in AGENTS.md §Architectural changes.
- `docs/decisions/` records what the founder accepted and when; it is never
  edited to change a past decision. `docs/adr/` materializes those decisions
  as single-topic, supersedable records. Neither copies the other's text.
- Settled architecture decisions live as ledger rows; reopening one requires
  reading its row and bringing new evidence (AGENTS.md §Scope discipline).
- Changes to AGENTS.md, this file, or the Workflow require explicit founder
  acknowledgement — governance is class C regardless of diff size.
- PRODUCT/ARCHITECTURE/ROADMAP were derived at PR0 from the Development Plan
  and the founder decision records. The plan now lives in `docs/history/` as
  a source, not canon: the canonical files own their topics, and a
  disagreement with the retired plan is resolved in favour of the canonical
  files.
