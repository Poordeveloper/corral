# M1 Completion Grill — rulings over dogfood readiness, packaging, notifications, and the release gate (rounds 1–2)

> Status: **design frontier cleared** (2026-09-06). Round 1 ruled what
> remains of M1 after the tray landed (`ROADMAP.md` §1, §4): the order of
> the remaining work against the 14-day evidence clock, the operational
> definition of the release gate's counts, dispute classification, the
> attention journal's protection and retention, the migration baseline
> the storage epoch's advance requires, the macOS bundle and the
> one-command install and uninstall, signing and notarization, Linux
> delivery, the build and release pipeline, the OS notification mechanism,
> `verify-release`'s missing gates, and the plan split. Round 2 ruled the
> questions those opened: the dispute with no current item, the support
> matrix, the distribution channel, versioning and release authority, the
> reading of "≥ 100 across Claude + Codex", tray-watchfulness evidence, and
> how uninstall stops the daemon. What remains is evidence and execution,
> not design. No ADR is accepted here; one was implied by Q6's migration
> baseline and is no longer needed — the founder withdrew that ruling on
> 2026-09-10 (§Amendments). Earlier rulings are not reopened: the dogfood
> start conditions (PR8 attention grill Q31), the notification product
> policy (tray grill Q12), the Quit gate (tray grill Q11), and the five
> gate conditions (M1 decision grill §1).
> Governing principle, ruled in Q1 and Q14:
>
> Start the long evidence clocks early; build independent delivery
> machinery in parallel; converge only at the final release gate.

Questions asked by the grill session of 2026-09-06; rulings verbatim below.

Facts the rounds were grounded in (found before asking, not decided):

- `STORAGE_EPOCH` is `dev`. No migration exists: `corral-state` knows
  schema 5 and refuses every other version; its own comment says the
  first migration is written by the change that needs one.
- The attention journal is JSONL per day under the diagnostics directory,
  30-day retention, declared deletable and never migrated (ADR 0015 D8),
  outside the epoch's protection. A transition record carries `source`,
  `assurance`, `sealed`, `provider_version`, `born`, `ended`, `item_end`
  and `notifiable`; `born` is filled by ItemBorn and ItemReplaced alike.
  `corral attention report` counts every record whose `to` is `needs_you`
  under the column `needs you`, filtering on nothing; no code defines
  "trusted".
- A dispute carries a session and, when the CLI has one, the current item
  id; no kind, no note. With no current item the daemon records
  `item: None`. Nothing can record a missed Needs You. Item ids are shown
  nowhere in the CLI, TUI, or Desktop.
- No OS notification code exists; the journal's `notifiable` flag has no
  reader. No packaging, installer, bundle, plist, signing, or uninstall
  tooling exists; the hook-level `corral integration` operations do
  (ADR 0006, ADR 0013), and ADR 0013 already obliges the installer to run
  `integration uninstall`.
- Daemon activation is sibling-only: the running executable is
  canonicalized and `corrald` must be its sibling. No shutdown RPC exists;
  `hello` does not expose the daemon's pid; SIGTERM enters ADR 0001's
  shutdown path.
- Manifests seal Claude Code 2.1.258 and Codex 0.152.0; the evidence is
  five dated records under `docs/references/`; no consolidated matrix
  exists; the coverage-audit harness does not exist; the provider noise
  catalog exists with six `unresolved` rows, three of them miss-shaped
  (`claude.reject.no-stop`, `codex.reject.no-notify`,
  `claude.api-error.ready-shaped`).
- `verify` runs on `ubuntu-latest` per PR; macOS `verify` runs only on a
  weekly schedule on `macos-latest`, unpinned. `verify-release` is
  incomplete by design and exits 1. `docs/evidence/` holds pre-merge PR
  evidence only. The workspace version is `0.0.0`.
- The founder's machine is Apple Silicon on macOS 26.5; the Linux matrix
  host is Ubuntu 24.04 x86_64 under PRoot, without a display.

# Round 1

Digest of what round 1 froze:

| Q | Ruling |
|---|---|
| Q1 | **(a).** First critical path: dogfood readiness → a human advances `STORAGE_EPOCH` to `dogfood` → the attention-fidelity and tray-watchfulness windows may begin at once. In parallel: packaging → a packaged `.app` → the notification probe → the notification-fidelity window. Packaging is never a prerequisite of the already-valid PR8 attention evidence. The three evidence questions stay distinct; the first two may accumulate before notifications exist, the third may not. Managed-only attention dogfood validates the C/fidelity bar and does not become A/observed-session evidence. |
| Q2 | **Measured unit renamed: trusted Needs You item activation.** Counted when the resulting state is Needs You, assurance is Deterministic or Attested, the evidence semantic is sealed for that provider/version/surface, and a **new** `AttentionItemId` became current. Manual never counts: it establishes a user-directed binding, not detection fidelity. ItemReplaced counts when a distinct Needs You item with a new id replaces the old one; the same blocker with a changed evidence source and the same id was frozen as one item and does not. Report shows `needs you (all)` and `trusted needs you`. The ROADMAP phrase may stay, pointing at this rule. **Not filtered by `notifiable`**: a trusted item discovered at a cold-start or reconnect baseline is valid fidelity evidence even when delivery is correctly suppressed. |
| Q3 | **Accepted with identity discipline.** `corral attention dispute <session>` → false-item; `--missed` → missed-item; optional `--note`. Wire/journal kind `false_item` / `missed_item`; a missing kind means `false_item`. A false-item dispute names the exact `AttentionItemId` whenever the item is available; when the CLI cannot establish which item is meant it refuses rather than attributing to whichever item is current. A missed-item dispute deliberately may carry no item id — its claim is that Corral failed to create one — and records at least session, timestamp, kind, optional note; no fake item is synthesized for the schema's sake. The journal records observations; the evidence review classifies them (avoidable, systematic, blocker): known noise an already-supported rule should have excluded → avoidable false; newly discovered → catalog entry plus deterministic fixture plus recorded disposition. Notes are diagnostic-only, never inference input, never provider evidence, never required for gate validity. |
| Q4 | **Every unresolved release-relevant noise row needs an explicit ship-time disposition**, and dogfood missed-item evidence participates. For the three miss-shaped rows the disposition is one of: fixed; correctly suppressed or reclassified; capability narrowed so the state is no longer claimed supported; documented non-systematic limitation supported by dogfood evidence. "Probably not systematic" without measurement is not available. Stronger rule: a measured condition that causes a repeatable miss inside a capability Corral still claims is systematic by construction — fix it or narrow the claim; low absolute frequency does not make it non-systematic. Genuinely intermittent misses record opportunities where measurable, confirmed misses, affected provider/version/surface, and the human disposition. No numeric miss-rate threshold is introduced; the bar stays *no systematic missed states inside a claimed supported capability*. |
| Q5 | **(a).** The journal stays diagnostic, deletable, non-authoritative, outside epoch migration protection; it is not promoted to durable truth because release evaluation reads it. Retention 30 → 90 days. At the end of each window the canonical report is generated immediately and frozen into `docs/evidence/`; the durable release claim is the reviewed evidence artifact, never the local journal's survival. Any INCOMPLETE day inside the 14-consecutive-day attention window breaks continuity; counting restarts from the next complete day; missing instrumentation is never read as "probably zero bad events", and the failure is itself reliability evidence. This does not silently redefine the cohort's 4-week rule, which owns its own calendar. Invariant: *diagnostic evidence may be bounded and deletable; release evidence may never be silently incomplete.* |
| Q6 | **Part 1 withdrawn 2026-09-10 (§Amendments): no migration runner, no fixture gate, no ADR; a schema change after the epoch advance is an approved reset. Parts 2 and 3 stand.** Original ruling: **Accepted in three parts, with a non-vacuous migration gate.** (1) `corral-state` gains a versioned forward migration runner: inspect the stored version; current → open; older supported → apply contiguous forward migrations, all steps and the version update in one transaction, failure → full rollback; newer than this binary → refuse; downgrade never attempted; no destructive fallback to a fresh database. `DOGFOOD_BASELINE_SCHEMA = 5` is frozen: at the epoch's advance schema 5 becomes the first protected baseline; zero real migrations exist and that is fine. A permanent representative schema-5 registry fixture is opened by the current build under `verify-release`; when the current schema exceeds 5 the complete chain must apply and the expected Corral-owned facts must survive, so a schema 6 cannot release without a working 5 → 6. Runner tests: rollback on failure, missing step rejected, newer schema rejected, old-build/new-DB refusal. (2) The local schema-1 database is dev-era state a human removes before the advance — allowed only while the epoch is `dev`; never encoded as startup behaviour; unavailable after dogfood begins. (3) The advance `dev → dogfood` is a human-only, repository-visible change in a dedicated commit/PR; an agent never advances it. Invariant: *dogfood starts only after schema 5 has become a migration-supported baseline future releases are obligated to preserve.* |
| Q7 | **Bundle accepted; identifier frozen: `com.poordeveloper.corral`.** `Corral.app/Contents/{Info.plist, MacOS/{corral-desktop, corrald, corral}}`, `CFBundleExecutable` = `corral-desktop`, the three executables siblings so sibling-only daemon resolution holds for the Desktop and for the CLI symlink after canonicalization. Install to `~/Applications/Corral.app`; `~/.local/bin/corral` → `Corral.app/Contents/MacOS/corral`. Regular Dock app; no LSUIElement conversion. |
| Q8 | **Install accepted; uninstall gains a safety gate.** Installer: identify OS/arch → resolve the exact release artifact → download → fetch and check the SHA-256 manifest → verify before extraction → place → CLI symlink → PATH action if needed → detect supported providers → disclose which integrations it intends to enable → `corral integration enable` per detected provider → per-provider status. An integration conflict never overwrites user-owned configuration, reports Limited awareness and the resolution path, and never rolls back an otherwise valid install. A checksum from the same release authority is integrity, not an independent trust root. **Uninstall preflight**: query runtime truth first; any managed runtime Running or Unknown → default uninstall refuses ("Corral is still managing N running sessions" / "could not verify whether U managed sessions have ended"); no destructive `--force` in M1. Successful uninstall: connect/activate → uninstall Corral-owned integrations → verify cleanup → request daemon shutdown → wait for ownership to end → remove symlink → remove `.app`/binaries; if cleanup cannot complete safely, fail before deleting binaries. Default preserves `~/.corral`; `--purge` removes state and diagnostics only after the preconditions succeed and is explicit destructive intent after `dogfood`. |
| Q9 | **Modified: three build classes.** Ad-hoc signing serves local developer dogfood and proves packaging mechanics only. The external cohort requires Developer ID Application signing, Hardened Runtime as the distribution design requires, a secure timestamp, notarization, and a Gatekeeper launch test on a clean user environment — never right-click Open, quarantine removal, or a Security Settings bypass as the onboarding path. Public M1 macOS release: Developer ID + notarization is a release gate, not polish; the packaging PR may merge without credentials, the cohort and public release cannot pass without them. Architecture: universal only if Intel is claimed; if claimed, every shipped executable carries coherent x86_64 + arm64 slices and both are validated; never a universal Desktop beside an arm64-only daemon or CLI. |
| Q10 | **Accepted with support-boundary wording.** `~/.local/share/corral/bin/{corral, corrald, corral-desktop}` as siblings; `~/.local/bin/corral` symlink; no `.desktop`, no Linux tray. `corral-desktop` is included and buildable but unvalidated and is neither placed on PATH nor marketed. Linux TUI/CLI supported per its tested matrix. Asset names carry the architecture; no generic `linux` artifact; only architectures actually built and tested may be claimed. |
| Q11 | **Accepted with exact-commit gating.** `scripts/package` is the one packaging entry point: release-profile `.app` archive, Linux tarball, checksum manifest, package metadata/version; signing/notarization steps when credentials exist. `verify-release` package gate: package → install into an isolated temporary HOME/prefix → `corral --version` → symlink canonicalization → sibling `corrald` activation → client handshake → uninstall → binaries and symlink gone, state preserved; isolated from the real `~/.corral`, provider configs, and application bundle, inheriting the e2e isolation invariant. A green weekly macOS verify on another commit is never release evidence: the release/tag workflow runs Ubuntu **and** macOS verify on the exact tagged commit before packaging, smoke, signing, and asset upload. |
| Q12 | **(a) preferred, sealed only by a Design-0 probe inside a real packaged `.app`** under the real bundle identity. The probe proves: authorization settings readable, requestable, and granted / denied / previously-denied distinguishable, with no repeated prompting after denial; Needs You with sound, Ready silent, session-keyed replacement, explicit withdrawal, acknowledged-item withdrawal; the click path — callback → bridge to the gpui main context → activate/reopen → select the `CorralSessionId` → the normal Open path — with the window visible and windowless; explicit delegate ownership, lifetime, and main-thread bridging. Corral-owned unsafe for the delegate stops at that boundary and lives only in a named macOS platform-boundary crate under the unsafe rule, never scattered into `corral-desktop`. Failure returns with evidence; no silent fallback to deprecated NSUserNotification or osascript. |
| Q13 | **Accepted with provenance checks.** `scripts/check-provider-matrix` verifies structural consistency among manifests/sealed versions, referenced evidence files, `docs/references/supported-matrix.md`, and sealing metadata; it never seals. Multi-platform: Ubuntu and macOS verify on the exact release commit, enforced by the workflow where practical, never satisfied by a scheduled run of another commit. Dogfood evidence: `docs/evidence/m1-dogfood-<date>.md` plus a machine-readable report generated by `corral attention report` for the exact interval; the record carries interval, build/commit, complete/incomplete days, needs-you-all, trusted activations, false-item and missed-item disputes, overflow/incomplete flags. `verify-release` validates report structure, interval continuity, thresholds, no INCOMPLETE day inside the consecutive window, and Markdown/machine agreement; humans add dispute classification, avoidable determination, noise disposition, capability interpretation, and sign-off. The gate prevents transcription convenience, not maintainer authority. No-quarantine gate checks the canonical quarantine state. Migration gate from Q6; packaging gate from Q11. *Machine gates validate mechanically derivable facts; human evidence decides what the machine cannot honestly infer.* |
| Q14 | **Four plans accepted** — `m1-dogfood-readiness`, `m1-packaging`, `m1-notifications`, `m1-release-gate` — as ownership boundaries, not serial phases. Graph: dogfood-readiness → epoch → the attention/tray clock; packaging → `.app` → notification probe → notification evidence; release-gate scaffolding starts early in parallel and is accepted only with real inputs (dogfood evidence, packaging, notification evidence, exact-commit CI, provider matrix, migration gate). Do not wait 14 days to write `verify-release`. The notification plan depends on a real `.app`, not on the attention window's completion. Decision record: this file; stale ROADMAP tray wording fixed with the first appropriate docs change. |

## Questions as asked (abridged, round 1)

- **Q1** — order: start dogfood readiness first and let the 14-day clock
  run while packaging proceeds, or package first, or serialize?
- **Q2** — the operational definition of a trusted Needs You transition:
  which assurance levels, whether ItemReplaced counts, what the report
  shows.
- **Q3** — dispute kinds `--missed` / `--note`, and whether "avoidable"
  is judged by the machine or by the evidence review.
- **Q4** — the three unresolved miss-shaped noise rows: disposed before
  ship, or judged by window counts alone?
- **Q5** — journal protection and retention; whether an INCOMPLETE day
  restarts the 14-day window.
- **Q6** — the migration framework as a precondition of the epoch
  advance; the founder deletes the schema-1 database and advances the
  epoch in a dedicated PR.
- **Q7** — macOS bundle layout, install location, CLI symlink, bundle id.
- **Q8** — the one-command install's steps; uninstall as a `corral`
  subcommand; state preserved unless `--purge`.
- **Q9** — ad-hoc for dogfood; whether notarization and a universal
  binary enter M1's packaging DoD or a later cohort build.
- **Q10** — Linux tarball, siblings under `~/.local/share/corral/bin`,
  Desktop included but unvalidated.
- **Q11** — `scripts/package`, tag-triggered CI on both runners, a
  package smoke gate in `verify-release`.
- **Q12** — notification mechanism: `objc2-user-notifications` vs
  `notify-rust` vs osascript; a Design-0 probe after packaging.
- **Q13** — the remaining `verify-release` gates and the evidence
  document's shape.
- **Q14** — four plans in order and this decision record.

## Founder rulings, verbatim (round 1)

```text
M1 completion grill — round 1 rulings
Q1 — ordering and the 14-day long pole
选择 (a)。
First critical path:
M1 dogfood readiness
→ human advances STORAGE_EPOCH to dogfood
→ attention-fidelity / tray-watchfulness evidence may begin immediately
In parallel:
packaging
Then:
packaged `.app`
→ notification mechanism/probe
→ notification-fidelity evidence window
Do not make packaging a prerequisite for the already-valid PR8 attention
dogfood evidence.
The three evidence questions remain distinct:

1. attention inference fidelity
2. tray/watchfulness lifecycle fidelity
3. OS notification delivery fidelity

The first two may accumulate before notification implementation exists.
The third may not.
Likewise:
managed-only attention dogfood may validate the C/fidelity bar,
but does not magically become A/Observed-session evidence.
So execution shape:
dogfood-readiness ───→ epoch → 14-day attention/tray window
│
└──── packaging ─→ notifications → notification window
This starts the longest serial clock as early as honestly possible.
Q2 — operational definition of "trusted Needs You transition"
方向接受，但 rename the measured unit internally to:
Trusted Needs You item activation
A counted activation requires:

* resulting attention reason/state = Needs You
* assurance = Deterministic OR Attested
* evidence semantic is sealed for that provider/version/surface
* a NEW AttentionItemId became current

Manual assurance does NOT count.
Reason:
Manual may establish a user-directed relationship/binding,
but it does not validate Corral's automatic Needs You detection fidelity.
ItemReplaced
Count it when:
old Needs You item A
→ genuinely distinct Needs You item B
→ B receives a new AttentionItemId
This is a new attention demand even if the primary state never passed
through Working/Unknown between them.
Do NOT count:
same blocker
+
evidence source changes
+
same AttentionItemId
That was already frozen as one item.
So the operational metric is:
StateEntered(NeedsYou with new item)
+
ItemReplaced(new distinct NeedsYou item)
not simply:
number of enum transitions into `NeedsYou`.
Report:
needs you (all)
trusted needs you
The ROADMAP phrase "trusted Needs You transitions" may remain,
but its canonical operational definition should point to the above item
activation rule.
Do not filter by:
notifiable == true
because a trusted item discovered during cold-start/reconnect baseline
is still valid attention-fidelity evidence even when notification policy
correctly suppresses delivery.
Q3 — dispute classification
接受，with identity discipline.
CLI:
`corral attention dispute <session>`
→ false-item dispute
`corral attention dispute <session> --missed`
→ missed-item dispute
optional:
`--note ...`
Wire/journal kind:
false_item
missed_item
Backward/default meaning:
missing kind
→ false_item
False item
A false-item dispute should identify the exact AttentionItemId whenever
the item is available.
Do not journal only:
"session X was wrong"
when Corral knows exactly which item was disputed.
If the CLI can no longer establish which historical/current item the user
means:
→ refuse ambiguous false-item recording
or require an explicit item-selection mechanism
rather than assigning the dispute to whichever item happens to be current.
Missed item
A missed-item dispute intentionally may have:
no AttentionItemId
because its claim is precisely:
Corral failed to create one.
It records at least:

* session
* timestamp
* kind = missed_item
* optional human note

Do not synthesize a fake AttentionItem merely so the diagnostic schema has
an id.
Human classification
Journal records observations:
false item reported
missed item reported
It does NOT automatically determine:
avoidable
systematic
release blocker
At evidence reconciliation:
false disputes
→ human classify against noise catalog / sealed rule evidence
If known noise should have been excluded by an already-supported rule:
→ avoidable false
If newly discovered:
→ add catalog entry + deterministic fixture
→ human records disposition
Likewise missed disputes feed the systematic-miss review.
Optional notes are diagnostic-only:

* never fed into inference
* never treated as provider evidence
* no raw note required for gate validity

Core principle:
The diagnostic records what the user observed.
The evidence review decides what that observation means for release.
Q4 — unresolved miss-related noise entries
接受：
every unresolved release-relevant noise row requires an explicit
disposition before ship
AND
dogfood missed-item evidence participates in that disposition.
For:

* `claude.reject.no-stop`
* `codex.reject.no-notify`
* `claude.api-error.ready-shaped`

ship-time disposition must be one of:

1. fixed
2. correctly suppressed / reclassified
3. capability narrowed so the state is no longer claimed as supported
4. documented non-systematic limitation supported by dogfood evidence

Do not allow:
"we reason that it probably isn't systematic"
without measured evidence.
Stronger rule:
If a measured condition causes a repeatable miss inside a capability
Corral still claims as supported,
it is systematic by construction.
The valid outcomes are then:
fix it
OR
narrow the support claim
Calling it "non-systematic" is not available merely because its absolute
frequency is low.
For genuinely intermittent/noise-dependent misses,
the evidence record must contain:

* opportunities observed where measurable
* missed disputes / confirmed misses
* affected provider/version/surface
* human disposition

No universal numeric miss-rate threshold is introduced in this grill.
The already-frozen release bar remains:
no systematic missed states inside a claimed supported capability.
Q5 — journal retention and evidence preservation
选择 (a)。
The attention journal remains:
diagnostic
deletable
non-authoritative
outside STORAGE_EPOCH migration protection
Do NOT upgrade it into product durable truth merely because release
evaluation reads it.
Change initial retention:
30 days → 90 days
Reason:
supports:

* 14-day fidelity window
* longer A/cohort observation
* reconciliation delay

without pretending diagnostics are permanent product state.
Evidence freeze
At the end of each evidence window:
generate the canonical report immediately
and commit/freeze the relevant report material into:
`docs/evidence/...`
The durable release claim is the reviewed evidence artifact,
not indefinite survival of the local diagnostic journal.
INCOMPLETE day
For the accepted 14-consecutive-day attention-fidelity window:
any INCOMPLETE day:
→ that 14-day consecutive window fails continuity
→ restart counting from the next complete day
Do not reinterpret missing instrumentation as:
"probably zero bad events."
A daemon/journal failure that creates an incomplete day is itself useful
dogfood reliability evidence.
This ruling does NOT silently redefine every separate 4-week cohort rule.
An incomplete interval cannot support quantitative claims,
but whether an external cohort's calendar restarts is owned by that
cohort's already-accepted evaluation definition.
Core invariant:
Diagnostic evidence may be bounded and deletable;
release evidence may never be silently incomplete.
Q6 — migration framework before dogfood epoch
接受三-part direction，with a stronger non-vacuous migration gate.
1. Forward migration framework
`corral-state` gains a versioned forward migration runner.
Rules:

* inspect stored schema version
* current version → open normally
* older supported version → apply contiguous forward migrations
* all migrations + schema-version update occur in one transaction
* migration failure → rollback entire migration
* newer-than-this-binary database → refuse
* downgrade is never attempted

No destructive fallback to a fresh database after migration failure.
Dogfood baseline
Freeze:
DOGFOOD_BASELINE_SCHEMA = 5
At the moment storage_epoch advances to dogfood,
schema 5 becomes the first protected baseline.
Today:
CURRENT_SCHEMA = 5
therefore:
zero real production migrations exist yet.
That is fine.
But the migration gate must be constructed so it becomes non-vacuous
automatically later.
Permanent fixture:
a representative schema-5 registry fixture
`verify-release` requires:
schema-5 fixture
→ open with current build
→ if CURRENT_SCHEMA > 5:
apply complete 5→...→CURRENT chain
→ verify expected Corral-owned facts survive
Thus when schema becomes 6:
release cannot pass until:
5 → 6
exists and succeeds.
Also add migration-runner tests for:

* transaction rollback on failure
* missing migration step rejected
* newer schema rejected
* old build/new DB refusal semantics

2. Local schema-1 database
The existing local schema-1 registry is dev-era state.
Before advancing epoch:
human explicitly removes/resets that local dev database.
This is allowed because storage_epoch is still dev.
Do not encode:
"delete any old database"
into product startup behavior.
After dogfood begins,
the same destructive shortcut is no longer allowed.
3. Epoch advance
After framework merged and verified:
human-only repository-visible change:
STORAGE_EPOCH:
dev → dogfood
Prefer a dedicated commit/PR whose purpose is only this clock advance.
Agent must not advance it automatically.
Core invariant:
Dogfood starts only after schema 5 has become a migration-supported
baseline that future releases are obligated to preserve.
Q7 — macOS delivery shape and bundle identity
接受 proposed bundle layout.
Use bundle identifier:
`com.poordeveloper.corral`
Freeze it now.
Reason:
bundle identity will become relevant to:

* macOS application identity
* notification authorization/delivery
* signing/notarization
* future upgrade behavior

Changing it casually later would make the OS treat Corral as a different
application for several purposes.
Bundle:
Corral.app/
Contents/
Info.plist
MacOS/
corral-desktop
corrald
corral
`CFBundleExecutable`:
corral-desktop
All three executable files remain siblings.
Therefore existing sibling-only daemon resolution remains valid for:
Desktop
and
the CLI symlink after canonicalization.
Install:
`~/Applications/Corral.app`
CLI entry:
`~/.local/bin/corral`
→ symlink to `Corral.app/Contents/MacOS/corral`
The canonicalized executable path still resolves back into the bundle,
where `corrald` is its sibling.
M1 keeps the already-accepted Regular Dock app behavior.
No LSUIElement/menu-bar-only conversion.
Q8 — one-command install / uninstall
Direction accepted, with an important uninstall safety gate.
Install
Canonical UX may be:
`curl -fsSL <installer> | sh`
Installer:

1. identify OS / architecture
2. resolve exact release artifact
3. download artifact
4. download/check release SHA-256 manifest
5. verify before extraction/install
6. place canonical installation
7. create CLI symlink
8. report PATH action if needed
9. detect supported providers
10. transparently disclose which integrations it intends to enable
11. invoke normal `corral integration enable` for detected supported
providers
12. print final per-provider status

A provider integration conflict/failure:
→ do not overwrite user-owned configuration
→ report Limited awareness / resolution path
→ do not roll back an otherwise valid Corral installation
The checksum protects artifact integrity/corruption;
do not describe a checksum fetched from the same release authority as an
independent signing trust root.
Uninstall preflight
Before making any mutation:
query daemon/runtime truth.
If any Corral-managed runtime is:
Running
OR
Unknown
default uninstall MUST refuse.
Reason:
stopping/removing corrald while it owns live PTYs can terminate or orphan
user work.
User message conceptually:
"Corral is still managing N running sessions"
and/or
"Corral could not verify whether U managed sessions have ended.
End or resolve them before uninstalling Corral."
M1 does NOT need a destructive `--force` that knowingly tears down managed
runtime ownership.
This is stronger and safer than:
integration uninstall
→ kill daemon
→ hope sessions survive
Successful uninstall
When no managed Running/Unknown runtimes remain:

1. activate/connect to corrald as necessary
2. uninstall Corral-owned provider integrations
3. verify integration cleanup outcome
4. request/allow daemon shutdown
5. wait for daemon ownership to end
6. remove CLI symlink
7. remove `.app` / installed binaries

If integration cleanup cannot be safely completed:
default uninstall fails before deleting binaries
rather than leaving a potentially broken residual integration.
This is particularly important even though installed hooks are designed
fail-open.
State
Default uninstall preserves:
`~/.corral`
`--purge` additionally removes Corral state/diagnostics
only after the normal uninstall preconditions succeed.
After STORAGE_EPOCH=dogfood,
`--purge` is explicitly destructive user intent,
not a silent recovery path.
Q9 — macOS signing, notarization, architectures
Modify proposed policy.
I do not have evidence that you currently have an Apple Developer Program
membership / usable Developer ID credentials, so packaging must not assume
they exist.
Separate three build classes.
Local developer dogfood
Ad-hoc signed build is acceptable for:

* your own machine
* controlled internal development
* non-distribution testing

This can satisfy:
packaging mechanics work
It cannot satisfy:
normal external macOS distribution is ready.
External cohort
Before giving Corral to the five-user external cohort through normal
download channels:
require:

* Developer ID Application signing
* Hardened Runtime as required by the distribution design
* secure timestamp
* Apple notarization
* successful Gatekeeper launch test on a clean/test user environment

Do not make cohort members learn:
right-click Open
xattr quarantine removal
Security Settings bypass
as the normal installation path.
That would contaminate product onboarding evidence.
Public M1 macOS release
Developer ID + notarization is a release gate,
not a post-M1 optional polish item.
So:
packaging implementation PR
may merge without credentials
but:
external cohort / public release
cannot pass the macOS distribution gate without them.
Architecture
Do not make "universal" unconditional unless Corral claims Intel support.
If M1 macOS support matrix says:
Apple Silicon only
→ arm64 package is honest.
If it claims:
Apple Silicon + Intel
→ all shipped relevant executables must contain/ship compatible
x86_64 + arm64 builds and both architectures require validation.
Do not make only:
corral-desktop
universal while shipping an arm64-only:
corral
or corrald.
For a universal app distribution, executable slices must be coherent
across the bundle.
Core principle:
Ad-hoc proves packaging mechanics.
Developer ID + notarization proves normal macOS distribution.
Q10 — Linux delivery
接受 with support-boundary wording.
Artifact layout:
`~/.local/share/corral/bin/`
contains:

* corral
* corrald
* corral-desktop

CLI symlink:
`~/.local/bin/corral`
→ installed `corral`
Keep all siblings together for daemon activation.
No `.desktop` registration in M1.
No Linux tray.
Do not place `corral-desktop` on PATH or otherwise market it as validated
merely because the binary is included.
Support statement:
Linux TUI/CLI:
supported according to its tested matrix
Linux Desktop rendering:
included/buildable but unvalidated
Asset names must include architecture.
Do not publish a generic `linux` artifact whose architecture is implicit.
Only architectures actually built/tested by release infrastructure may be
claimed supported.
Q11 — build/release pipeline and packaging gate
接受 general structure with exact-commit gating.
`scripts/package`
One repository-owned packaging entry point.
Builds release-profile artifacts.
Produces:

* macOS `.app` archive
* Linux tarball
* checksum manifest
* package metadata/version

When signing credentials are available,
macOS packaging additionally performs the accepted signing/notarization
steps.
Package smoke test
`verify-release` package gate:
package
→ install into isolated temporary HOME/prefix
→ verify `corral --version`
→ verify symlink canonicalization
→ activate sibling corrald
→ verify client handshake
→ uninstall
→ verify installed binaries/symlink removed
→ verify Corral state remains by default
Test environment MUST be isolated from:

* real `~/.corral`
* real provider configs
* real user application bundle

This inherits the earlier e2e isolation invariant.
CI release commit
Do not use:
"the latest weekly macOS verify was green"
as release evidence for another commit.
The release/tag workflow must run or depend upon:
Ubuntu verify
AND
macOS verify
for the exact release commit/tag.
Only after the exact commit gates pass should release assets become
publishable.
Recommended workflow shape:
release tag/commit
→ exact-commit verify jobs
→ packaging jobs
→ package smoke tests
→ signing/notarization where required
→ upload assets/checksums
→ publish/draft according to release process
Weekly macOS verify remains useful regression evidence,
but it is not a substitute for release-commit verification.
Q12 — macOS notification mechanism
选择 A as preferred mechanism:
`UNUserNotificationCenter`
via `objc2-user-notifications`
but final mechanism acceptance is conditional on a Design-0 probe inside
a real packaged `.app`.
The probe must verify more than "a banner appears".
At minimum:
Authorization

* current notification settings can be read
* authorization can be requested
* granted
* denied
* previously denied

are distinguishable
Do not repeatedly prompt after denial.
Exact user-facing authorization UX remains owned by the notification plan,
but the probe must expose the real states.
Delivery semantics
Prove:

* Needs You notification with sound
* Ready notification without sound
* session-key replacement
* explicit withdrawal
* acknowledged item withdrawal

Click path
Notification click:
→ callback arrives
→ bridge to GPUI/main application context
→ activate/reopen Desktop
→ select correct CorralSessionId
→ normal Open path
Test both:

* window visible
* windowless tray/watchfulness mode

Delegate lifetime / threading
Because the UserNotifications delegate is weak and the relevant Objective-C
objects are not general Send/Sync application-state objects,
the implementation must demonstrate explicit ownership/lifetime and
main-thread/event bridging.
If implementing the delegate requires Corral-owned unsafe:
STOP at that boundary
and put all such code in an explicitly named macOS platform-boundary crate
under the repository's unsafe governance rule.
Do not scatter unsafe into `corral-desktop`.
Bundle identity
Probe uses the real:
`com.poordeveloper.corral`
bundle identity.
Do not test notification authorization under a disposable bundle id and
assume permissions transfer.
If the probe passes:
the notification plan may seal this mechanism.
If not:
return with evidence rather than silently falling back to deprecated
NSUserNotification or osascript.
Q13 — verify-release and evidence artifacts
Direction accepted with stronger provenance checks.
Provider matrix gate
`scripts/check-provider-matrix` verifies structural consistency among:

* manifests / sealed versions
* referenced evidence files
* `docs/references/supported-matrix.md`
* sealing metadata

It does NOT auto-seal new versions.
Human-reviewed matrix evidence remains the authority.
Multi-platform release gate
Evidence must refer to:
Ubuntu verify
AND
macOS verify
on the exact release commit.
A human evidence document may contain the CI run references,
but scheduled verification from a different commit cannot satisfy this
gate.
Where practical,
the release workflow should enforce this mechanically rather than relying
only on prose.
Dogfood evidence
Canonical artifact:
`docs/evidence/m1-dogfood-<date>.md`
but do NOT make:
"script parses manually typed frontmatter numbers >= threshold"
the sole release check.
Generate a machine-readable report artifact from:
`corral attention report`
for the exact evidence interval,
for example JSON alongside the Markdown evidence record.
Evidence record contains/references:

* exact interval
* Corral build/commit
* complete/incomplete days
* needs-you all count
* trusted Needs You item activation count
* false-item disputes
* missed-item disputes
* journal overflow/incomplete flags

`verify-release` validates:

* generated report structure
* interval continuity
* threshold numbers
* no INCOMPLETE day inside the required consecutive window
* Markdown summary agrees with machine-readable report

Human evidence section then adds what machinery cannot decide:

* dispute classification
* avoidable false determination
* unresolved/missed noise disposition
* capability/support interpretation
* founder/maintainer signoff

A user could always edit repository evidence files;
this gate is not a cryptographic anti-maintainer system.
Its purpose is to prevent accidental or convenient number transcription,
not to eliminate human authority.
No-quarantine gate
Keep the already-accepted release condition explicit:
no active release-critical quarantine / known gap may be hidden behind a
passing evidence summary.
`verify-release` must check the repository's canonical quarantine state or
equivalent recorded source of truth.
Migration gate
Include Q6's schema-5-baseline migration check.
Packaging gate
Include Q11 package/install/uninstall isolated smoke.
Core principle:
Machine gates validate mechanically derivable facts.
Human evidence decides classifications the machine cannot honestly infer.
Q14 — plan split and execution graph
接受 four plans:

1. `m1-dogfood-readiness`
2. `m1-packaging`
3. `m1-notifications`
4. `m1-release-gate`

Decision record:
`docs/decisions/2026-09-06-m1-completion-grill.md`
Update stale ROADMAP tray wording with the first appropriate docs change.
But the four plans are logical ownership boundaries,
not four fully serial implementation phases.
Execution graph:
dogfood-readiness
→ epoch
→ attention/tray evidence clock starts
packaging
→ packaged `.app`
→ notification Design-0 probe
→ notification implementation/evidence
release-gate scaffolding may begin early in parallel,
but final acceptance cannot occur until it has real inputs from:

* dogfood evidence
* packaging
* notification evidence
* exact-commit CI
* provider matrix
* migration gate

So:

      ┌─ dogfood-readiness → epoch → evidence ───┐
      │                                           │
main ─┤                                           ├→ final release gate
      │                                           │
      └─ packaging → notifications → evidence ────┘
                release-gate scaffolding ─────────┘

Do not wait 14 days before writing `verify-release`.
Build the gate early,
then let real evidence fill it.
The notification plan depends on a real packaged `.app`,
but not on completion of the 14-day attention window.
Core principle:
Start long evidence clocks early.
Build independent delivery machinery in parallel.
Converge only at the final release gate.
```

(The Q14 diagram is reproduced with its fences removed and its columns
re-aligned, since the founder's message carried it across broken code
fences; typographic quotes are normalized to ASCII. Nothing else is
altered.)

# Round 2

Digest of what round 2 froze:

| Q | Ruling |
|---|---|
| Q15 | **(a), semantics pinned.** A false-item dispute binds only the current active attention item. No current item → non-zero exit, no journal record, message `No current attention item. If Corral failed to surface an item, use --missed.` `--missed` is a distinct statement of fact — "should have appeared, did not" — not a dispute without an id. A false positive that has already ended is never attributed by guessing the most recent item; the evidence review matches it by session and wall-clock time against `born` / `ended`. Item ids stay out of the normal product UI; a diagnostics surface, if ever needed, is its own task. |
| Q16 | **Amended 2026-09-11 (§Amendments): the minimum macOS is the deployment target the build compiles against, not the runner's major.** Accepted with a narrowed Linux claim. macOS: Apple Silicon only; minimum version = the lowest macOS major M1 CI actually verifies, filled into `LSMinimumSystemVersion` after the runner image is pinned. Linux: **Ubuntu 24.04 x86_64 only** for M1 — `ubuntu-latest` plus the PRoot host prove nothing about Debian, Fedora, Arch, or older Ubuntu; a glibc baseline may replace the distro claim only if the build actually establishes one. aarch64 Linux is explicitly unsupported and unclaimed. |
| Q17 | **Accepted, artifact pin distinguished from installer pin.** GitHub Releases is the canonical distribution; the root `install.sh` is a bootstrap served from `main`; default = latest published, non-draft, non-prerelease; `CORRAL_VERSION=vX.Y.Z` pins the release artifact, **not** the revision of `install.sh`, which is a moving target. No installer release asset in M1 and no claim of a fully reproducible pinned installation; freezing `install.sh` at the tag arrives with a later supply-chain task. |
| Q18 | **Accepted with two invariants.** (1) tag `vX.Y.Z` must mechanically equal `workspace.package.version = X.Y.Z`, else the workflow fails before any draft release. (2) draft → publish is the only human release authority: human pushes tag → CI verifies the exact tagged commit → package → smoke → checksums → draft release → release-gate evidence accepted → human publishes. "CI ran" leaves the human evidence document; the machine proves it. `0.0.0 → 0.1.0` for M1. |
| Q19 | **(a), count unit pinned.** ≥ 100 trusted activations in aggregate across the declared provider set — not journal transitions, sessions, attention items, or daemon runs. Trusted activation = `to == needs_you && born.is_some() && assurance ∈ {deterministic, attested} && sealed == true`. Coverage: Claude + Codex total ≥ 100 **and** each declared-supported provider ≥ 1 trusted activation; one is not maturity, it prevents claiming a provider the window never exercised. No second per-provider threshold; missing coverage narrows the claim or extends the window (Q4). |
| Q20 | **(b); nothing goes to the daemon.** The Desktop owns presentation-layer evidence: a diagnostic log that can reconstruct process start, generation N observed, generation N published, publish failed, window/tray lifecycle, process exiting — with timestamp, Desktop pid, generation, event kind, result/error — and no attention content. The window's gate is the mechanical log summary plus human spot checks: badge equals `corral needs`, no duplicate tray icon, close/reopen behaves, and the Desktop process is gone after Quit — the last never claimed by a log, since a process cannot prove its own death. |
| Q21 | **(a).** `hello` gains an additive `pid`; no privileged shutdown RPC, so possession of the local endpoint still is not privileged authorization. Uninstall: connect to the live daemon → `hello` → pid → SIGTERM → wait for exit → remove artifacts. Edges: daemon not running → continue; SIGTERM permission failure → uninstall fails with a clear error; daemon does not exit → no silent SIGKILL, M1 fails and keeps the installation; never a stale pidfile, only the pid the live `hello` just returned. |

## Questions as asked (abridged, round 2)

- **Q15** — a false-item dispute with no current item: refuse, add
  `--item` with ids surfaced in `corral needs`, or point at the most
  recently ended item?
- **Q16** — the support matrix: Apple Silicon only, pinned runner image,
  Linux x86_64 only, no aarch64 claim.
- **Q17** — GitHub Releases plus a root `install.sh` served from `main`;
  `CORRAL_VERSION` pin.
- **Q18** — version from the workspace manifest, human-pushed tag, draft
  release by the workflow, human publish after evidence.
- **Q19** — "≥ 100 across Claude + Codex": aggregate with per-provider
  presence, or a per-provider floor?
- **Q20** — tray-watchfulness evidence: human log, a Desktop diagnostic
  log plus spot checks, or reporting to the daemon?
- **Q21** — stopping the daemon at uninstall: SIGTERM to a pid `hello`
  exposes, or a shutdown RPC?

## Founder rulings, verbatim (round 2)

```text
这轮我认为已经能收口。Q15–Q21 的方向全部成立，没有需要重新打开前面裁决的冲突；但有两处我建议在文档里把声明写窄一点，主要是 Q16 和 Q17。
Q15 — false-item dispute：选 (a)
接受，而且我建议把语义钉死：

* `dispute` 只能绑定当前 active attention item。
* session 当前没有 item：非零退出，不写 dispute journal record。
* 提示：
`No current attention item. If Corral failed to surface an item, use --missed.`
* `--missed` 是另一种事实陈述："应当出现，但没有出现"，不是一个没有 item id 的 dispute。
* "曾经一闪而过、现在已经 ended"的 false positive 不允许 CLI 猜最近 item；按 session + wall-clock 时间在 evidence 阶段对照 `born` / `ended`。

这样很好地保持了 Q3 的原则：产品数据里不产生 ambiguous attribution。
我赞成不要为了这个把 item id 暴露到正常产品 UI。以后真有诊断需求，再单独做 diagnostics surface，而不是污染 `corral needs`。
Q16 — 支持矩阵：方向接受，但 Linux 声明要收窄
macOS：
macOS Apple Silicon only，最低版本 = M1 CI 实际验证的最低 macOS major。
固定 runner 后再填 `LSMinimumSystemVersion`，可以。
但我不建议文档写：
Linux x86_64 only
这仍然太宽。你现在实际证据是 Ubuntu。
M1 应该写成类似：
Ubuntu 24.04 x86_64
或者如果最终构建方式确实形成了明确的 glibc baseline，再声明那个 baseline。
`ubuntu-latest` + Ubuntu 24.04 PRoot 并不能证明 Debian/Fedora/Arch/旧 Ubuntu 都支持。
所以这里是唯一一个我认为要实质改一句声明的地方：
Linux → Ubuntu 24.04 x86_64 only for M1。
aarch64 Linux 明确 unsupported / unclaimed。
Q17 — GitHub Releases + install.sh：接受，但区分"版本 pin"和"installer pin"
方案可以：

* GitHub Releases 是 canonical distribution。
* root `install.sh` 是 bootstrap。
* 默认取 latest published、non-draft、non-prerelease。
* `CORRAL_VERSION=vX.Y.Z` 固定下载的 release artifact。

但文档最好明确：
`CORRAL_VERSION` pins the Corral release artifact, not the revision of `install.sh`.
因为：

raw.githubusercontent.com/.../main/install.sh

本身是 moving target。
M1 没必要因此再搞 installer release asset；只要你不声称这是 fully reproducible pinned installation 就没问题。
以后若要 supply-chain/reproducibility，再让：

install.sh at tag vX.Y.Z

和 artifact 一起冻结。
Q18 — version / tag / release authority：接受
这个闭环已经很干净。
我只会在 completion doc 里再钉两个 invariant：
1. tag/version 必须机械匹配

tag v0.1.0
↕
workspace.package.version = 0.1.0

不匹配直接 workflow fail，不能出 draft release。
2. draft → publish 是唯一人工 release authority
即：

human pushes tag
        ↓
CI verifies exact tagged commit
        ↓
package
        ↓
smoke
        ↓
checksums
        ↓
draft release
        ↓
release-gate evidence accepted
        ↓
human publishes

尤其好的一点是你把：
"CI 跑过"
从人工 evidence 文档里拿掉了。
机器能证明的东西让机器证明；evidence 只记录 gate judgement。
`0.0.0 → 0.1.0` 也合理。
Q19 — ≥100 across Claude + Codex：选 (a)
接受。
但 completion criteria 里必须明确计数单位，否则以后非常容易争：
≥100 trusted activations in aggregate across the declared provider set.
不是：

* journal transition 数；
* session 数；
* attention item 总数；
* daemon run 数。

而 trusted activation 就按你刚核实的现有字段：

to == needs_you
&& born.is_some()
&& assurance ∈ {deterministic, attested}
&& sealed == true

Provider coverage：

Claude + Codex total >= 100
AND
each declared-supported provider >= 1 trusted activation

这不是说 `1` 足以证明某 provider 很成熟，而只是避免：
"号称支持 Codex，但整个 cohort Codex 一次都没发生过。"
Q4 已经提供了正确处理方式：没有覆盖就缩声明或延长窗口。
不需要再造一个 `≥20/provider` 之类的第二门槛。
Q20 — tray watchfulness evidence：选 (b)
接受，而且不要送进 daemon。
Desktop 自己负责表现层证据是正确边界。
建议 Desktop diagnostics 至少能重建：

process started
generation N observed
generation N published
publish failed
window/tray lifecycle
process exiting

最好带：

timestamp
desktop process pid
generation
event kind
result/error

不需要记录 attention 内容。
最终 gate 是：
机械日志摘要
＋
人工 spot checks

* badge == `corral needs`
* 没有 duplicate tray icon
* close/reopen 行为正常
* Quit 后 Desktop process 确实消失

尤其最后一项不要假装日志能证明——进程死掉以后自己无法证明自己已经死掉，人工检查正合适。
Q21 — uninstall：选 (a)
接受。
`hello` additive 增加：

pid

是比新增 privileged RPC 小得多的协议变化，而且不会破坏：
possession of the local endpoint != privileged authorization
这个 invariant。
建议把 uninstall 行为明确成：

1. connect to live daemon
2. hello → obtain live daemon pid
3. send SIGTERM
4. wait for daemon exit
5. remove installed artifacts

几个边界也最好直接裁死：

* daemon 本来没运行 → 正常继续 uninstall。
* SIGTERM 权限失败 → uninstall fail，明确报错。
* SIGTERM 后 daemon 不退出 → 不要悄悄 SIGKILL；M1 直接 fail 并保留安装状态，比暴力杀进程更可控。
* 不用旧 pidfile；只使用刚从 live `hello` 获得的 pid。

这样 pid reuse / stale pid 的风险已经压到 M1 足够低。
Round 2 最终状态
所以我会记成：
Q15 ACCEPT (a)
Q16 ACCEPT with narrowed Linux support claim: Ubuntu 24.04 x86_64
Q17 ACCEPT, with artifact-pin vs installer-pin distinction documented
Q18 ACCEPT
Q19 ACCEPT (a), count unit = trusted activations
Q20 ACCEPT (b)
Q21 ACCEPT (a), live hello PID + SIGTERM, no automatic SIGKILL
两个动作项也保持原样：

* 删除本机 schema-1 `~/.corral/state/registry.sqlite3`
* Apple Developer Program 仍然是发布 gate 的外部依赖，不是打包 PR 的 blocker。

我看不到需要 Round 3 grill 的新洞了。现在写 `2026-09-06-m1-completion-grill.md`，然后把第一份实施计划限定在 `m1-dogfood-readiness`，边界是清楚的。
```

(Inner code fences in the founder's round-2 message are flattened to
plain lines so the block stays one fence, and typographic quotes are
normalized to ASCII; nothing else is altered.)

## Founder action items

Not agent work, recorded so nobody waits on them silently:

- delete the local schema-1 `~/.corral/state/registry.sqlite3` before the
  epoch advances (dev-era state; Q6);
- advance `STORAGE_EPOCH` to `dogfood` in a dedicated PR; nothing in the
  store gates it any more (Q6 as amended);
- decide on Apple Developer Program enrolment: an external dependency of
  the macOS distribution gate and of cohort recruiting, not a blocker of
  the packaging PR (Q9).

## What this grill leaves open

Evidence and execution only. The four plans (Q14) materialize these
rulings and decide nothing new. Open evidence: the notification probe's
result inside a real `.app` (Q12); the pinned CI runner's macOS version,
which fills the minimum-version claim (Q16); whether normal dogfood yields
100 trusted activations in 14 days (`ROADMAP.md` §9.7); the ship-time
disposition of each unresolved noise row (Q4). No ADR follows from this
record: the one Q6 implied went with the withdrawn migration baseline.

## Amendments

### 2026-09-10 — Q6 part 1 withdrawn: reset, not migration

Founder, on being shown the runner with its empty chain: 「为什么要迁移，
我们还没发布，有啥都可以直接改，不需要写迁移」, then 「重置，别搞迁移」.

What changes: `corral-state` gets no migration runner, no
`DOGFOOD_BASELINE_SCHEMA`, no schema-5 fixture, and `verify-release` has no
migration gate (this also drops the migration input from Q13 and Q14). ADR
0018 is not written. The store keeps refusing every version but its own by
name. When the registry schema changes after the epoch advance, the change
is a destructive reset the founder approves under `AGENTS.md` §Durable
state, and the evidence windows that depended on the discarded data
restart; nothing is silently reinterpreted. Q6 parts 2 and 3 stand: the
founder deletes the schema-1 store by hand, startup never deletes a
database, and the advance is a human-only dedicated PR. Why: the epoch is
`dev`, the chain is empty, and the remaining M1 plans do not touch the
registry schema; a runner with nothing to run is speculative
infrastructure. If a migration is ever wanted, the change that needs one
writes the first, as `corral-state::schema` already says.

### 2026-09-11 — Q16: the minimum macOS is the deployment target, not the runner's major

Founder, on being told the release runner would be pinned to `macos-15`
because `macos-14` is marked deprecated, and that Q16 would then make the
claim 15.0: 「runner是15，不影响编译支持14」.

What changes: `MACOS_MINIMUM` in `scripts/package` — `MACOSX_DEPLOYMENT_TARGET`
and `LSMinimumSystemVersion` — is 14.0, the deployment target the release
build compiles against; the release workflow verifies on the pinned
`macos-15` image. The two numbers are no longer one: the runner pin says
where the tests ran, the constant says the oldest macOS the binaries are
built to run on. What stays: nothing runs the suite on macOS 14, so the
14.0 claim rests on the deployment target and on the founder's own
dogfood, not on CI; a macOS-14 failure report is a bug, not a contract
violation. Q16's Linux claim and its Intel exclusion are unchanged.
