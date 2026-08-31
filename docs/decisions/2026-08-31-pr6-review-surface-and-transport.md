# PR6 review — founder rulings on surface and transport

> Acceptance evidence for ADR 0010, from the founder's review of PR6 at
> `b0cf0cd`. Two of the three findings in the first round were repaired in
> code; this record holds the rulings that are decisions rather than repairs.
> The rulings are recorded in substance; the directive lines are verbatim in
> meaning.

## R1 — The transport ceiling is not a follow-up, and ADR 0009 is not edited

The first round proposed leaving ADR 0009 D2's uniform "256 KiB cap with the
oversize marker" as a follow-up, on the reading that D2 describes what the
relay does with what it receives. Overruled:

> Do not leave D2 as a follow-up, and do not quietly change ADR 0009. Record a
> superseding decision inside PR6.

D2 promises the marker as a property of argv delivery, and a supported Linux
target can fail with `E2BIG` before the relay process exists — so it is not a
promise a relay can keep. The repository's own rule is that an accepted ADR is
never edited into a different decision; it is superseded.

The new decision must say at least two things: that argv transport has a
provider- and OS-owned ceiling *before* the relay, so the oversize marker only
guarantees the Corral-owned limit once the relay actually has the payload; and
that argument refusal now has two legitimate reasons — defeating the
injection, or entering a provider surface Corral has explicitly declared
unmanaged.

## R2 — Refusing every subcommand is the right direction, and it is a decision

The implementation refuses every Codex subcommand rather than only `resume`
and `fork`. Endorsed, and not narrowed:

> I agree with your implementation over falling back to refusing only
> `resume`/`fork`: ADR D1 already says the interactive TUI is the whole managed
> surface. But it has to be promoted from "the code happens to be stricter now"
> into an explicit decision.

The two architectural items may be merged into a single superseding decision
rather than split across two ADRs.

## R3 — Claude's identical exposure stays the next task

`corral new claude -- --resume <id>` reaches Claude Code's own resume without
the per-Session continuation claim, for the same reason PR6's did. Left out of
PR6 deliberately:

> Leave the Claude native-resume bypass as the immediately following task; I do
> not want PR6 widening Claude's user-visible behaviour as a side effect.

## What the review did not reopen

The identity-confirmation generalization, the seam reshape, the notify override
and its refusals, and the capability substitution of ADR 0009 D4 were reviewed
and left standing. The repaired asynchronous assertion in the Claude
continuation test was accepted as a test fix, not a behaviour change.
