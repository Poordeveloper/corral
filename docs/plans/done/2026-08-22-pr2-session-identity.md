---
status: done
class: C
writes: [corral-core, corral-state, corrald, corral-rendezvous, canonical-docs]
reads: [docs/adr/0002-resume-lineage.md, docs/decisions/2026-08-22-pr2-resume-lineage-acceptance.md]
---

# PR2 — Session identity, bindings, and the durable event log

## Goal

A Session that outlives the processes running it: Corral-minted identity,
bindings with assurance, and a durable event log with idempotent command
receipts — the substrate PR3 and PR4 attach to (ROADMAP §3).

## Non-goals

No PTY (PR3), TUI (PR4), providers or hooks (PR5+), discovery (PR7),
attention behaviour (PR8), history index (M2), remote.

**Zero wire change.** No RPC, no `session.list` fields, no session wire
shape, no mutating method, no stream/event vocabulary. `session.list`
keeps returning what it can truthfully provide; if that is `[]`, it stays
`[]`. `corral-protocol` is not in `writes:` — nothing is pre-staged for
PR3 (acceptance record Q4).

Also out of scope, each awaiting the phase with a real producer *and*
consumer: lineage proposals and manual-confirmation flows (Q5); binding
supersession and assurance-change persistence (Q15); correction,
archive/delete events; degraded-store UX and its vocabulary (Q14);
cross-node receipts (Q13).

## Existing owner / architecture involved

Identity, bindings, assurance and the store shape are accepted
(`ARCHITECTURE.md` §1, §5; ledger rows 43, 44). Resume mechanics are ADR
0002, accepted 2026-08-22. `corral-state` is new so the store has one
owner rather than growing inside corrald.

## Design

1. **corral-core**: `CorralSessionId`, `RunId`, `Session`, `Run`,
   `SessionBinding`, `Assurance`, `Evidence`, plus the
   `NeedsInputRequest` / `AttentionItem` actionable-status vocabulary per
   `ARCHITECTURE.md` §2 (domain meaning only — no wire, no persistence,
   no notification, scoring, or provider mapping; Q6). No IO.
2. **Run and association**: a Run is a concrete runtime occurrence with a
   Corral-minted `RunId`, minted only from constructive evidence (Corral
   created the runtime) or authoritative node-local runtime observation.
   Semantic evidence alone never mints one. Association assurance lives on
   the `RuntimeBinding`, never on the Run; a Run's `session_id` is a
   structural reference whose trust comes from that binding. No
   Session-less Run (Q8, Q9).
3. **corral-state** (new): SQLite; `sessions`/`bindings` as projections;
   `session_events` with per-session monotonic seq; `command_receipts`. A
   projection and its event commit in one transaction; migrate from empty.
   Every persistent projection mutation is derivable from an accepted
   event (Q15).
4. **Binding resolution**: uniqueness on `(node, provider, external_id,
   kind)`; idempotent re-discovery resolves to the existing Session.
   Supersession and assurance-change *persistence* are deferred with their
   producers; live re-evaluation is unrestricted.
5. **Durable writes follow assurance**: a heuristically bound Run exists in
   live state but emits no durable `RunStarted`/`RunEnded` under that
   Session. Facts are appended when assurance is established — never
   inserted into an earlier seq. Occurrence time is recorded only when
   independently supported by authoritative runtime evidence;
   `first_observed_at` is never written as `started_at` (Q10).
6. **Commands**: client-supplied ids unique in the node's durable command
   namespace (`command_id` as primary key), stable across Sessions,
   clients, connections, and daemon restarts. Same id + same fingerprint
   returns the original receipt without re-executing; different
   fingerprint is `CommandIdConflict` — a domain error in PR2, executing
   nothing and leaving the receipt untouched. The fingerprint covers
   command kind and semantic inputs only (Q12, Q13).
7. **corrald** owns one `corral-state` handle, opened and validated before
   the daemon advertises serving readiness.

## Interfaces or persistence changed

First durable storage; no wire surface. The registry lives at
`<corral root>/state/registry.sqlite3`: the Corral root's layout has one
owner, so `corral-rendezvous` derives that path as it does the run and log
trees, and `ARCHITECTURE.md` §5 gains the `runs` and `session_lineage`
projections the accepted event set implies. `STORAGE_EPOCH` stays `dev`, so
databases stay disposable, but schema and event diffs need human merge
plus `DURABLE-APPROVED-BY:` from the first write. Durable events used:
`SessionCreated`, `BindingAdded`, `BindingConfirmed`, `RunStarted`,
`RunEnded`, `RunAttached`, `RunDetached`, `SessionForkedFrom`,
`CommandAccepted` — the last three founder-accepted in this change set.

## Failure / unknown states

**Fail closed applies both before readiness and after an unrecoverable
runtime state failure; PR2 never continues to serve state-derived claims
from an untrusted store.** At startup — DB unopenable, schema
uninitializable, integrity check failed, state directory unusable — the
daemon never reaches protocol-ready: no successful hello, exit as an
internal state startup failure, client resolves through PR1's activation
failure path. After readiness, an unrecoverable state failure moves the
daemon to fatal shutdown; established callers see connection loss. No
degraded alive-but-stateless mode (Q14).

A Run whose exit cannot be established ends unverifiable. Unresolvable
lineage records no edge. Heuristic association never enables control and
never produces a durable lifecycle fact.

## Tests

Invariant/scenario: identity survives process death; re-discovery never
duplicates a Session; at most one control-capable runtime binding;
**Run existence never implies control eligibility**; a heuristically bound
Run exists in live state yet writes no durable lifecycle fact; NativeResume
opens a Run under the same Session; ContextHandoff opens a new Session with
an edge; observed-fork similarity records no `SessionForkedFrom`.

Storage: seq monotonic per session; projection and event commit atomically;
a crash between them leaves neither; migration from empty on a real file;
**replaying the event log into empty projections reproduces the persistent
state**, covering every durable transition PR2 can produce.

Receipts: first execution mutates once and stores a receipt; same id + same
semantic command returns the same receipt with no second mutation; same id
+ different semantic command conflicts, leaving the receipt unchanged and
mutating nothing; equivalent representations of one command do not conflict
on encoding differences alone.

Fail-closed: unopenable store → no readiness; corrupt store → no readiness;
injected fatal state failure after readiness → the daemon stops serving
rather than returning trusted-looking state.

## Definition of done

- Design 1–7 landed; `./scripts/verify` green on the final tree.
- Every listed test present and failing against the pre-fix behaviour where
  practical.
- Two independent fresh-context reviews **before** merge, with
  deliberately different briefs — one contract conformance, one
  adversarial — and their fix reviewed too. PR1 shipped a hang and a
  fabricated runtime fact by reviewing after merge.
- `DURABLE-APPROVED-BY:` present for the schema and event diff.
- No wire diff: `crates/corral-protocol` untouched.
- Plan moves to done/ on land.
