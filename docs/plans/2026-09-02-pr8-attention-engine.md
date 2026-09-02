---
status: blocked   # ADR 0015 / ADR 0016 proposed; structural grill closed (rounds 1–4); PR8a's ruled mechanics are built on task/pr8-attention (grill Q32), nothing merges before acceptance
class: C
writes:           # one plan, two independently correct PRs (grill Q1); overlapping owners serialize
  pr8a: [corrald, corral-core, corral-protocol, corral-tui, corral]
  pr8b: [corrald, corral-state, corral-protocol, corral-tui, corral]
reads: [docs/adr/0002-resume-lineage.md, docs/adr/0004-hook-delivery.md, docs/adr/0007-managed-session-lifetime.md, docs/adr/0009-codex-notify-delivery.md, docs/adr/0013-global-hook-integration.md, docs/adr/0014-external-session-evidence.md, docs/adr/0015-attention-derivation.md, docs/adr/0016-history-enumerated-sessions.md, docs/decisions/2026-08-21-m1-ux-contract.md, docs/decisions/2026-09-02-pr8-attention-grill.md, docs/references/herdr-runtime-report.md, docs/references/architecture-benchmarks.md, ARCHITECTURE.md, PRODUCT.md, ROADMAP.md]
---

# PR8 — The Attention Engine: Know, and the loop it closes

**Class C, and why.** Two proposed ADRs cross canonical boundaries: the
agent-status evidence authority order becomes an entitlement table with a
composition rule, and a received sealed hook event may be sufficient for
the claim it denotes (ADR 0015; `ARCHITECTURE.md` §2 wording corrected on
acceptance); a history-claimed identity is Attested for its own claim and
continuable under a disclosure (ADR 0016; glossary corrected on
acceptance). A detection-manifest schema is introduced — a
compatibility-sensitive external surface the workflow already lists
(§8.2, §10). Structural rulings are founder-accepted
(`docs/decisions/2026-09-02-pr8-attention-grill.md`, rounds 1–4, closed);
the ADRs move to accepted by one acceptance reconciliation after the
matrix (Q32's closing conditions), never by a re-grill. No registry schema or event diff is
expected in either PR.

**Two PRs, one plan (grill Q1).** PR8a and PR8b below are independent
workstreams with their own owner claims, tests, definition of done, and
review checklist. Each must be correct on `main` alone: no invariant may
be left broken by one for the other to repair. Both write
`corral-protocol::method` and `corral-tui::presentation`, so they land
serially: PR8a first, PR8b rebased onto it (grill Q11). PR8a must be
independently dogfoodable without history rows; PR8b must be reviewable
as an additive history/continuation surface and invents no transitional
ranking or presentation of its own.

## Goal

Make Corral say what needs you, truthfully. PR8a: a daemon-side engine
derives the five main states of `PRODUCT.md` §4 from execution truth,
provider facts, PTY activity, and manifest-driven screen detection under
one entitlement and freshness rule; attention items, ephemeral
acknowledgement, and "Needs You n · Ready m" exist in the daemon and on
the wire; the TUI and CLI render them; a diagnostic journal makes the
release gate countable. PR8b: recent resumable sessions from the
providers' own stores join the list and offer Continue in Corral under the
daemon's disclosure. Together: See → Know → Control complete for managed
sessions, Know complete for every session Corral can place.

## Non-goals

No capability rungs 1–2 for external sessions — S3's census and a hook
protocol evolution with decision-hold are their own task; a live external
Needs You in this phase is Know without an action, said plainly. No
Continue in Corral for a discovered session whose runtime is still live
(grill Q5; a phase limitation). No operating-system notifications, no tray
(M1 completion). No Desktop (PR9). No history parsing, titles from
transcripts, search, or timeline (M2; grill Q9). No left-behind-branch
surface. No unlink or correction UI. No `PostToolUse` in any entry set
(grill Q7). No remote manifest channel. No persisted status, items, or
acknowledgements (grill Q6). `STORAGE_EPOCH` stays `dev`.

## Existing owner / architecture involved

`corral-core` owns the reserved vocabulary — `AttentionItem`,
`AttentionReason`, `NeedsInputRequest`, `Evidence`, `EvidenceSource` — and
gains the derived-state types. `corrald`: the screen thread in
`runtime::session` is the only place the emulator's rows exist;
`provider::*` adapters own the meaning of every provider event;
`hook_evidence` and `provider::reported` hold live provider facts; `sweep`
and `external_session` hold external runtime truth; `state::DaemonState`
is where per-Session live facts meet; `connection` projects
`session.list`; `lifecycle` is untouched — a daemon that is alive is not
Corral watching. `corral-state` owns the store operations that compose
durable events. `corral-protocol::method` owns the wire; `corral-tui`'s
`presentation` is the one place both terminal surfaces decide what a row
may say. Benchmarks ledger §6 fixes the settled shape: PTY activity as
Working authority, hooks weighted for delivery and entitled for received
semantics, manifests versioned, attention in the daemon only.

## Design 0 — The matrix, first, for both

Every load-bearing fact ADR 0015/0016 list is measured on the installed
Claude (2.1.258) and Codex (0.145.0) before either ADR is accepted and
before a manifest rule or a store layout is sealed — and only those
versions are sealed by PR8; no semantics are backfilled onto 2.1.252, and
PRODUCT §10's "latest stable + previous tested" is a release requirement
met by Q22's automation, not a PR8 merge requirement (grill Q28). The
inventory (grill Q21), captured from Corral's own emulator with OSC
titles: Claude positives — tool permission, AskUserQuestion, ExitPlanMode
approval, Stop → idle; Codex — command approval, any question prompt,
completed-turn idle; negatives and transitions for both — thinking
spinner, silent long tool, compaction, `/resume` picker, API error and
retry, help overlay, resize and redraw, typing at the prompt, a long
paste, permission-like wording in ordinary output, a blocker resolved and
the screen moving on, a blocker rejected or cancelled; every
`Notification.notification_type` observed, any `PermissionRequest`, and
the ordering of `Stop`, `SubagentStop`, and background-task events; every
Codex notify variant and its `tui.notifications`/OSC behavior;
redraw-after-`Stop` activity windows; both session-store layouts; how
each provider's version is bound to the runtime that produced an event —
the versioned-path and package-root shapes per channel (grill Q12). Needs
You for AskUserQuestion and ExitPlanMode follows from the measured
"blocked on an explicit user response", never from the tool name. Recorded as
`docs/references/2026-09-02-pr8-attention-matrix.md` (run 2026-09-02 in
the PR7 spike container on `ne`); captures under
`crates/corrald/fixtures/screens/<provider>/<version>/`, rendered by
`cargo run -p corrald --example replay_capture`; the driver in
`scripts/matrix/`.

Where it runs (grill Q10): the real-provider matrix on macOS; the Linux
process/discovery chain on the deterministic harness. The three gates
this produces are in "Gates" below.

---

# Workstream PR8a — engine, evidence, projection, protocol, surfaces (ADR 0015)

**Writes:** `corrald` (`attention`, `detection`, `provider::*` adapters,
`runtime::session` screen thread, `state`, `connection`), `corral-core`,
`corral-protocol`, `corral-tui`, `corral`.

## Design

**A1. Vocabulary (`corral-core`).** `MainState` — Working, NeedsYou,
Ready, Unknown, Exited — as domain state; `AttentionState { main, since,
last_known: Option<(MainState, at)> }`; `AttentionItemId`, a daemon-life
identity by construction; `Entitlement` as ADR 0015 D3's table expressed in
types, so an adapter's fact carries what it may assert rather than the
engine guessing from its source. `AttentionReason` unchanged —
`NeedsInput` for permission, question, and plan approval alike, the
distinction riding `NeedsInputContext` as display context only (grill
Q30); `RuntimeEnded` unproduced.

**A2. Evidence intake.**
- *PTY activity.* The screen thread publishes the last-output instant
  (monotonic) per Run beside its published geometry; device replies are
  not output. Input Corral wrote is recorded so the echo of a person
  typing can be discounted — a false Working, never a false Needs You,
  and tuning rather than contract.
- *Screen detection.* `corrald::detection`: manifest loading (built-in
  `crates/corrald/manifests/<provider>.toml` plus the state-directory
  override read at daemon start), `schema` and `min_engine_version`
  checks with ADR 0015 D6's refusal rules, rule evaluation over regions
  read from the emulator on the screen thread, rate-limited to one reading
  per settle interval after output. A reading is a value: rule id,
  asserted state, manifest version, evaluated-at, sealed-or-diagnostic.
  TOML is parsed with the `toml_edit` dependency already present; no new
  dependency; substring gates only.
- *Provider facts.* The Claude adapter splits `Notification` by
  `notification_type` per the matrix: sealed blocked kinds →
  `AwaitingInput`; `idle_prompt` → a turn-ended re-observation; unknown
  kinds → a counted fact asserting nothing. Codex `agent-turn-complete`
  stays turn-ended. Each fact carries whether its event is version-sealed
  for the runtime it arrived from.
- *Provider version.* Established per runtime and cached by runtime and
  install identity, never per event (grill Q12): a managed launch
  establishes it at the launch boundary — sealed installation metadata
  first (Claude's local `node_modules/@anthropic-ai/claude-code/package.json`
  and versioned-path channel, Codex's package root beside `bin/`),
  `--version` as the fallback (measured 10–20 ms Claude, 60–550 ms
  Codex) — and binds it to the Run; an external runtime gets one only where
  the sealed recognizer chains runtime, installation, and metadata, and a
  package root whose metadata changed after the process started yields
  Unknown. Sealing is exact or an explicitly approved range (grill Q13);
  an unsealed version keeps the row visible, parses diagnostically,
  journals "matrix expansion due", asserts nothing, and reads Limited
  awareness. An in-band signal the matrix seals becomes an adapter fact from
  the emulator's OSC handler; unsealed, it is diagnostics.
- *Execution truth* is read where it already is: managed
  `ExecutionState`, the sweep's incarnation table, reconciliation.

**A3. The engine.** `corrald::attention`, a focused module: a per-Session
ledger of the latest entitled claim per source, ordered by the daemon's
observation sequence (never by cross-source wall clock — grill Q3); a pure
`derive(ledger, now) -> AttentionState` implementing ADR 0015 D2–D4; item
birth and end; ephemeral acknowledgement; the journal writer. Recomputed
on arrival and on a freshness tick; results held in `DaemonState` beside
the reported facts. Initial horizons, policy defaults and not wire
contract (grill Q15): activity quiet 3 s; screen settle 200 ms; hook
Working 15 min; hook Needs You 5 min; hook Ready 2 h on an external
runtime, unbounded while a Corral-owned screen supports it. Items carry
an ephemeral `AttentionItemId` minted on each entry into Needs You or
Ready, kept across an evidence-source change for the same blocker, and
replaced on re-entry (grill Q19). A Ready item is acknowledged by the
daemon when Open succeeded — data channel bound and initial snapshot
established, never on the attach request or after a refused attach; a
NeedsYou item only by `attention.acknowledge`.

**A4. The journal.** `~/.corral/diagnostics/attention-journal-YYYY-MM-DD.jsonl`
(grill Q8, Q26): one record per transition or dispute with the fields
ADR 0015 D8 names — including configured horizon, actual expiry, whether
contradicting evidence came first, and the provider version and its
sealing — and none it forbids. Daily files, thirty-day retention pruned at
start and at day rollover; a 16 MiB per-day budget that, when exhausted,
stops ordinary records, writes a sidecar `.incomplete` marker, warns, and
makes `corral attention report` show that day INCOMPLETE — never rotating
early records away. Never read by `attention`; read only by the report.

**A5. Protocol (additive, `attention.v1` capability).** `session.list`
items gain `attention { state, since_unix_ms, last_known?, items[] }` —
open strings, absent means an older daemon, instants named `_unix_ms` and
durations `age_ms` (grill Q23). Items carry `attention_item_id`, `reason`,
`since_unix_ms`, `acknowledged`; at most one current item per session,
the list extensible and never historical. New methods:
`attention.summary` → per class `{ total, unacknowledged }`, the daemon's
projection of current items — the TUI header shows totals, a badge shows
unacknowledged, no client recomputes either;
`attention.acknowledge { session_id, attention_item_id }`, idempotent per
item and without a command id because it records no durable fact; a stale
id answers `StaleAttentionItem` and acknowledges nothing (grill Q18).
Ordering of `session.list` becomes daemon-owned attention rank — Needs
You, Ready, Working, Unknown with a live runtime, other recent or
non-active rows, Exited — recency then deterministic id within a tier;
acknowledgement changes the badge, not the rank. PR8b's history rows
form their own non-live recency tier.

**A6. Surfaces.** `corral-tui::presentation` grows `MainState` to five,
fed only by the attention field, with `PRODUCT.md` §4/§6 wording: "Needs
You", "Ready", "Working", "Running · Status unknown", "Last known: …",
"Exited before you responded", and PR7's "Limited awareness" beside
Unknown where integration cannot claim delivery. Header line "Needs You n
· Ready m" from `attention.summary`'s totals. Version copy (grill Q24):
an unsealed known version reads "Running · Limited awareness · Claude
Code 2.1.258 not yet verified by Corral" with help "Corral has not yet
verified attention support for Claude Code 2.1.258."; an unbindable
version reads "Running · Limited awareness · Claude Code version unknown"
with help "Corral could not reliably determine which Claude Code version
this session is running." — two facts, two copies, and "Unsupported
version" reserved for a real support decision. Keys: acknowledge. CLI (grill Q34): `corral list`
renders the same presentation; `corral needs` lists Needs You and Ready
as a projection of daemon truth; `corral ack <session>` resolves the
session's current acknowledgeable item, sends its exact
`attention_item_id`, tolerates `StaleAttentionItem` without acknowledging
a replacement, and reports no-current-attention rather than acknowledging
a future item; `corral attention dispute <session>` records the exact
current or recent item id, noting a stale one; `corral attention report
[--since]` reads the journal only and reports INCOMPLETE intervals,
transition totals, trusted Needs You totals, and known disputes, never
treating INCOMPLETE as zero, and names which evidence question a window
supports (C, A, or both).

**A7. Docs.** Glossary gains **Attention state**, **Entitlement**,
**Freshness horizon**, **Screen reading**, **Attention journal**;
**Detection manifest** is already there.
`docs/references/provider-noise-catalog.md` is created with ADR 0015
D9's fields and the initial entries — Codex title-generation notify,
post-Stop `SubagentStop`, background-task hooks after `Stop`, PTY echo
false-activity risk, permission-like strings in ordinary output — and
every manifest negative and adapter suppression cites an id.
`PRODUCT.md` §6 gains the two version copies. `ARCHITECTURE.md` §2's "never
load-bearing" sentence gains grill Q2's split on acceptance; `PRODUCT.md`
§10 gains the PR8 matrix rows as capability-scoped support — external
Codex: discovery, identity, and Ready where sealed; approval / Needs You
detection unsupported in M1 (grill Q20); `ROADMAP.md` §5's blocker becomes
"No systematic missed states within any provider / version / surface /
evidence capability that Corral claims as supported (`PRODUCT.md` §10)."
and §6 gains the approved Needs You evidence floor paragraph (grill Q27,
verbatim in the decision record) — both canonical prose changes under this
Class C decision; ADR 0004 D7's "the engine is PR8's" gains its
implemented-by note (workflow §11.2).

## Interfaces or persistence changed

Client protocol: additive fields and methods only; a PR7-vintage client
ignores them and keeps rendering Unknown/Exited from execution state,
asserted by future-input tests; a PR8 client against a PR7 daemon reads no
attention field and renders exactly what it renders today. Hook wire:
unchanged. Detection manifest schema 1: a new compatibility-sensitive
surface, with its refusal rules under test. Persistence: none. The Corral
root gains `diagnostics/` and the state directory a manifest override
directory, both Corral-owned, neither durable truth. Provider-owned files:
untouched.

## Failure / unknown states

Execution unknown → main state Unknown regardless of semantic evidence.
Runtime ended with a cached Needs You → Exited, "Exited before you
responded", item invalidated. Claim past its horizon → Unknown with last
known; no notification, badge falls. Event from a runtime whose provider
version cannot be established or bound, or whose version the matrix has
not sealed → asserts nothing; row visible, Limited awareness, journal
"matrix expansion due". Stale acknowledgement → no-op, replacement item
untouched. Attach refused or snapshot failed on a Ready session → item
stays unacknowledged. Manifest override refused → reported,
built-in stands, sessions keep deriving. Rule unsealed → runs, counts,
asserts nothing. Screen poisoned → no readings; hook and activity evidence
continue; the row already says "Screen unavailable". Hook dropped →
missed transition tolerated; the screen carries a managed session, an
external one rots honestly. Stale Needs You after a native approval →
stands until fresher evidence, a later event, or rot (grill Q7's recorded
limitation; the journal measures it). This is not confined to external
sessions: Claude's approval is followed by `PostToolUse`, which PR8 does
not consume, and no other entitled source may assert Working — screen
Working is refused by D3 and activity never clears a blocker — so a
managed Claude session also stands at Needs You from the permission
`Notification` until `Stop`, measured at roughly 30 s in matrix C2. Unknown
notification type → nothing asserted. Heuristic binding → secondary only,
never an item. Daemon restart → every Session Unknown until it acts;
items and acknowledgements gone, nothing replayed; journal keeps its
history. Journal unwritable → derivation continues, the failure is logged
once; diagnostics never gate product state.

## Tests

- Engine unit: `derive` over ledgers with a fake clock and explicit
  observation sequence — every D3 row's entitlement, causally-newest over
  authority, blocker-over-activity, rot per horizon with last-known,
  screen-supported claims not rotting, Exited override, late-event
  non-revival, item identity kept across a source change and replaced on
  re-entry, unsealed event asserts nothing.
- Version binding: versioned path establishes; package root read once
  and cached; metadata mutated after process start → Unknown; unsealed
  version → diagnostic parse, no claim, journal entry; explicit range
  sealed → claim.
- Manifest fixtures: each sealed rule fires on its capture, on no
  sibling capture, and on no adversarial near-miss fixture; a Working
  rule loads as diagnostic and asserts nothing; schema refusal, `min_engine_version` refusal, unknown
  field ignored, unknown `asserts` refuses one rule; override precedence
  and refusal reporting.
- Integration (MUST): managed Claude via the mock-provider harness through
  a real PTY and the real emulator — prompt submitted → Working from
  activity; permission capture replayed → Needs You; approval → the blocker
  stands until `Stop`, which is the limitation above and not the behaviour
  this line first claimed; `Stop` → Ready; silence → Ready holds; process exit with a standing
  Needs You → Exited override; hook Needs You then screen clear → resolved
  item. Managed Codex: turn-complete → Ready; approval capture → Needs
  You. External Claude (token-less deliveries on the deterministic
  harness): Working → Needs You → rot → Unknown with last known. Restart:
  everything Unknown, journal intact. Acknowledge: a successful open acks
  Ready, a refused attach does not, viewing never acks Needs You, a stale
  item id is a no-op while the replacement stays unacknowledged; summary
  counts.
- Journal: record shape, forbidden content absent by construction, daily
  rollover and pruning, budget exhaustion → `.incomplete` marker and an
  INCOMPLETE report interval with earlier records intact, dispute append,
  report aggregation.
- Protocol: future-input for the attention object, unknown state strings
  decode to no claim, PR7-shape decode of a PR8 list; compatibility for
  the new methods.
- TUI: insta snapshots for every main state, last-known, Limited
  awareness beside Unknown, both version copies, the header totals.
- CLI: `needs`, `ack`, `attention dispute`, `attention report`.

## Definition of done (PR8a)

- Matrix recorded and cited by every sealed rule and every sealed event
  kind; ADR 0015 accepted; this workstream unblocked before any boundary
  was crossed.
- A1–A7 implemented; `./scripts/verify` green on the final tree; snapshot
  coverage present; horizons recorded as tuning with their initial values.
- The journal and `corral attention report` produce `ROADMAP.md` §5's
  counts from day one of dogfood.
- `PRODUCT.md` §8 law holds in every rendered string: no "binding",
  "assurance", "entitlement", "manifest", "rule", "horizon", or "journal"
  reaches a person.
- Unverified Linux external Know is fail-closed and unclaimed (Gates);
  every sealed rule and event kind names its exact version or approved
  range.
- The initial provider noise catalog exists, and every semantic-capable
  screen or adapter rule cites its sealed evidence and noise fixtures
  (grill Q34).
- Under test: a delayed or stale `corral ack` cannot acknowledge a
  replacement item; a successful Open acknowledges Ready and a failed Open
  does not; journal overflow makes the report explicitly INCOMPLETE.
- Human-merged (Class C). Glossary entries landed; drift notes placed.

## Review checklist (PR8a)

Entitlement table matches ADR 0015 D3 exactly, in types; no path
manufactures Working/Needs You/Ready from execution state; no client
derives or counts; the screen thread still owns the emulator; nothing
derived reaches the registry; the journal carries none of D8's forbidden
content; every rendered string is PRODUCT §8-clean; future-input tests
cover the attention object.

---

# Workstream PR8b — history enumeration, recent list, continuation, disclosure (ADR 0016)

**Writes:** `corrald` (`history`, `provider::*` store layouts,
`connection`), `corral-state` (one new composing operation),
`corral-protocol`, `corral-tui`, `corral`.

## Design

**B1. History enumeration.** `corrald::history`: the `HistorySource` seam
with one implementation per provider under `provider::*`, sealed layouts
from the matrix, run at daemon start and on a cadence; identity from the
file name — Claude's top-level `<uuid>.jsonl`, Codex's
`rollout-<timestamp>-<uuid>.jsonl`; directories and `memory/` are not
rows — recency from mtime; content never opened. A location only where
Corral holds the Session's exact cwd or the encoding is proven reversible:
Claude's dashed directory name is not, so a pure Claude history row shows
"Claude Code · 2h ago · <short id>" and never the encoded name as a path
(grill Q25). Resolution against known Sessions by external id across
binding kinds; unresolved identities held as live history rows in
`DaemonState`; recent window 14 days, 30 rows per provider, newest mtime first, dedupe
by `(provider, external_id)` with the newest file winning — query
defaults, not wire constants (grill Q25).
Nothing durable until continuation. Titles: Corral's own for a Session it
holds, structural metadata only for a pure history row (grill Q9).

**B2. Continuation.** The eligibility ladder in `session.resume` gains
ADR 0016 D4's four answers; a live discovered session answers "Still
running outside Corral. Continuation is unavailable while this session
remains live." (grill Q5); `session.continuation` preflight returns the
daemon's decision, disclosure text and code, and a `disclosure_revision`
bound to that decision. A history row's continuation runs a
new `corral-state` operation beside `start_managed_session` and
`resume_managed_session`: `SessionCreated`, `BindingAdded` of the existing
`History` kind at Attested with provenance Discovered, and the Run, in one
transaction; then a managed launch like any other, whose first identity
report confirms or contests the store's claim.

**B3. Protocol (additive).** `session.list` items gain
`origin: "history"` and `last_active_ms`; `session.continuation
{ session_id }` → `{ decision, disclosure?, disclosure_revision }`;
`session.resume` gains `disclosure_revision`, and where a disclosure is
required the daemon recomputes eligibility and refuses unless the revision
matches — disclosure correlation, not consent, in the wire doc's words
(ADR 0016 D5, grill Q18). History rows join PR8a's ordering after live rows, by recency.

**B4. Surfaces.** Presentation: "Found in Claude Code history" / "Found in
Codex history" origin line, the structural metadata line, and the
disclosure prompt before Continue, in the daemon's words (grill Q33):
history row — "Corral can't tell whether this session is still running
somewhere else. Continuing here starts another <Provider> process for
this session." [Continue] [Cancel]; discovered live — "Still running
outside Corral. Continuation is unavailable while this session remains
live."; managed live — ordinary Running / Open; managed Unverifiable —
"Corral couldn't verify that the previous process ended, so continuation
is unavailable." CLI: `corral continue` runs the preflight, prints the
disclosure, asks unless `--yes`, and sends the `disclosure_revision`
either way — `--yes` skips the CLI's own question and nothing else (grill
Q34); `corral list` shows history rows.

**B5. Docs.** Glossary gains **History row**; `ARCHITECTURE.md` §1's
Attested/Heuristic lines gain grill Q4's claim-scoped clarification on
acceptance; `PRODUCT.md` §8 gains the history origin wording; ADR 0014
D6's "the surface is PR8's" gains its implemented-by note.

## Interfaces or persistence changed

Client protocol: additive; a PR7-vintage client renders a history row as
an unknown-origin row with execution `unknown`, which is true. Persistence:
no schema or event diff — one new store operation composing existing
events. Provider-owned files: read only, and only the session-store paths
the matrix sealed; never opened for content, never held open.

## Failure / unknown states

Store unreadable → no history rows, said in the log, never a fabricated
empty state. A file the sealed layout does not recognize → not a row.
Same id in the store and in a live Session → one row, decorated. History
row whose provider session is live elsewhere → the disclosure; the
provider's first report either confirms or contests (ADR 0004 D8).
Resume of a disclosure-requiring Session without a matching
`disclosure_revision` → refused, fresh preflight required. Managed
Run open or Unverifiable → refused (grill Q7 of PR5 stands). Continuation
transaction fails → nothing durable, row stays a history row, the person
is told.

## Tests

- Fixture stores per provider layout: enumeration yields the sealed files
  only; directories, `memory/`, and headless files excluded; mtime
  ordering; window and cap; dedupe by id; a Claude row shows no location
  while a Corral-known Session shows its exact cwd.
- Resolution: a known Session is decorated, never duplicated; a discovered
  live Session and its file are one row.
- Continuation: all four D4 answers; the history-row transaction commits
  atomically or not at all, and launches; the provider's report confirms,
  or contests, the claim; a missing or stale `disclosure_revision`
  refused, and a revision from an earlier decision refused after
  eligibility changed.
- Protocol: future-input for `origin: "history"` and `last_active_ms`;
  PR7-shape decode of a history row; compatibility for
  `session.continuation` and `disclosure_revision`.
- TUI: insta snapshots for history rows and the disclosure prompt.
- CLI: `continue` with and without `--yes`, and the revision carried
  from its own preflight; `list` with history rows.

## Definition of done (PR8b)

- Store layouts sealed by the matrix for both providers; ADR 0016
  accepted; this workstream unblocked before any boundary was crossed.
- B1–B5 implemented; `./scripts/verify` green; snapshot coverage present;
  the recent window recorded as tuning.
- Never an empty first list when the sealed store holds recent sessions;
  never a fabricated row.
- Under snapshot: a pure Claude history row shows no inferred cwd, and a
  Corral-known Session shows its exact trusted cwd (grill Q34).
- Continuation under test: the historical disclosure, external-live
  refusal, managed-live Open, Unverifiable refusal, stale
  `disclosure_revision` rejection, and `--yes` still preflighting.
- The provider's `--resume` from a directory other than the session's
  measured on the sealed versions and recorded in the matrix, before
  `history::resume_location_sealed` answers true for that provider.
- Human-merged (Class C). Glossary entry landed; drift notes placed.

## Review checklist (PR8b)

No file content is read; no title is parsed; the `HistoryBinding` claim
is worded as the store's claim and nothing more; no history row gains a
runtime or a main state; the continuation transaction is atomic; the
disclosure is decided by the daemon and shown in its words; the four
ladder answers are exhaustive in code.

---

## Gates (grill Q10)

1. **PR8 merge gate** — `./scripts/verify` with mock, unit, and
   integration coverage of the mechanics, plus every unverified path
   failing safely when evidence is insufficient. Real Linux E2E is not a
   repository merge gate on its own.
2. **Linux support / dogfood entry gate** — before Linux external
   observed-session behavior counts toward the A-thesis dogfood, trusted
   Needs You statistics, external Know validation, or the supported
   provider/platform matrix: a real provider on a real Linux process
   environment verifying the whole chain — a real provider process
   exists; the recognizer establishes approved runtime evidence; a
   token-less integration event arrives; identity/binding at the allowed
   assurance; the engine produces the expected projection; TUI/CLI show
   the row and state; lifecycle change converges — with a positive Claude
   flow, a known-negative noise process, process exit, version
   determination, and a Needs-You-capable path where claimed. The artifact
   is a dated `pr8-linux-external-know` record in `docs/evidence/` (a
   support claim, not research), automated later under `verify-release`
   rather than `verify` because it needs real provider credentials
   (grill Q16). udocker and a mock `/proc` are insufficient. No suitable
   Linux host is currently confirmed, and the plan assumes none; until the
   artifact exists Linux external Know is unvalidated and its evidence
   window has not started.
3. **Harness isolation** (founder ruling, round 5) — before either PR8a
   or PR8b merges, the end-to-end suite must be unable to reach the
   developer's own Corral. Both `corral` and `corrald` are validated as
   the intended test-support build; a wrong binary fails the harness
   before any process starts; execution cannot fall back to the account's
   canonical endpoint or state paths; a concurrent ordinary `cargo build
   -p corral` cannot redirect a run in progress; and the wrong-binary case
   has a permanent regression test. Promoted from a follow-up because
   `./scripts/verify` demonstrated the race on 2026-09-02: a concurrent
   plain build replaced `target/debug/corral`, and four attention e2e
   tests started a daemon under `~/.corral` (log writes only; no registry
   mutation, no Session created). The damage was small and the invariant
   violation is not: a suite may not depend on nobody rebuilding a binary
   while it runs. Production daemon identity and rendezvous semantics are
   not to be changed to satisfy the harness.
4. **Conditional escalation** — if PR8 would present Linux external Know
   to ordinary users by default at merge rather than behind an honest
   capability boundary, real Linux E2E becomes the PR8 merge gate.

Unverified implementation may merge behind an honest capability boundary;
unverified product claims may not.

**ADR 0015 status.** Accepted 2026-09-03, after the round-5 reconciliation
confirmed all nine conditions: the attention matrix evidence artifact
exists (`docs/evidence/pr8-attention-semantics-2026-09-03.md`); every
required Q21 scenario has a capture or an explicit measured absence;
provider/version rows are explicit and inherit nothing across versions;
every semantic-capable event or screen rule is sealed by human-reviewed
evidence, and `sealed_by` names it; Claude's `Notification` variants are
classified by `notification_type`, with unknown ones diagnostic only; the
noise catalog exists and the fixtures cite it; every load-bearing fact
points at measured evidence, an earlier accepted invariant, or an
explicitly non-load-bearing limitation; and the glossary, `ARCHITECTURE.md`
and `PRODUCT.md` carry received hook evidence supporting its own positive
claim, claim-scoped assurance, capability-scoped support and release
semantics, and unverified versions meaning Limited awareness rather than
inherited authority. Q1–Q34 do not reopen. What remains before PR8a lands
is the review pass and the harness-isolation gate above.

**When the dogfood window may start (grill Q31).** PR8a merged; a human
has advanced `STORAGE_EPOCH` to `dogfood`; the exercised
provider/version/capability rows are sealed; diagnostics function and the
counted interval is not INCOMPLETE; Linux external Know is excluded until
the Q16 artifact; PR8b is not required. Two evidence questions stay
apart: a managed-only window accumulates attention-fidelity (C) evidence
— false positives, staleness, acknowledgement, re-arming, synthesis — and
validates nothing about observed aggregation (A); trusted Needs You
counts reach an A verdict only from surfaces meeting the evidence floor.
Every report names which question its window supports.

**Implementation before the matrix (grill Q32).** On a branch, now:
core vocabulary, the pure engine and freshness mechanics, item identity,
synthetic-evidence tests, acknowledge-by-item, the journal, grill-decided
protocol structures, version-evidence plumbing, generic manifest schema
and validation that grants no unmeasured rule authority. After the
matrix: event → state mappings, Needs You / Ready screen rules, sealed
version rows, capture-dependent noise suppression, any adapter behavior
claiming measured semantics. Nothing merges while the ADRs are proposed.
Acceptance reconciliation closes when the matrix artifact exists, every
Q21 scenario has a capture or a measured absence, the initial noise
catalog exists, every load-bearing fact is measured, covered by an
accepted invariant, or marked a non-load-bearing limitation, and no
semantic-capable rule exists merely because code preceded evidence.

## Follow-ups

- Test-support guard for the `corral` binary: `support::corrald_binary`
  refuses a daemon built without the `CORRAL_TEST_ROOT` seam, but
  `TestAccount::corral()` runs whatever `target/debug/corral` is, and a
  concurrent plain `cargo build -p corral` during `./scripts/verify`
  handed the attention e2e a binary that resolved the real account's
  paths and tried to auto-start a daemon there (observed 2026-09-02;
  nothing was written). The same image check belongs on both binaries.
- S3 live-join census, then rungs 1–2: hook protocol decision-hold under
  the first-response lease (ADR 0004 D4's admission conditions), Claude
  IDE/MCP and Codex app-server channels.
- The left-behind-branch surface for continuing a live external session
  (ADR 0016 D4, second row), after S3.
- A push notification on the client protocol for item transitions, when
  the tray lands; polling at 1 Hz carries PR8.
- `PostToolUse` in the entry sets, decided on the journal's stale-Needs-You
  evidence and a measured interference cost (grill Q7). This is what closes
  the managed-Claude approval window: until an entitled source may say the
  turn resumed, a managed session reads Needs You from the permission
  `Notification` until `Stop`. Deciding it needs the evidence; asserting
  Working from the screen instead would change D3's authority order.
- Durable `AttentionItem` identity, and with it durable acknowledgement
  (grill Q6).
- Matrix expansion automation (grill Q13, Q22, Q28): a credentialed
  `verify-release` job that runs the sealed scenario suite against a
  newly discovered provider version, captures, compares with sealed
  fixtures, and proposes a row, fixture diff, and catalog changes; a
  person seals by high-consequence Class B review and merge, and a result
  needing new semantic authority takes the decision path. An M1 release
  prerequisite: it is how "latest stable + previous tested" is met without
  backfilling semantics onto untested versions.
- Unlink / "Not the same session" as first-class UI, with the correction
  mechanism ADR 0004 D8 deferred.
- Desktop integrates the five-state projection before PR9 merges
  (`docs/decisions/2026-08-22-surface-sequencing.md`).
- Dogfood measurements the plan commits to recording: stale Needs You
  duration after native approval; how often it is noticeable; whether
  later events clear it.
- Eviction for the live tables that only grow: the ledger keeps a tracker
  per Session it ever observed. It is small and it is not wrong; a daemon
  alive for weeks still accumulates.
- The screen thread wakes every settle interval even where no manifest is
  attached and nothing was drawn since the last reading. Blocking until
  there is something to read costs nothing and is what it did before.
- `AttentionReason::RuntimeEnded` has no wire spelling, so the encoder
  sends it as an unrecognized reason. Nothing mints such an item today —
  an exit ends items rather than opening one — so the arm is unreachable;
  it should become reachable or go.

## Grill status

Round 1 closed 2026-09-02 (`docs/decisions/2026-09-02-pr8-attention-grill.md`):
split, entitlement of received events, causal composition, claim-scoped
assurance, no live-external Continue, ephemeral acknowledgement, no
`PostToolUse`, diagnostic journal, no titles, three gates. Round 2 closed 2026-09-02: PR8a first; version bound to the producing
runtime, exact or explicit-range sealing; sealing discipline with
adversarial negatives and revocable seals, screen Working diagnostic only;
Needs You 5 min, Ready 2 h; Linux evidence in `docs/evidence/`, no host
assumed; no reload; item-addressed acknowledgement, disclosure revision,
ranking tiers, ack on successful open; ephemeral item identity;
capability-scoped support in the release gate. Round 3 closed 2026-09-02: scenario inventory with transition
negatives; automation proposes, people seal; totals and unacknowledged on
the summary, `_unix_ms`/`age_ms` naming; two version copies; enumeration
defaults and no location for the ambiguous Claude encoding; daily journal
with explicit INCOMPLETE; ROADMAP §5/§6 wording approved; PR8 seals only
measured versions without weakening PRODUCT §10; the noise catalog.
Round 4 closed 2026-09-02: `AttentionReason` unchanged; window start
conditions with C and A kept apart; ruled mechanics may start on a branch,
matrix-dependent semantics wait, nothing merges before acceptance;
continuation copy; CLI bound to item identity and disclosure revision;
DoD additions. **The structural grill is closed.** The Q21 matrix ran 2026-09-02
(`docs/references/2026-09-02-pr8-attention-matrix.md`) with the initial
noise catalog beside it; its evidence map is written against Q32's
closing conditions, and the reconciliation check is proposed as round 5 of
the decision record. Next: the founder's acceptance reconciliation —
`proposed → accepted` on both ADRs, or the named fact that fails and the
ruling it reopens.

**Built ahead of acceptance, on `task/pr8-attention` (grill Q32(b)).**
A1 vocabulary and the entitlement table; A3 engine, tracker, ledger, and
the one-second tick; A4 journal with disputes and the report; A5 protocol
(attention on `session.list`, summary, acknowledge, report, dispute,
`attention.v1`); A6 five-state presentation with snapshots, heading counts,
`a` to acknowledge, `corral needs|ack|attention report|dispute`; A2 PTY
activity publication with the echo window, the manifest loader with its
per-level refusals, screen readings on the settled screen, provider version
bound at launch and from the observed process; the built-in manifests
carry the matrix's rules **unsealed**. Waiting for acceptance, by ruling:
the sealed version rows (`attention::sealing` is an empty table), the
`sealed_by` lines in the manifests, the Claude adapter's `Notification`
split, the glossary and canonical prose changes A7 names. Until then every
session reads Unknown from the daemon — visible, diagnostic, and honest.

**Built ahead of acceptance, on `task/pr8b-history` (grill Q32(b)).**
B1 enumeration over the two store layouts (window, cap, dedupe by id,
no content read), gated per provider by `history::layout_sealed` —
false for both, so nothing is enumerated yet; the history tier of live
rows, resolved against the registry under any binding kind, known
Sessions decorated with the store's recency, the rest listed after the
external rows with `origin: "history"`, execution `unknown`, and
`last_active_unix_ms`; the five-state presentation says "Found in Claude
history" and the age, and nothing about a runtime or a location;
`session.continuation` with the four D4 answers in the daemon's words,
`disclosure_revision` bound to the decision's facts, `session.resume`
carrying it back and refusing a missing or stale one with
`stale_disclosure`; `corral continue --yes` still preflights, renders
the disclosure it answered, and carries the revision. The fourth rung
answers *eligible with disclosure* only once
`history::resume_location_sealed` holds for the provider — the matrix
has not measured `--resume` from a directory other than the one the
session was started in (ADR 0016, unmeasured), a history row carries no
location, and Corral does not guess one — so today a history row's
continuation is refused with that said. Waiting on that measurement and
on acceptance: the sealed layout rows, the composing store operation
(Session + `HistoryBinding` at Attested + Run, one transaction) and the
launch it precedes, the TUI's own disclosure prompt (the list currently
hands the daemon's words and the `--yes` command line to the person),
and the glossary and PRODUCT §8 prose.

## Plan size justification

Over target because it is two PRs' worth of plan by ruling (grill Q1),
sharing one milestone, one matrix, one vocabulary, and one gate
structure; duplicating the background into two plans would cost the
reviewer the cross-references and buy nothing. Each workstream is
separately executable from its own section, and each review seam —
vocabulary, intake, engine, detection, journal, protocol, surfaces;
enumeration, continuation, disclosure — stays separable.
