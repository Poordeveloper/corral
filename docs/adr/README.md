# Architecture Decision Records

One decision per file, `NNNN-<slug>.md`, numbered sequentially. An ADR is
never edited into a different decision — supersede it and mark the old one
`superseded-by-NNNN`.

Write an ADR when all three hold: the decision is hard to reverse, a future
reader would otherwise wonder why the code looks this way, and there were
genuine alternatives. Otherwise record the choice where it belongs — the
benchmark ledger, a plan, or a code comment — and move on.

An ADR that materializes an already-accepted founder decision cites its
record in `docs/decisions/` as acceptance evidence; it does not re-open the
decision. Which changes require an ADR at all is the canonical list in
`AGENTS.md` §Architectural changes.

Frontmatter:

```yaml
---
status: proposed | accepted | superseded-by-NNNN
read_when:
  - <the situation that should make an agent read this>
---
```

Numbers 0001–0004 are reserved for the decisions `ROADMAP.md` §3 schedules
alongside the implementation sequence: corrald activation, resume lineage,
terminal snapshot format, and hook delivery. The phase each belongs to is
the roadmap's to state, not this file's — resequencing there must not have
to edit here.
