# Founder Decision Record — Plan size: budget, not cap

> Status: founder-accepted, 2026-08-22. Materialized by the
> `docs/ENGINEERING_WORKFLOW.md` §2.4 edit landing with this record.
> Process governance only: no ADR, no architectural invariant, no code.
> Deliberately separate from the ADR 0002 acceptance change set.

## The problem

§2.4 declared *"Hard cap: one page / ~60 lines."* No plan has ever
satisfied it: PR0's plan is 197 lines, PR1's is 156, PR2's is 77 —
including the plan written in the same bootstrap batch as the rule
itself. A hard rule nothing has ever met trains every agent to ignore the
word "hard", and the eight-section template the same section mandates
does not fit in 60 lines for any non-trivial phase.

## The decision

- The 60-line hard cap is **deleted**, and not replaced by another
  arbitrary hard cap.
- **Size target: ~150 lines.** Exceeding it is not a governance
  violation.
- A plan over the target must carry a **`Plan Size Justification`**
  section: why it remains one coherent semantic scope, and why splitting
  would make implementation or review worse.
- The **fresh reviewer** rules on the justification: coherent → the plan
  is accepted over budget; scope too broad → split.
- The target is a **review-pressure threshold, never a line-count CI
  gate**. No script enforces it.
- The eight-section template (Goal / Non-goals / Owners / Design /
  Interfaces / Failure–unknown states / Tests / DoD) is retained.
- Core rule, now stated in §2.4: the plan must be as short as possible
  while remaining executable without oral history; length is a review
  signal, not a correctness invariant.

Rejected: raising the cap to a 150-line hard cap (swaps one failed
absolute for another; the failure mode was the absolutism, not the
number); shrinking the template to fit 60 lines (the eight sections are
themselves governance requirements); pure guidance with no threshold
(PR0's 197 lines shows unbounded plans grow — review pressure needs a
stated trigger).

## Transition

- **PR0 plan** (197 lines) and **PR1 plan** (156 lines), both in
  `docs/plans/done/`, are historical records and are not retro-edited.
  Under the new law each would have required a Plan Size Justification;
  their length is hereby accepted as-is.
- **PR2 plan** (77 lines at ruling time) is within the target. The ADR
  0002 acceptance change set will grow it; it stays governed by the
  budget-plus-justification rule like every future plan.
- From this record on, a fresh reviewer citing plan size cites this
  decision and §2.4, not the retired cap.
