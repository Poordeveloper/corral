# M1 UX Semantics Contract — Founder Decision Record

> Status: founder-accepted decisions from the 2026-08-21 UX-semantics grill.
> Newer than, and where in conflict superseding, older statements in
> `Corral_Development_Plan_v2.0_EN.md` (notably §2/§6 takeover semantics),
> pending reconciliation. Companion record:
> `2026-08-21-m1-decision-grill.md`.
> Routing: state model + terminology → `ARCHITECTURE.md`/`PRODUCT.md` at
> PR0; notification rules → PR7 acceptance criteria; the AGENTS.md
> first-response-lease amendment was founder-acked in this grill and is
> applied in the same change that lands this record; S3 joins the spike
> list.

## 0. Control model: capability ladder (rewrites "Take control")

For a live Observed session, the primary path is **live synchronized
control**: Corral joins the same live provider session as a second
synchronized interaction surface. No fork, no resume, no PTY takeover; the
original terminal stays fully usable; input, output, and state changes
synchronize both ways; one session, one run, one list row.

Fallback ladder (Corral always offers the highest level the
provider/version actually supports; users never see the ladder, only the
resulting actions):

1. **Live synchronized control** — full same-session interaction.
2. **Structured in-place control** — structured interactions
   (permission/question responses) resolved centrally even when free input
   is unavailable.
3. **Continue in Corral** — managed continuation from provider
   session/history: same CorralSession + new Run. Normal continuation when
   the original runtime has exited. If the original runtime is still live:
   explicit disclosure that the continuation no longer synchronizes; the
   left-behind branch never silently disappears; its pending attention is
   never faked as resolved; after the user's explicit fork choice it may be
   muted from attention until new activity, which re-enters attention.
4. **Jump** — focus the original execution surface. Escape hatch only; if
   users must routinely jump back to terminals, centralized control has
   failed.

Product priority: Stay in Corral > preserve the same live session > safe
managed continuation > Jump.

**First-response lease**: for an interaction already blocked awaiting user
input, Corral may hold routing for at most **15 seconds**. Answered in
Corral within the lease ⇒ the original runtime continues. Timeout ⇒
immediate release to the provider-native surface. corrald / hook-bridge
loss ⇒ immediate fail-open without waiting out the lease. Corral never
indefinitely owns a pending interaction. A provider that cannot reliably
return the interaction after the lease does not qualify for rung 2 —
implementations auto-downgrade.

**A-test validity floor**: the A-thesis behavioral kill test is valid only
if the user's primary provider delivered rung ≥ 2 during the evaluation
window (Needs You / structured interactions resolvable inside Corral
without returning to the original terminal). Rung-3-only delivery ⇒
capability/delivery failure: verdict void, re-examine S3/provider
integration.

## 1. User-visible state model

Collapse principle (frozen): "Main status is the strongest fresh claim
Corral can safely make. When semantic evidence is insufficient, preserve
authoritative runtime truth and degrade only the semantic dimension to
Unknown. Never promote heuristics merely to avoid showing Unknown."

Internal dimensions: execution state, attention state, assurance,
freshness. Assurance and freshness never become main states; they decide
whether Corral is entitled to assert a semantic label.

Frozen main-state vocabulary:

| State | Meaning |
|---|---|
| **Working** | runtime alive; reliable evidence the agent is executing; user not needed |
| **Needs You** | runtime alive; reliable, fresh evidence the agent is blocked on user input/approval/answer |
| **Ready** | runtime alive; current turn complete; awaiting the user's review or next step |
| **Unknown** | no reliable fresh semantic claim; when runtime truth is known, show it alongside: "Running · Status unknown" |
| **Exited** | runtime ended; historical attention is no longer a current main state |

"Finished" is banned as a main state (turn-completed vs runtime-exited are
different facts with different actions). Ready ≠ Needs You ≠ Idle ≠ Exited.

Staleness: "A user-visible semantic state is a claim with a freshness
horizon." Working and Needs You both rot to Unknown on staleness alone —
no contradiction required; Needs You has no sticky privilege. Rot: no new
notification, old notification invalidated, badge count falls, session
remains, runtime truth remains independently displayed, secondary text
preserves the last reliable fact ("Last known: Needed input 45m ago").
Timeout values are implementation tuning, not part of this contract.

Exited overrides cached Needs You: label Exited, secondary "Exited before
you responded"; the pending request's runtime no longer exists, and the
request is neither shown as live nor faked as answered.

## 2. Session action matrix

UI verbs (frozen): **Open** · **Respond** (contextual) · **Continue in
Corral** · **Jump**. "Take control" remains only in the north star; it is
not a UI verb or a runtime transition name. "Adopt" is dead.

| State × capability | Primary | Also | Notes |
|---|---|---|---|
| Needs You @ rung 1/2 | Respond (Allow/Deny/Answer) | Open · Jump | rung 2 via the 15s lease |
| Needs You @ rung 3 | Jump (honest resolution path) | Open · Continue in Corral (fork disclosure) | fork does not promise resolving the pending request |
| Working @ rung 1 | Open → full live interaction | Jump | |
| Working @ rung 2/3 | Open → observe | Jump | |
| Ready (alive) | Open → view result; rung 1 continues the conversation directly | rung 3 next turn = Continue in Corral (disclosed) | |
| Unknown (degraded) | Open / inspect | Enable Integration · Jump · view available history | |
| Exited | Continue in Corral | View transcript · Archive | same CorralSession + new Run |
| Left-behind branch | per its own state | relation text ("Original session still running / Left behind") | one actionable branch = one row |
| Unsupported version | Open / inspect (provably safe only) | Jump · view available history | no Respond, no centralized control |

Rung → experience mapping: rung 1 = Open is full live interaction; rung 2 =
observe + structured Respond; rung 3 = Continue in Corral; rung 4/5 =
Open/read where safe + Jump.

## 3. Degraded-mode behavior

- Integration disabled / merge-failed: "Running · Status unknown / Limited
  awareness" — execution truth preserved, only the semantic dimension
  degrades. Never "Working ⚠ unverified". Heuristic evidence may serve
  secondary metadata, ranking, debug, and discovery correlation — never a
  main state, never a notification.
- Safe-merge failure is an exception path: never overwrite user config;
  enter Limited awareness honestly; ask the user to resolve.
- Unsupported provider version: display best-effort but outside the
  "every" guarantee. "Running · Limited awareness · Unsupported version";
  provably safe actions only; no semantic attention notifications;
  direction-aware CTA ("Upgrade Claude Code for full Corral integration."
  vs "This Claude Code version is not yet supported by Corral."). Failing
  to discover matrix-outside sessions ≠ contract violation; systematic
  blind spots inside the supported matrix = release blocker.
- Identity: "Identity certainty is itself a user-visible claim."
  Display-merge requires ≥ Attested identity evidence. Heuristic
  correlation ⇒ two honest rows + weak hint ("Possibly the same session");
  candidate bindings stay internal. Strong evidence arriving later ⇒ silent
  convergence (no toast; selection follows the converged row).
- Correction asymmetry: **Unlink / "Not the same session" is first-class
  M1 UI** — removes the wrong binding, restores independent entries,
  revokes derived control eligibility, never deletes provider history,
  never kills the runtime. **Manual link is not in normal M1 UI**
  (CLI/debug-only, with an explicit warning that the assertion may allow
  Corral to control the runtime); never a lightweight drag/merge gesture.
- List unit: normally one CorralSession ≈ one row. Truly divergent live
  branches (fallback-3 fork while alive) expand to one row per
  independently actionable branch, with secondary relation text
  ("Continued from … / Branch of …" only when lineage is reliably known).
  No tree/DAG UI in M1. External `--fork` sessions: independent rows;
  "Forked from …" only when reliably known; never guessed.

## 4. Notification rules

- Emission: "A notification is emitted once when a new attention item
  becomes actionable. Persistence of the same attention item never causes
  repeated notifications." New attention item (new request, new Ready
  turn) ⇒ may notify again. No recurring reminders / time-based snooze in
  M1. Needs You = urgent class; Ready = normal completion class.
- Eligibility gate (from the strategy grill): attested-or-better evidence
  only; heuristic never notifies; unsupported versions never notify; zero
  avoidable false notifications; known provider noise suppressed.
- Invalidation: notifications are projections of current fact, not a
  historical archive. Resolution (including the user answering in the
  original terminal), exit, rot, and supersession invalidate the old
  notification — replacement preferred, withdrawal otherwise; invalidation
  itself never rings.
- Badge = unacknowledged attention items (not the count of currently
  blocked sessions). Tray groups "Needs You n · Ready m".
  - Ready: view = acknowledge (badge clears; the state may remain Ready).
  - Needs You: view ≠ acknowledge; resolution clears; explicit
    acknowledge-current-alert clears the badge while the list honestly
    keeps Needs You; a new attention item re-arms notification + badge.
  - Acknowledge state is held by corrald; consistent across
    Desktop/TUI/Tray.
- Watchfulness ⇔ tray/menu-bar presence. Closing the Desktop window ≠
  stopping Corral (tray persists, watching continues for observed and
  managed sessions). Quit tray = watchfulness explicitly ends; if managed
  sessions continue, warn once: "N sessions will continue running. Corral
  will no longer notify you when they need attention." corrald may outlive
  the tray for runtime ownership, but corrald alive ≠ Corral watching. The
  hook shim never starts corrald for notifications. Zero-background
  remains: the visible tray icon is the explicit, ongoing, revocable
  background presence.
- Surface roles: Desktop window = focused control surface; Tray = ambient
  watch surface; corrald = runtime/session infrastructure.

## 5. Terminology decisions

Principle: "Expose user facts and actions, not architecture vocabulary."

- User-visible: **Session** (the only exposed domain noun); the five main
  states; Limited awareness; Enable/Disable Integration; Open / Respond /
  Continue in Corral / Jump; origin facts only when reliably known
  ("Launched from Corral" / "Running outside Corral" / "Found in VS Code /
  Ghostty / …" — never guess the terminal host); branch secondary language
  ("Continued from … / Original session still running / Left behind").
- Internal only: Observed / Managed (architecture, code, diagnostics), Run,
  Binding, Assurance, Evidence, AttentionItem, CorralSessionId, capability
  rung.
- Dead words: "Adopt"; "Finished" (as a main state); "Take control" as a
  UI verb.
- Acknowledge is the semantic name; concrete button copy (Got it / Dismiss
  alert) belongs to visual design.

## 6. First-run and New Session

- New Session dialog: provider (Claude Code / Codex) + working directory
  (recent + browse) + optional CLI arguments folded under Advanced;
  remembers the last choice. No wrapping of provider-owned model /
  permission / policy config. Sessions are born Managed and open in
  Corral; no "open in external terminal" peer option.
- First-run list = live sessions + recent resumable sessions reliably
  discovered from supported provider history, with Continue in Corral
  available. Never an empty first screen when identifiable history exists;
  never fabricate sessions to avoid an empty state. Boundary frozen: recent
  resumable sessions in the normal list = M1; history browsing / search /
  filtering / timeline / archive UX = M2. The "recent" window is
  implementation tuning.
- Integration disclosure (once, lightweight): "Corral integrates with
  Claude Code and Codex so it can discover sessions, understand their
  status, and let you respond from Corral." Buttons: [Got it] [Integration
  Settings] — never [Enable] [Skip].
- Warm-up: pre-existing live sessions show "Running · Status unknown" with
  a one-time secondary explanation: "Status is limited until new activity
  arrives from this session." No promise of imminent status; no heuristic
  pre-fill.

## 7. Remaining UX spikes (implementation mechanics / visual design)

1. **S3 — per-provider live-join channel census** (determines rungs):
   Claude IDE/MCP channels, hook decision-hold, remote-control surfaces;
   Codex app-server, notify. Includes per-provider proof of reliable
   return-after-lease (the admission condition for rung 2).
2. Original-terminal presentation during the lease (provider-dependent);
   post-lease Respond reachability — hold-based channels lose central
   answering after expiry (UI grays Respond, points to Jump); native
   dual-surface channels do not. S3 annotates per channel.
3. Respond interaction forms: which structured request types each provider
   exposes → the corresponding forms.
4. Freshness-rot thresholds (Working / Needs You horizons); "recent
   resumable" window.
5. Notification replacement/withdrawal support on macOS/Linux notification
   APIs.
6. Selection/focus continuity on row convergence; visual grammar for
   Limited awareness, left-behind badges, tray grouping; final disclosure
   and warm-up copy.
