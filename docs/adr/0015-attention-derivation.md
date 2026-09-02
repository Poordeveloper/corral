---
status: proposed
read_when:
  - deriving, ranking, or rendering a session's main state
  - adding an evidence source, a hook event, or a screen rule that touches status
  - deciding what an attention item, a badge count, or an acknowledgement means
  - writing or changing a detection manifest, or the schema it is written in
  - deciding what survives a daemon restart about status, and what is recorded about it
---

> Structural rulings founder-accepted 2026-09-02, rounds 1–4
> (`docs/decisions/2026-09-02-pr8-attention-grill.md`); the ADR stays
> proposed until the PR8 matrix measures the load-bearing facts below and
> an acceptance reconciliation finds grill Q32's closing conditions met:
> the matrix artifact exists, every Q21 scenario has a capture or a
> measured absence, the noise catalog exists, every load-bearing fact is
> measured, covered by an accepted invariant, or marked a non-load-bearing
> limitation, and no semantic-capable rule exists merely because code came
> before evidence. Mechanics the rulings already fix may be built on a
> branch before then; nothing merges before then.

# Attention derivation: which evidence may assert which state, how a claim rots, and what the engine owns

`PRODUCT.md` §4 fixes the five main states and the collapse principle.
`ARCHITECTURE.md` §2 fixes the authority ladder, that PTY activity is the
default Working authority, that screen rules are versioned manifest data,
and that attention is derived in `corrald` only. `AGENTS.md` §Runtime truth
makes freshness-qualified authority law. None of them says how the ladder is
*applied*: which source may assert which state, what happens when two fresh
sources disagree, when a claim stops being one, what an item is, and what —
if anything — is written down. Scheduled by `ROADMAP.md` §3 for PR8; the
founder's UX contract (`docs/decisions/2026-08-21-m1-ux-contract.md` §1, §4)
is the acceptance evidence for the state and notification semantics this
materializes, and the decisions below that go beyond it are new.

**The invariant.** A main state is the most recent claim a fresh, entitled
source supports. A source may never assert a state the matrix has not
proved it can recognize, and nothing derived is ever durable.

## D1 — One engine, in `corrald`, and it is the only deriver

`corrald::attention` owns derivation. It consumes, per Session, an
evidence ledger: execution truth from the runtime, the sweep, and
reconciliation; provider facts from `hook_evidence` through the provider
adapters; PTY activity and screen readings from the screen thread; in-band
signals the emulator saw. It produces one `AttentionState` per Session —
main state, since when, the last reliable fact when the main state is
Unknown — and the Session's attention items. Derivation is a pure function
of the ledger and the clock, recomputed on every arrival and on a freshness
tick, so a test can put a ledger and an instant in and read a state out.

Clients render what `session.list` carries and derive nothing; an absent
attention field is an older daemon, and the client renders what it renders
today — Exited or Unknown from execution state alone — rather than a
claim of its own. The projection PR4 froze — execution
state may establish Exited and never manufactures Working, Needs You, or
Ready — stands: from this phase those three exist, and only this engine
asserts them.

## D2 — Execution gates semantics

`Exited` when the runtime's end is established, and then nothing else is
claimed. When execution is `unknown` — a Run reconciled `Unverifiable`, a
runtime the daemon cannot see — the main state is Unknown whatever semantic
evidence says: Working, Needs You, and Ready each begin with "runtime alive"
(`PRODUCT.md` §4), and a claim about an agent Corral cannot place is not one
Corral may make. `running` is the secondary fact shown beside Unknown.

Exited overrides a cached Needs You: the label is Exited, the secondary line
"Exited before you responded", and the item is invalidated, not resolved.

## D3 — Entitlement: a source asserts only what it is entitled to

Entitlement has two axes, and both must hold before a claim reaches a main
state.

**Association** — is the evidence about this Session? The binding it
arrives through must be Deterministic, Attested, or Manual. Evidence over a
Heuristic binding — a sweep row, an identity candidate — is secondary
metadata only, never a main state and never an item (`AGENTS.md` §Core
model).

**Interpretation** — may this source say this? Fixed here, per source:

```text
runtime / sweep / reconciliation    Exited; running as secondary fact;
                                    gates every semantic state (D2)
PTY activity                        Working, and nothing else
  (Corral-owned PTY only)
screen detection                    Needs You · Ready · Working, each only
  (Corral-owned PTY, sealed rule)   through a rule sealed for it (D6)
in-band signal                      what the sealed matrix row says the
  (Corral-owned PTY, sealed)        provider's sequence means, no more
provider hook / notify              the exact positive claim the sealed
  (Attested binding,                event directly denotes: Needs You
   version-sealed event)            from an event sealed as blocked on
                                    the user, Ready from turn-ended,
                                    Working from turn-started
history record                      nothing about the present (ADR 0016)
```

Sealed means what it meant in ADR 0014 D2: measured against the real
provider at a supported version, with the capture committed as a fixture
and a test that the rule fires on it and on nothing beside it. An unsealed
rule, an unrecognized hook event kind, and a notification type this build
has no word for all load, count, and assert nothing user-visible. The
display gate PR7 ruled for rows applies to states: discovery may collect
weak evidence freely; a main state needs evidence that supports its literal
claim.

**A received event may be sufficient.** Grill Q2 (a′) separates two
things `ARCHITECTURE.md` §2's "one weighted source, never load-bearing"
ran together. Delivery is unreliable, and that is the rule's whole
content: Corral never assumes every hook arrives, the absence of an event
proves no absence of a state, a missing or delayed event never wedges the
engine, a stale one never resurrects a state, and fresher eligible
evidence may invalidate it (D4). Semantics are another matter: a received,
fresh, Attested, version-sealed event may by itself be sufficient for the
exact positive claim it directly denotes — a sealed permission request
*is* Needs You, with no second signature from a screen heuristic — and for
nothing beyond that claim. On acceptance `ARCHITECTURE.md` §2 is
corrected so the sentence cannot be read as a semantic ceiling.

> Unreliable delivery does not imply weak semantics. A received attested
> event may prove what it directly says; nothing is ever inferred from an
> event that did not arrive.

**Version-sealed means the version that produced the event.** Nothing
carries it today — neither provider's payload names a version, and no
launch or sweep records one — and reading the version currently installed
on disk is not the same fact: a process started before an in-place update
runs the old one. So a runtime's provider version is established only
where Corral can bind installation metadata to that concrete runtime
(grill Q12): directly from a sealed versioned installation path; from a
mutable package root only when its metadata has not changed since the
process started, otherwise Unknown; for a managed launch at the launch
boundary — sealed metadata first, `--version` as the fallback — and bound
to the Run in memory; for an external runtime only where the sealed
recognizer chains runtime, installation, and metadata. It is cached per
runtime and install identity, never re-read per event, and a version that
cannot be established makes every version-sensitive claim from that
runtime unsealed: visible, diagnostic, Limited awareness.

And sealing is exact (grill Q13). A semantic claim is sealed for an exact
measured version, or for an explicitly approved finite range whose
semantic compatibility the matrix itself established — never for "the
same major.minor still parses". Payload compatibility is not semantic
compatibility: a patch release can change when an event fires, how many
times, and what completion or approval means, while every field still
decodes. A version the matrix has not sealed keeps its runtime visible,
keeps parsing as diagnostics, writes "matrix expansion due" to the
journal, and asserts no main state. That is more short-term Unknown than
inheritance would give, and the repair is making matrix expansion cheap
and automated — an M1 release prerequisite — never lowering the standard.

> Forward-compatible parsing may be optimistic. Semantic attestation may
> not be.

The provider adapter is where a hook fact learns whether it is *blocked*
or merely *idle*. The measured `Notification` of Claude 2.1.247 fires with
`notification_type: idle_prompt` when the agent is idle at its prompt — a
Ready re-observation, not a Needs You — and the adapter's current
"every Notification is awaiting input" reading is exactly the over-claim
D3 forbids. The types that mean blocked are sealed by the PR8 matrix; a
type outside it asserts nothing.

## D4 — Composition: the most recent entitled claim wins, and every claim has a horizon

Among fresh, entitled claims about one Session, the causally newest wins;
authority breaks a genuine ordering tie and nothing else. "Newest" is an
ordering Corral can establish — the daemon's own observation sequence, a
provider's sequence where it supplies one, order within one runtime — and
never a comparison of wall clocks across sources or machines (grill Q3).
A later claim from a lower-authority source therefore does displace an
older claim from a higher one — that is `AGENTS.md`'s "a stale
high-authority signal is invalidated by fresher contradicting evidence",
applied — but only to a state its own row in D3 lets it assert: a fresh
permission request is Needs You, and a screen that then plainly shows the
agent working again ends it, however much more authoritative the request
was. Older evidence never revives a state: a late turn-started arriving
behind a turn-ended is diagnostics, which is the failure Herdr rolled hook
state back over.

> Authority controls whether a source may make a claim. Fresh contradictory
> evidence controls whether that claim is still true.

Three consequences are fixed rather than left to tuning:

- **The screen is re-observed continuously.** A screen reading is dated at
  its last evaluation, so a blocker that stays visible is always the most
  recent claim, and a claim the screen supports does not rot while it
  supports it. When the screen changes and no rule matches, the screen
  asserts nothing — absence of a match is not a claim — and the last
  entitled claim from another source stands until its horizon.
- **Activity is the default, and a blocker beats it.** PTY output asserts
  Working only when no fresh Needs You claim stands: the prompt that blocks
  the agent is drawn by the same output flow that would otherwise read as
  work. Activity's own claim ends at the quiet horizon.
- **Every semantic claim rots.** Working, Needs You, and Ready each carry a
  freshness horizon per source, past which the main state is Unknown with
  the last reliable fact as secondary text — "Last known: Needed input
  45m ago". Rot is not resolution: no new notification, the old one
  invalidated, the badge falls, the row stays, runtime truth stays
  displayed (UX contract §1). Ready rots too, because on a runtime Corral
  does not own a missed turn-started would otherwise leave Ready standing
  over a working agent for hours; on a Corral-owned PTY the screen's
  re-observation keeps it alive for as long as it is true.

Horizon values are tuning, not contract (UX contract §1). The contract is
that each claim has one, that it is per source and per state, and that no
horizon is widened to make Unknown rarer.

## D5 — PTY activity is a named evidence source

On a Corral-owned PTY, activity is bytes the emulator consumed, dated on
the daemon's monotonic clock, excluding device replies. It asserts Working
and nothing else, and it exists only where Corral owns the stream: an
external runtime has no activity source, and nothing stands in for it.

The stream is also complete, and that is what makes quiet mean something.
Absence of a hook asserts nothing, because hooks drop. Absence of output on
a stream Corral reads every byte of is an observation that nothing was
drawn — which is why a Ready or Needs You claim on a managed session stands
through a silent hour, while on an external session the same silence is
absence and the claim rots.

## D6 — Screen detection is Corral-owned manifest data, sealed rule by rule

A detection manifest is one TOML document per provider:

```toml
schema = 1                      # the format; unknown fields are ignored
min_engine_version = 1          # refuse the manifest above the binary's
version = "2026.09.02.1"        # data version, recorded with every reading
provider = "claude"

[[rule]]
id = "permission-prompt"
asserts = "needs_input"         # needs_input | turn_complete | working
region = "bottom_non_empty_lines"
lines = 12
all = ["Do you want to proceed?"]
none = ["Esc to cancel"]
priority = 10
sealed_by = "docs/references/<matrix>.md#<scenario>"
```

Gates are substring gates in this phase; a regular-expression gate is an
additive schema change made when a sealed scenario cannot be pinned without
one, and the dependency it needs is decided then. Regions are the
emulator's own rows — `whole_screen`, `bottom_non_empty_lines`,
`osc_title` — read on the screen thread, because the emulator never leaves
it; readings are rate-limited and published as values.

Built-in manifests are compiled into `corrald`. An override directory under
Corral's state directory is read once at daemon start — no reload RPC, no
signal semantics, no watcher; a changed manifest means a restart, which
the idle lifecycle makes cheap (grill Q17) — so agent-UI drift is fixable
during dogfood without a binary; there is no remote channel
(`ARCHITECTURE.md` §2) and no upstream data in the correctness path
(`docs/decisions/2026-08-21-m1-decision-grill.md`, Herdr posture).
Compatibility is per level: an unknown field is ignored; a rule with an
unknown `asserts` or `region` is refused alone; a `schema` or
`min_engine_version` above the binary refuses the whole document and the
built-in stands. A refused override is reported, never silently ignored.

A rule earns the right to assert a state through demonstrated precision,
not a recognizable positive screenshot (grill Q14). It is sealed only
with: real positive captures from the claimed provider, version, and
surface; every other captured semantic state of that provider exercised
as a negative; adversarial near-miss fixtures — ordinary prompts, tool
output, errors, help, completion, redraws; no unresolved false-positive
case for it in the noise catalog; deterministic regression fixtures under
`./scripts/verify`; and the exact asserted state declared in the
manifest. In this phase a sealed rule may assert Needs You or Ready;
a rule asserting Working loads and is diagnostic only, because activity
and hooks already carry Working and "looks busy" has the blurriest edge.
A seal is revocable: a false positive from a dispute or a provider
upgrade disables or demotes the rule at once, is a P1 when it could
create a false Needs You, adds a minimized negative fixture, and is
re-sealed only with evidence.

## D7 — Items, acknowledgement, and what notifies

An attention item is born when a Session's main state enters Needs You or
Ready, and it carries an `AttentionItemId`: an ephemeral identity, valid
for the daemon's life, visible on the wire wherever acknowledgement needs
to name it, never reconstructed across a restart and never persisted
(grill Q19). Evidence-instance identity and item identity are distinct:
the same blocker whose evidence moves from a hook to the screen, or back,
is the same item, and no notification is re-sent for a source change.
Leaving the state and re-entering it — Needs You → Working → Needs You —
mints a new item, re-arms notification, and puts the badge back. An item
ends by resolution — a later entitled claim, the screen clearing, a turn
starting — by rot, by exit, or by supersession; each invalidates it, and
invalidation never rings. `AttentionReason` stays `NeedsInput` and `TurnComplete` — wire
`needs_input` and `turn_complete` — and a permission prompt, a question,
and a plan approval all produce `NeedsInput`: the reason says why the
user is needed, never the shape of the answer, which is M2's
`NeedsInputRequest` to define; what the provider said it is blocked on
rides the existing `NeedsInputContext` as display context and nothing
more (grill Q30). `AttentionReason::RuntimeEnded` stays reserved: Exited
is a state, not a notification class; whether an exit ever notifies is
the tray's policy later.

Acknowledgement is held by `corrald` and is consistent across surfaces,
and it names the item: `attention.acknowledge` carries the Session and
the `AttentionItemId`, is idempotent per item, and treats a stale id as a
no-op — `StaleAttentionItem` — that never acknowledges the replacement
(grill Q18). A delayed acknowledgement of a resolved item must not eat
the next real blocker. A Ready item is acknowledged when Open succeeded —
the terminal data channel is bound and the initial snapshot established —
never on the attach request, and never when the attach was refused or the
snapshot failed. A Needs You item is never acknowledged by viewing: only
explicitly, or by resolution. An acknowledgement clears the badge while
the row keeps its state, and does not move the row's rank.

What this phase implements is the **ephemeral acknowledgement of an
ephemeral item** (grill Q6 a′). An item has no identity beyond the
daemon's life — derived state is rebuilt from live evidence, and status
restored without a live signal is immediately stale (`ARCHITECTURE.md`
§2), so after a restart every Session reads Unknown until it acts — and
an acknowledgement is scoped to that item. A restart drops both, replays
nothing onto a rebuilt item, and never decides by fingerprint that a new
item is the old one. This is not the durable acknowledgement `AGENTS.md`
§Durable state names, and that law is not weakened here: when an
attention item gains a durable identity, acknowledging it is a
Corral-owned durable fact and is persisted then. What is missing in this
phase is the object, not the principle.

> Do not persist an acknowledgement without a stable object to
> acknowledge, and do not guess object identity across a restart.

Notification is a transition — an item became actionable — defined and
recorded here; delivering it to an operating-system surface is the tray's
(M1 completion work). Eligibility is fixed now: Attested-or-better
association only, sealed interpretation only, unsealed versions never,
known provider noise suppressed at the adapter or the manifest. Counts are the daemon's, carried on
the wire as a projection of the current items and never as a state of
their own (grill Q23): per class, `total` — sessions presently projected
in that state — and `unacknowledged` — those whose current item is not
acknowledged — with `0 ≤ unacknowledged ≤ total`; the TUI header shows
totals, a badge shows unacknowledged, and no surface recomputes either
from a filtered list. In this model a session has at most one current
Needs You or Ready item, because an item is born on entry and retired on
exit; the wire's item list stays extensible and is never historical
storage. An instant on the wire is named `since_unix_ms` and a duration
`age_ms`; no field is named ambiguously.

## D8 — Nothing derived is durable, and the gate's evidence is a diagnostic journal

The registry store gains nothing: no state, no item, no acknowledgement.
`ROADMAP.md` §5 still needs to count trusted Needs You transitions and
false ones, so the engine appends every transition to a dedicated
diagnostic journal — `~/.corral/diagnostics/attention-journal.jsonl`,
placed and named so nothing mistakes it for product truth (grill Q8 a′).
A record carries the Session, the previous and new projected state, the
evidence source class, reason, assurance, the configured horizon and the
actual expiry when a claim rotted and whether contradicting evidence came
first, the provider version and whether it was sealed, whether a
notification was emitted, an ordering sequence, and the build. It never
carries a raw screen, prompt text, a hook payload, tool arguments,
transcript content, or anything secret. A dispute from the CLI appends a
record naming the exact current or recent item — and noting when that
item was already stale on arrival — so that a dispute of A is never
attributed to the B that replaced it; it rewrites nothing.

The journal is one file per day, kept thirty days and pruned at daemon
start and at day rollover, with a per-day budget (initially 16 MiB) that
is never met by rotating early records away: when the day's budget is
exhausted, ordinary records stop, an explicit overflow marker is written
— a sidecar `.incomplete` is enough — a warning is emitted, and
`corral attention report` marks that interval INCOMPLETE, which means it
cannot count as a complete evidence day (grill Q26). A bounded journal
may become incomplete; it must never become silently incomplete. It is
deletable, never migrated, and never a promise of rebuildability. The invariant is about
direction: the attention engine never reads it back into product state or
semantic inference — not to suppress an item, not to produce one.
Reporting reads it, which is what it is for: `corral attention report` is
how the dogfood evidence exists without a grep over rotated tracing logs,
and it reports INCOMPLETE intervals as such, never as zero events, and
says which evidence question a window supports — attention fidelity,
observed aggregation, or both — because a managed-only window is complete
evidence for the first and none for the second (grill Q31).

## D9 — Sealing authority: automation gathers, people grant

Sealing is a human act on evidence. A credentialed `verify-release` job
may discover a provider version, run the sealed scenario suite against
it, capture events and screens, compare them with the sealed fixtures,
and propose a matrix row, a fixture diff, and noise-catalog changes; it
may never seal (grill Q22). A version whose measurements stay inside the
accepted semantic envelope is sealed by a high-consequence Class B change
under `HUMAN_REVIEW_REQUIRED` and a human merge, and the review reads
what a screenshot diff cannot — ordering, multiplicity, presence, timing,
payload meaning, negatives, the catalog — because an empty diff never
proves unchanged semantics. A measurement that needs new semantic
authority — a new evidence source category, a state assertion this ADR
does not authorize, a changed claim ladder, changed identity or assurance
meaning — leaves the expansion envelope and takes the decision path. A
manifest's `sealed_by` names the human-reviewed sealing evidence, never
the automation.

The measured ways evidence misleads live in one repository entity,
`docs/references/provider-noise-catalog.md` (grill Q29): a stable id,
provider, observed version or range, surface, the phenomenon, its
evidence, the risk if misread, a disposition — unresolved, suppressed by
adapter, excluded by a manifest negative, diagnostic-only, not semantic
evidence — and the regression fixtures that hold it. Tests cite ids;
runtime code never parses the catalog. A journal dispute is triaged by a
person and, when it reveals a reusable class, becomes an entry, a
deterministic fixture, and if needed a re-seal. Positive semantics —
"the idle prompt means Ready" — belong to the sealed matrix and the
manifest, never to the catalog, which records the confusion instead:
"the idle prompt must not be read as Needs You".

> The matrix says what evidence may mean. The noise catalog records the
> measured ways evidence misleads.

## Rejected

- **Hook state as the load-bearing source.** Both production references
  agree, from opposite directions (`ARCHITECTURE.md` §2). Hooks are
  entitled and weighted here; the engine is complete without them.
- **Client-side derivation, or a client counting for itself.** N clients
  re-deriving the state machine is the documented reference failure.
- **Hardcoded detection patterns.** Every UI change would be a release.
- **A floating confidence score.** Assurance is discrete; entitlement is a
  table; a number would be a way to average a claim into existence.
- **Promoting heuristic evidence to avoid Unknown.** The collapse
  principle forbids it by name.
- **Persisting status or items.** Derived state in the log is a
  reinterpretation waiting to happen; a restart honestly rediscovers.
- **A hook heartbeat (`PostToolUse`) in the entry sets, now.** It would
  keep Working alive on external Claude sessions and clear a stale hook
  Needs You after an approval given in the terminal, at a relay invocation
  per tool call. It is additive under ADR 0004 D6 and ADR 0013's repair
  cycle, and it is decided on measured interference and dogfood need, not
  here.

## Load-bearing facts, measured

Measured 2026-09-02 on Claude Code 2.1.258 and Codex 0.152.0
(`docs/references/2026-09-02-pr8-attention-matrix.md`; captures under
`crates/corrald/fixtures/screens/`), inside the PR7 spike's udocker
container on Linux, which leaves screen bytes, hook and notify payloads,
and their timings unaltered and does not measure the Linux external-Know
chain (grill Q16).

- Claude `Notification` types: `permission_prompt` fires 6 s after a
  pending `PermissionRequest`; `idle_prompt` fires 60 s after `Stop` at
  an idle prompt. The first confirms a standing item, the second is a
  Ready re-observation; neither is the request. **`PermissionRequest`
  exists**, fires 70–100 ms after `PreToolUse` for tool permission,
  AskUserQuestion, and ExitPlanMode alike, and carries `tool_name`,
  `tool_input`, and `permission_suggestions`.
- Screen shapes captured from the daemon's own emulator for every
  inventory item the driver reached: the three Claude dialogs share one
  structure — a `❯ 1.` option list under a rule, the mode bar absent;
  the fresh-directory trust dialog precedes `SessionStart`; Codex's
  approval dialog and its blinking `Action Required` title; the Ready,
  Working, spinner, resume-picker, help, paste, resize, typing, and
  permission-like-output negatives. Not induced: compaction and an API
  error on Claude; the `/` popup and compaction on Codex.
- Codex approvals announce themselves in-band through the OSC 0 title
  (`[ . ]`/`[ ! ] Action Required | proj`), focused or not;
  `tui.notifications` adds only a bare BEL when unfocused. Codex has no
  question surface. The title-generation thread's `agent-turn-complete`
  can arrive before the user turn completes — 9 s before an approval
  dialog — and is told apart by identity: the session's `thread-id`
  names its rollout file, the title thread has none.
- Redraw after turn end: Claude's screen settles 31 ms after `Stop`;
  Codex keeps the stream busy 4 s past its notify with the title turn's
  spinner. Both providers redraw continuously while running — a silent
  `sleep 8` included, at gaps under 230 ms — and while blocked; both are
  silent at idle.
- Late events after `Stop`: `SubagentStop` 2–6 s after every turn that
  generated a title, with no subagent; a background task's completion
  arrives as an unprompted `UserPromptSubmit` → `Stop` → `SubagentStop`.
  Esc on any dialog, in either provider, produces no turn-end event at
  all.
- Provider version: neither provider's events carry one; each channel's
  installation metadata and its cost are tabled in the matrix.

## What this does not decide

Capability rungs 1–2 for external sessions — S3's census, and a hook
protocol evolution with its own admission conditions (ADR 0004 D4).
Operating-system notification delivery and the tray. Horizon values. The
matrix-expansion automation that keeps exact sealing operable — an M1
release prerequisite with its own task.
Desktop rendering. History-enumerated sessions and continuation
(ADR 0016). Correction and unlink. A remote manifest channel. Structured
answerable requests (M2).
