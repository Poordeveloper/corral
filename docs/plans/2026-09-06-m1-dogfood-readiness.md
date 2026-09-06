---
status: active   # rulings in docs/decisions/2026-09-06-m1-completion-grill.md Q2–Q6, Q15, Q19
class: C         # D4 crosses AGENTS.md §Architectural changes: durable storage semantics requiring migration guarantees; accepted on the grill's Q6, recorded as ADR 0018
writes: [crates/corrald/src/attention, crates/corral-protocol, crates/corral/src, crates/corral-state, scripts/verify-release, docs/adr/0018-migration-baseline.md, ARCHITECTURE.md]
reads: [crates/corral-core, docs/decisions/2026-09-06-m1-completion-grill.md, docs/adr/0015-attention-derivation.md, docs/decisions/2026-09-02-pr8-attention-grill.md]
---

# M1 dogfood readiness — the journal measures the gate, disputes carry a kind, schema 5 becomes a migration baseline

## Status

**Accepted 2026-09-06** by the completion grill; this plan materializes Q2–Q6,
Q15 and Q19 and decides nothing new. It ends where a human can advance
`STORAGE_EPOCH` to `dogfood` and start the attention and tray evidence
windows. Two PRs: A carries D1–D3 and the glossary (no durable surface);
B carries D4 with ADR 0018 and the `verify-release` step, and its body needs
the founder's `DURABLE-APPROVED-BY` marker (schema gate).

## Goal

Before the 14-day window starts, the attention journal must record what the
release gate counts, a dispute must say which kind of observation it is, the
journal must keep the evidence long enough, and the registry store must be a
baseline future builds are obligated to migrate rather than refuse.

## Non-goals

Machine-readable report output for the evidence document (`--json`,
`--until`) and the evidence template — `m1-release-gate`. Packaging, `hello`
pid, notifications, the Desktop tray log (Q20) — their own plans. Any real
migration: the chain is empty at schema 5. Advancing the epoch, deleting the
founder's schema-1 store — human actions the grill records. No change to
what the journal is (ADR 0015 D8 stands) and no item ids in product UI.

## Existing owner / architecture involved

- `corrald::attention::journal` owns the records and the day report;
  `tick.rs` writes transitions, `connection.rs` writes disputes; the day
  facts travel as `AttentionDayFacts` in `corral-protocol::method`.
- `corral` (CLI) owns the `attention report` table and `attention dispute`,
  which already resolves the session's current item before sending.
- `corral-state::schema` owns `SCHEMA_VERSION = 5` and the refusal of every
  other version inside `initialize`; `store.rs` opens it nowhere else.
- `ServerHello.capabilities` is how a client learns what a daemon serves
  before offering an action (`corral-protocol::hello::capability`).
- `scripts/verify-release` lists "migration verification" among the gates
  it does not have.

## Design

### D1 — The journal counts trusted activations (Q2, Q19)

`TransitionRecord::trusted_activation(&self) -> bool` is the one definition:
`to == NeedsYou && born.is_some() && matches!(assurance, Some(Deterministic
| Attested)) && sealed == Some(true)`. The day report gains
`trusted_needs_you`; `into_needs_you` keeps counting every record landing in
Needs You. `AttentionDayFacts` gains `trusted_needs_you: Option<u64>`,
`false_disputes: Option<u64>`, `missed_disputes: Option<u64>` (`serde
default`, omitted when `None`); `disputes` stays the total. The CLI table
prints `needs you (all)`, `trusted`, `ready`, `false`, `missed`, and `-`
where a daemon did not send a column — absence is not zero.

### D2 — A dispute states its kind (Q3, Q15)

`DisputeKind { FalseItem, MissedItem }` in the protocol; wire
`AttentionDisputeParams` gains `kind: Option<DisputeKind>` (absent =
`false_item`) and `note: Option<String>`; the result echoes `kind`. The
journal's dispute record gains `dispute_kind` (the top-level `kind` stays the
record discriminator) and `note`; a line without `dispute_kind` reads as
`false_item`; an unrecognized value makes the day INCOMPLETE, as an
unrecognized `to` does today. Daemon rules: `false_item` without
`attention_item_id` → `InvalidParams`, nothing journaled; `missed_item` is
journaled with `item: None`, `stale: false`. The daemon advertises capability
`attention-dispute-kinds`. CLI: `dispute <session>` resolves the current item
and, finding none, exits non-zero before sending with `No current attention
item. If Corral failed to surface an item, use --missed.`; `--missed` is sent
only to a daemon advertising the capability, otherwise the CLI reports the
daemon as older and records nothing; `--note` travels as given. Notes are
never read by inference.

### D3 — Retention (Q5)

`Budget::default().retention` becomes 90 days. Nothing else about the
journal changes: still diagnostic, still deletable, still not migrated.

### D4 — Schema 5 becomes the migration baseline (Q6; ADR 0018)

`corral-state::schema` gains `DOGFOOD_BASELINE_SCHEMA: u32 = 5` and a
runner: `Migration { from: u32, to: u32, apply: fn(&Transaction) ->
Result<(), StateError> }` with `MIGRATIONS: &[Migration] = &[]`. `initialize`
reads the stored version and decides: equal → open; `baseline ≤ found <
current` → one transaction applies the contiguous chain `found → current`
and writes the version, any error rolls the whole transaction back and the
open fails with `FatalState::MigrationFailed { from, to, detail }` naming that
nothing changed; a gap in the chain → `MigrationUnavailable { from, to }`;
`found > current` → the existing `SchemaVersionMismatch` refusal; `found <
baseline` → `SchemaVersionMismatch` with detail naming the baseline and that
a pre-dogfood store is not migrated. No path creates a fresh store over an
existing one. The runner takes its chain as a parameter so tests inject
chains; production passes `MIGRATIONS`. Fixture:
`crates/corral-state/fixtures/registry-schema-5.sqlite3` and
`registry-schema-5.expected.json`, produced once by
`examples/make_schema_fixture.rs` through the public `Store` API — a node, a
managed session with a run started and ended, a binding confirmed, a lineage
row, integration intent and an authorized repair, a command receipt — and
committed. `verify-release` replaces its "migration verification" line with
the step `cargo test -p corral-state migration_baseline` and keeps exiting 1
for the gates still missing.

### D5 — Records

ADR 0018 *Migration baseline and forward-migration guarantee* (status
accepted, acceptance evidence: the completion grill Q6): schema 5 is the
first protected baseline; forward-only; atomic; refuse newer; never a
destructive fallback; the fixture gate turns non-vacuous at schema 6.
`ARCHITECTURE.md` §5 gains one paragraph pointing at it; §11 gains *trusted
Needs You item activation*, *dispute kind*, and *migration baseline*.

## Interfaces or persistence changed

Wire, additive: `AttentionDayFacts` three optional fields;
`AttentionDisputeParams.kind`, `.note`; `AttentionDisputeResult.kind`;
capability `attention-dispute-kinds`. Journal, additive: `dispute_kind`,
`note`. Registry store: no schema change; the open path gains the migration
decision; a `FatalState` variant per refusal. Diagnostic files only
otherwise.

## Failure / unknown states

Journal absent → empty report, unchanged. Old daemon, new CLI: the three
columns print `-`; `--missed` refused client-side, nothing recorded. New
daemon, old CLI: a dispute with no item is refused with `InvalidParams`
instead of being recorded as `item: None` — the old client prints the
request failure. Migration failure → the daemon does not start; the store is
byte-identical to before the attempt (transaction rollback), the error names
`from`, `to` and the step. Fixture missing or unreadable → the test fails;
it is never skipped.

## Tests

- Journal: a synthetic day with deterministic, attested, manual, heuristic,
  unsealed, and `born: None` transitions counts exactly the trusted ones;
  ItemReplaced with a new id counts, a source change on one id does not.
- Report: dispute lines with `false_item`, `missed_item`, no
  `dispute_kind`, and an unknown value → the two columns, the default, and
  INCOMPLETE. Wire future-input fixtures: `AttentionDayFacts` and
  `AttentionDisputeParams` with unknown fields and an unknown `kind`.
- Daemon (integration, real corrald): `false_item` without an item refused
  and unjournaled; `missed_item` journaled without an item; the capability
  advertised.
- CLI: no current item → exit code and message; `--missed` against a
  daemon without the capability → refused, no request sent.
- Retention: a 91-day-old day pruned, a 89-day-old kept.
- Runner, injected chains: a two-step chain applies and lands the version;
  a failing second step leaves version and rows untouched; a gap refuses;
  newer refuses; below baseline refuses with the baseline named.
- `migration_baseline`: the fixture copied to a temp dir opens under the
  current build, its version equals `SCHEMA_VERSION`, and every fact in
  `expected.json` reads back through the public API.

## Definition of done

- PR A merged: D1–D3, glossary; `./scripts/verify` green on the final tree.
- PR B merged with `DURABLE-APPROVED-BY`: D4, ADR 0018, `verify-release`
  step; `./scripts/verify` green; `./scripts/verify-release` reaches and
  passes the migration step before its designed exit 1.
- `corral attention report` on a real daemon shows the new columns;
  `corral attention dispute --missed` on a session without an item records
  a `missed_item` line; without `--missed` it refuses.
- Exit condition for the founder, not this plan: delete the schema-1 store,
  open the `STORAGE_EPOCH` PR.

## Plan Size Justification

Over the target because it carries two owners that must both hold before
the same clock may start: the journal's measurement (PR A) and the store's
migration baseline (PR B). Splitting into two plans would let one merge
while the epoch stays blocked on the other, with nothing recording why.
