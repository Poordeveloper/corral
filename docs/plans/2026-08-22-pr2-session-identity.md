---
status: blocked   # unblocked when ADR 0002's open questions are ruled
class: C
writes: [adr, corral-core, corral-state, corral-protocol, corrald]
reads: [docs/adr/0002-resume-lineage.md, docs/references/architecture-benchmarks.md]
---

# PR2 — Session identity, bindings, and the durable event log

## Goal

A Session that outlives the processes running it: Corral-minted identity,
bindings with assurance, and a durable event log with idempotent command
receipts — the substrate PR3 and PR4 attach to (ROADMAP §3).

## Non-goals

No PTY (PR3), TUI (PR4), providers or hooks (PR5+), attention (PR8),
history index (M2), remote. **No ghost wire surface**: a field reaches the
wire when a surface renders it, not before.

## Existing owner / architecture involved

Identity, bindings, assurance and the store shape are accepted
(`ARCHITECTURE.md` §1, §5; ledger rows 43, 44); their resume mechanics are
ADR 0002, drafted and not accepted. `corral-state` is new so the store has
one owner rather than growing inside corrald.

## Design

1. ADR 0002 accepted, open questions ruled, in its own change first.
2. **corral-core**: `CorralSessionId`, `RunId`, `Session`, `Run`,
   `SessionBinding`, `Assurance`, `Evidence`. No IO.
3. **corral-state** (new): SQLite; `sessions`/`bindings` as projections;
   `session_events` with per-session monotonic seq; `command_receipts`. A
   projection and its event commit in one transaction; migrate from empty.
4. **Binding resolution**: uniqueness on `(node, provider, external_id,
   kind)`; idempotent discovery; process-only records superseded once
   provider identity is learned.
5. **Commands**: client-supplied ids, exact reuse returns the same
   receipt — the first mutating RPC lands with these or not at all (S5(e)).
6. **corrald** owns one `corral-state` handle, serving only what a caller
   already uses.

## Interfaces or persistence changed

First durable storage. `STORAGE_EPOCH` stays `dev`, so databases stay
disposable, but schema and event diffs need human merge plus
`DURABLE-APPROVED-BY:` from the first write.

## Failure / unknown states

Store unavailable or corrupt ⇒ corrald fails closed rather than serve a
list it cannot vouch for. A Run whose exit cannot be established ends
unverifiable. Unresolvable lineage records no edge. Assurance is
re-evaluated when evidence changes.

## Tests

Invariant/scenario: identity survives process death; re-discovery never
duplicates a Session; one control-capable runtime binding at most;
heuristic assurance never enables control; NativeResume opens a Run under
the same Session; ContextHandoff opens a new Session with an edge; lineage
absent rather than guessed. Storage: seq monotonic per session; projection
and event commit atomically; a receipt replays rather than re-executes; a
crash between them leaves neither; migration from empty on a real file.

## Definition of done

- ADR 0002 accepted and merged before any implementation commit.
- Design 2–6 landed; `./scripts/verify` green on the final tree.
- Two independent fresh-context reviews **before** merge, with
  deliberately different briefs — one contract conformance, one
  adversarial — and their fix reviewed too. PR1 shipped a hang and a
  fabricated runtime fact by reviewing after merge.
- `DURABLE-APPROVED-BY:` present for the schema and event diff.
- Plan moves to done/ on land.
