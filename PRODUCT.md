# Corral — Product

> What Corral is and is not. Boundaries and glossary: `ARCHITECTURE.md`.
> Current-phase scope: `ROADMAP.md`. Hard rules: `AGENTS.md`.
> Derived at PR0 from `docs/history/Corral_Development_Plan_v2.0_EN.md`
> §1–3, §6, §7, §13, §17, §19 and the founder decision records
> `docs/decisions/2026-08-21-m1-decision-grill.md` and
> `2026-08-21-m1-ux-contract.md`. Where this file and the retired plan
> disagree, this file wins.

## 1. Positioning

Corral is an open-source, user-owned control center for coding-agent
sessions. A **Session is the unit of AI work** — not a chat transcript, not
a terminal pane. Transcript, runtime, artifacts, control, provider, and
execution location are facets of a Session, never its identity.

```text
Public shorthand:   Every coding agent. One place.
Product discipline: See every session. Know what needs you. Take control.
Supporting:         Work anywhere. Manage everything.
```

Multi-machine operation is expansion, not the top-level definition.

Corral never requires users to move into a new IDE, terminal, cloud account,
or hosted service. They keep Claude Code, Codex, future CLI agents, Ghostty
or any terminal, tmux, Zed or VS Code, local machines, SSH servers, GPU
boxes, and their own networks. Corral discovers, indexes, connects, and
controls work that already exists.

The same logical Session is surfaced through Desktop, Terminal/TUI, Tray,
Mobile, Web, and CLI. Changing surface never creates a new Session.

### Corral is not

A replacement IDE · a launcher only · an AI SaaS platform · a VPN product ·
a hosted device-management system · a cloud relay service.

### Core principles

No Corral account system, hosted device directory, relay infrastructure, or
hosted history. Users own their machines, identities, network paths, and
data. Coding agents are the initial wedge. The architecture never hardcodes
one provider or one runtime. Zero-workflow-change discovery is a first-class
requirement. Desktop and Terminal/TUI are both first-class surfaces —
Terminal is never reduced to a debug fallback. Installation is simple while
background persistence and network reachability stay explicitly
user-controlled.

### Product hierarchy

```text
CORE          See every session · Know what needs you · Take control
SURFACES      Desktop · Terminal/TUI · Tray · CLI
SUPPORTING    Recent transcript · History · Search · Diff · Artifacts
EXPANSION     Remote nodes · SSH · LAN discovery · Tailscale ·
              Mobile/Web · future plugins
```

A surface is not a capability. A supporting capability must improve the core
loop rather than become a competing product identity. Expansion earns its
place only after the one-machine loop is valuable.

## 2. The loop and the first magic moment

```text
Multiple Claude/Codex sessions are running
  ↓ Corral discovers them automatically
One view shows what needs you, what is working, what is done
  ↓ the user opens the right Session
Respond · continue · interrupt · resume
```

The first magic moment is not remote access, history search, or a terminal
grid:

> **I no longer hunt through terminals to find out which coding agent needs
> me.**

## 3. Control model — capability ladder

For a session Corral did not launch, "Take control" is delivered through a
fixed preference ladder. Corral always offers the highest level the
provider and version actually support; users see the resulting actions,
never the ladder (`2026-08-21-m1-ux-contract.md` §0).

1. **Live synchronized control** — Corral joins the same live provider
   session as a second synchronized interaction surface. No fork, no
   resume, no PTY takeover; the original terminal stays fully usable; one
   session, one run, one row.
2. **Structured in-place control** — structured interactions (permission
   and question responses) resolved centrally even when free input is
   unavailable.
3. **Continue in Corral** — managed continuation from the provider session:
   the same Session with a new run. Normal when the original runtime has
   exited. While it is still live, the UI discloses that the continuation
   no longer synchronizes; the left-behind branch never silently
   disappears, its pending attention is never faked as resolved, and after
   the user's explicit choice it may be muted until new activity re-enters
   it.
4. **Jump** — focus the original execution surface. An escape hatch, not
   the normal path. Routine jumping back to terminals means centralized
   control has failed.

Priority: stay in Corral > preserve the same live session > safe managed
continuation > Jump.

Corral never injects input into a terminal it does not control.

**First-response lease.** For an interaction already blocked awaiting the
user, Corral may hold routing for at most 15 seconds. Answered in Corral
within the lease, the original runtime continues. Timeout, daemon loss, or
delivery failure releases immediately to the provider's native surface.
Corral never indefinitely owns a pending interaction (AGENTS.md §Runtime
truth).

## 4. User-visible state model

> Main status is the strongest fresh claim Corral can safely make. When
> semantic evidence is insufficient, preserve authoritative runtime truth
> and degrade only the semantic dimension to Unknown. Never promote
> heuristics merely to avoid showing Unknown.

| State | Meaning |
|---|---|
| **Working** | runtime alive; reliable evidence the agent is executing; the user is not needed |
| **Needs You** | runtime alive; reliable fresh evidence the agent is blocked on user input, approval, or an answer |
| **Ready** | runtime alive; the current turn is complete; awaiting the user's review or next step |
| **Unknown** | no reliable fresh semantic claim; when runtime truth is known it is shown alongside — "Running · Status unknown" |
| **Exited** | the runtime ended; historical attention is no longer a current main state |

Assurance and freshness are internal dimensions. They never become main
states; they decide whether Corral is entitled to assert a semantic label.

A user-visible semantic state is a claim with a freshness horizon. Working
and Needs You both rot to Unknown on staleness alone — no contradiction is
required, and Needs You has no sticky privilege. Rot means: no new
notification, the old notification invalidated, the badge count falls, the
session stays in the list, runtime truth stays independently displayed, and
secondary text preserves the last reliable fact ("Last known: Needed input
45m ago"). Exited overrides a cached Needs You: the label is Exited with
"Exited before you responded" — the request is neither shown as live nor
faked as answered.

## 5. Actions

UI verbs: **Open** · **Respond** · **Continue in Corral** · **Jump**.

| State and capability | Primary | Also |
|---|---|---|
| Needs You, live control available | Respond (Allow / Deny / Answer) | Open · Jump |
| Needs You, continuation only | Jump — the honest resolution path | Open · Continue in Corral (with fork disclosure) |
| Working, live control | Open, full live interaction | Jump |
| Working, observe only | Open, observe | Jump |
| Ready, alive | Open, view the result; with live control the conversation continues directly | Continue in Corral, disclosed, when only continuation is available |
| Unknown, degraded | Open / inspect | Enable Integration · Jump · view available history |
| Exited | Continue in Corral | View transcript · Archive |
| Left-behind branch | per its own state | relation text: "Original session still running" |
| Unsupported version | Open / inspect where provably safe | Jump · view available history |

A fork never promises to resolve a pending request in the branch it left
behind.

## 6. Honest degradation

- **Limited awareness.** Integration disabled or a failed safe merge shows
  "Running · Status unknown · Limited awareness" — execution truth is
  preserved and only the semantic dimension degrades. Never "Working ⚠
  unverified". Heuristic evidence may serve secondary metadata, ranking,
  debug, and discovery correlation; never a main state, never a
  notification.
- **Safe-merge failure** is an exception path: never overwrite the user's
  configuration, enter Limited awareness honestly, ask the user to resolve.
- **Unsupported provider version** displays best-effort but sits outside
  the discovery guarantee: "Running · Limited awareness · Unsupported
  version", provably safe actions only, no semantic notifications, and a
  direction-aware call to action ("Upgrade Claude Code for full Corral
  integration" versus "This Claude Code version is not yet supported by
  Corral").
- **Identity certainty is itself a user-visible claim.** Display-merging two
  entries requires attested-or-better identity evidence. Heuristic
  correlation produces two honest rows plus a weak hint ("Possibly the same
  session"); candidate bindings stay internal. Stronger evidence arriving
  later converges the rows silently — no toast, and selection follows the
  converged row.
- **Correction is asymmetric.** Unlink ("Not the same session") is
  first-class M1 UI: it removes the wrong binding, restores independent
  entries, revokes derived control eligibility, never deletes provider
  history, and never kills a runtime. Manual link is not in the normal M1
  UI — it is CLI/debug only and warns that the assertion may let Corral
  control the runtime. It is never a lightweight drag-to-merge gesture.
- **List unit.** Normally one Session is one row. Truly divergent live
  branches expand to one row per independently actionable branch, with
  secondary relation text only when lineage is reliably known. No tree or
  DAG UI in M1.

## 7. Notifications

- A notification is emitted once when a new attention item becomes
  actionable. Persistence of the same item never re-notifies. A genuinely
  new item — a new request, a new Ready turn — may notify again. No
  recurring reminders and no time-based snooze in M1. Needs You is the
  urgent class; Ready is the normal completion class.
- Eligibility: attested-or-better evidence only. Heuristic evidence never
  notifies; unsupported versions never notify; known provider noise is
  suppressed. The standard is zero avoidable false Needs You notifications,
  measured at the user-visible outcome.
- Notifications are projections of current fact, not a historical archive.
  Resolution — including the user answering in the original terminal —
  exit, rot, and supersession invalidate the old notification; replacement
  is preferred and withdrawal is the fallback. Invalidation itself never
  rings.
- The badge counts **unacknowledged attention items**, not currently
  blocked sessions. Tray groups them as "Needs You n · Ready m". Viewing a
  Ready session acknowledges it. Viewing a Needs You session does not:
  resolution clears it, or the user explicitly acknowledges the current
  alert, which clears the badge while the list honestly keeps showing Needs
  You. A new attention item re-arms both notification and badge.
  Acknowledgement is held by `corrald` and is consistent across surfaces.
- **Watchfulness equals tray presence.** Closing the Desktop window does not
  stop Corral: the tray persists and watching continues for observed and
  managed sessions. Quitting the tray explicitly ends watchfulness; if
  managed sessions continue, warn once — "N sessions will continue running.
  Corral will no longer notify you when they need attention." `corrald` may
  outlive the tray for runtime ownership, but a live daemon is not Corral
  watching. The visible tray icon is the explicit, ongoing, revocable
  background presence.

## 8. Terminology law

> Expose user facts and actions, not architecture vocabulary.

- **User-visible**: Session (the only exposed domain noun); the five main
  states; Limited awareness; Enable / Disable Integration; Open · Respond ·
  Continue in Corral · Jump; origin facts only when reliably known
  ("Launched from Corral", "Running outside Corral", "Found in VS Code") —
  never a guessed terminal host; branch relation text.
- **Internal only**: Observed, Managed, Run, Binding, Assurance, Evidence,
  AttentionItem, CorralSessionId, capability rung.
- **Dead words**: "Adopt"; "Finished" as a main state; "Take control" as a
  UI verb — it stays in the north star only.

## 9. First run and new sessions

The New Session dialog offers provider (Claude Code / Codex), working
directory (recent plus browse), and optional CLI arguments under Advanced,
remembering the last choice. Corral does not wrap provider-owned model,
permission, or policy configuration. Sessions are born managed and open in
Corral; there is no peer "open in external terminal" option.

The first-run list shows live sessions plus recent resumable sessions
reliably discovered from supported provider history, each offering Continue
in Corral. Corral never shows an empty first screen when identifiable
history exists, and never fabricates sessions to avoid an empty state.

Integration is disclosed once, lightly: "Corral integrates with Claude Code
and Codex so it can discover sessions, understand their status, and let you
respond from Corral." — [Got it] [Integration Settings], never [Enable]
[Skip].

Pre-existing live sessions warm up honestly: "Running · Status unknown" with
a one-time explanation, "Status is limited until new activity arrives from
this session." No promise of imminent status, no heuristic pre-fill.

## 10. Provider support

Supported means the latest stable release of a provider CLI plus the
previous tested release, carried by a version matrix, fixtures, and
integration tests that Corral maintains. The matrix begins as a dated
first-party record — `docs/references/2026-08-27-pr5-claude-code-hook-matrix.md`
for Claude Code — and becomes a `verify-release`-owned task before the M1
release: a one-time evidence document is not a permanent release gate. Within that matrix, every live
session must be discovered regardless of terminal host, including tmux;
systematic blind spots are release blockers. Outside it, Corral degrades
honestly rather than guessing.

Corral's integration is installed and enabled by default with the normal
installation and is transparently disclosed; settings offer per-provider
Disable Integration, which enters Limited awareness. Existing user and
third-party hooks are preserved; merge ambiguity fails safe. Uninstall
removes only Corral-owned changes and promises no byte-for-byte restore
(`2026-08-21-m1-decision-grill.md` §1).

## 11. Boundaries

M1 non-goals (`ROADMAP.md` carries the phase-scoped list): full-text history
search UI, full history-library UX, rich artifact browser, tmux-class
workspace and split feature sets, third-party plugin runtime or
marketplace, Tailscale, SSH onboarding, mobile and web, cloud relay,
enterprise permissions, universal semantic approvals.

Identity boundaries — reopening these requires a fresh product case, and the
existence of session, event, and control primitives is not one:

- **Fleet and worktree orchestration** (one-click N agents, N worktrees,
  task fleets) is not a Corral feature.
- **Agent-to-agent orchestration and pipelines** (artifact passing, chained
  triggers, automated review pipelines) is not a Corral feature.
- The Attention Engine serves coding-agent sessions only. Corral is not a
  generic notification hub — no Slack, email, CI, GitHub, or calendar
  sources.
- Transcript analysis beyond the minimal semantic processing attention
  requires is out: no session summaries, quality scoring, token or cost
  analytics, daily reports, productivity analytics.

M1 proves a session-first coding-agent control plane, not an agent
operating system.

Product-decision priority:

```text
real high-frequency user control needs
    > Corral's own product principles
    > category signals from major platforms
    > implementation and UX references from independent projects
```

> Keep working however you already work. Corral will find the sessions, tell
> you what needs attention, and let you continue from anywhere.
