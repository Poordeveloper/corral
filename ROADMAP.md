# Corral — Roadmap

> What the current phase allows. The current phase is a scope boundary
> (AGENTS.md §Product invariant). What Corral is: `PRODUCT.md`. Boundaries
> and invariants: `ARCHITECTURE.md`.
> Derived at PR0 from `docs/history/Corral_Development_Plan_v2.0_EN.md` §16,
> §17 and the founder decision records in `docs/decisions/`. Where this file
> and the retired plan disagree, this file wins.

## 1. Current phase

**M0 — foundation. PR1 landed.** Nothing beyond the PR0–PR9 sequence
below is in scope; nothing in M2 or later is solved inside an M1 task.

## 2. What M1 must prove

Three claims of different kinds — they are not ranked against each other,
they fail differently (`2026-08-21-m1-decision-grill.md` §1):

| Claim | Kind | Failure means |
|---|---|---|
| **B — managed runtime**: Corral reliably manages the sessions it launches | mandatory foundation | M1 is incomplete: delay and fix. Never a kill signal |
| **A — observed aggregation**: Corral also sees and controls sessions it did not launch | the primary differentiated bet | the only kill-class item |
| **C — attention fidelity**: what Corral says needs you is true | trust and quality bar | do not ship |

> Corral must reliably manage sessions it launches, and its reason to exist
> is that it also sees sessions it did not launch.

## 3. Implementation sequence

```text
PR0  repository governance; canonical verify scripts; benchmark-ledger
     maintenance rule; canonical PRODUCT / ARCHITECTURE / ROADMAP split
     out of the development plan
PR1  corrald walking skeleton; local IPC; lazy activation;
     singleton / stale-endpoint semantics;
     protocol hello / version / capabilities;
     corral ping / list
     (no ghost wire surface: the live-stream and durable-event
     vocabulary stays ARCHITECTURE prose until the phase that serves
     the behaviour gives it a wire representation)
PR2  CorralSessionId; SessionBinding; evidence / assurance model;
     SQLite with durable semantic event log + command receipts;
     idempotent client-supplied command ids;
     needs-input request + actionable-status vocabulary;
     resume lineage semantics (ADR 2)
PR3  PTY/process ownership in corrald; authoritative VT state;
     terminal snapshot + sequenced deltas; resize ⇒ snapshot epoch;
     advisory exclusive/shared lease seam;
     corral new -- bash; corral attach; detach / reattach (ADR 3)
PR4  minimal TUI — list / new / attach / switch.
     The first surface a person uses daily, and the first build that can
     be dogfooded. Every session reads Unknown until PR5 supplies
     attested evidence, which is the honest answer, not a gap
PR5  Claude managed sessions; launch-scoped hook injection;
     NO global config mutation (ADR 4)
PR6  Codex managed sessions; launch-scoped hooks;
     the second provider validates the Provider abstraction
PR7  externally launched Claude/Codex discovery;
     managed global hook integration (merge/version/uninstall/lock;
     atomic backfill-before-overwrite writes);
     unsafe binding degrades to read-only
PR8  daemon-side Attention Engine; versioned screen-detection manifests
     + PTY-activity evidence;
     CLI/TUI surfacing the five-state model (PRODUCT.md §4) plus the
     recent-resumable list;
     the full See → Know → Control loop
PR9  GPUI Desktop — the first graphical session/attention/control surface
     (entity-per-terminal; custom Element; embedded/standalone modes;
     pinned gpui rev). May begin once PR5 lands; see the Desktop bar
     below
```

PR7 carries two release gates at once — discovery coverage and safe
coexistence with the user's existing hooks — and is therefore the highest-
risk point in the schedule.

**The Desktop bar.** No Desktop work begins before session identity,
runtime ownership, terminal streaming, and control are demonstrable in the
TUI, and before attested attention evidence exists (PR5). The bar protects
one thing: the daemon's semantic model must not be shaped by a graphical
surface. A TUI already exercising identity, streaming, and control proves
that, so the bar does not additionally wait for the Attention Engine — but
it does wait for PR5, because a Desktop opening onto a screen of Unknown
would be rebuilt as every later phase adds meaning to render. The
five-state projection lands in PR8, and the TUI renders it inside that
phase; the in-flight Desktop workstream integrates the same projection
before PR9 merges. Each surface pays the extension once, rather than at
every phase.

Accepted in `docs/decisions/2026-08-22-surface-sequencing.md`.

### Scheduled ADRs

```text
ADR 1  corrald activation: endpoint location, singleton claim,
       stale-endpoint recovery, idle exit                    → PR1
ADR 2  resume lineage: Session outlives process              → PR2
ADR 3  terminal snapshot format: ANSI replay + seq deltas    → PR3
ADR 4  hook delivery: shim → endpoint → corrald;
       versioning; fail-open budget                          → PR5
ADR 5  platform scope: Windows deferral + re-entry trigger   → PR0 (accepted)
ADR 6  provider hook integration policy                      → PR0 (accepted)
```

### Spikes

```text
S1  VT serialization — select the emulator by proving the chain
    PTY bytes → VT → authoritative state → ANSI snapshot → client parser
    → identical screen, across scrollback, resize, alternate screen,
    cursor state, OSC title/color, colors, wide chars, Unicode,
    query/reply, and snapshot restore. No emulator is committed before
    this closes.                                        → ADR 3 / PR3
S2  Hook payload verification — Claude/Codex session identity
    (session_id / transcript_path) stability across resume, verified
    first-party against current CLI versions. Scope extended by the
    strategy grill: a real-world settings corpus including other tools'
    hooks, a merge-ambiguity taxonomy, and the fail-safe trigger set.
                                                        → PR5, PR7
S3  Per-provider live-join channel census — which channels can carry
    live synchronized or structured in-place control (Claude IDE/MCP
    channels, hook decision-hold, remote-control surfaces; Codex
    app-server, notify), including per-provider proof of reliable
    return-after-lease, the admission condition for structured in-place
    control.                                            → PR7, PR8
```

The GPUI integration spike is not on the critical path and runs shortly
before PR9.

## 4. M1 scope

Initial providers: Claude Code and Codex.

**See** — automatically discover active and existing Claude/Codex sessions;
unify observed and managed sessions under one Session identity; show
provider, project hints, runtime location, and recency without making any of
them identity.

**Know** — reliable Working / Needs You / Ready / Unknown / Exited
semantics; tray attention count and notifications; recent transcript
inspection sufficient to understand why a Session needs attention.

**Control** — create a managed Session; attach or open the correct runtime;
send input; interrupt; provider-native resume; deterministic runtime binding
for Corral-launched work; native terminal control; and the capability ladder
of `PRODUCT.md` §3 for sessions Corral did not launch.

**Surfaces** — Desktop session/attention view; tray grouping "Needs You n ·
Ready m" (PRODUCT.md §7), notifications, quick open and create; a minimal
TUI (list / needs / new / attach / switch / control); CLI equivalents;
one-command install delivering Desktop, CLI/TUI, and `corrald`; default
Local Mode with no login service, no listener, no discovery broadcast.

**Platform** — macOS and Linux; host-OS execution domain (`ARCHITECTURE.md`
§9).

After PR9, M1 completion work: tray, packaging, and one-command install.
These are not part of PR0–PR9.

## 5. Release gate

M1 ships only when all hold (`2026-08-21-m1-decision-grill.md` §1):

```text
14 consecutive days of normal dogfood use
>= 100 trusted Needs You transitions across Claude + Codex
zero avoidable false Needs You notifications, measured at the
  user-visible outcome, with known provider noise normalized or
  suppressed
no systematic missed states in supported hooked flows
no release-critical test quarantine outstanding
```

Systematic blind spots inside the supported version matrix are release
blockers. Failing to discover sessions outside that matrix is not a contract
violation.

Evidence windows count only after the storage epoch advances to `dogfood`,
and restart if the data behind them is discarded (AGENTS.md §Durable state).

Success criterion:

> A user running several coding agents starts opening Corral instead of
> hunting through terminals to find what is running and what needs
> attention.

## 6. Kill and reconsider criteria

Only the A claim is kill-class.

```text
cohort   5 qualified external daily coding-agent users on the full
         experience
window   4 weeks after M1
verdict  A is unproven if fewer than 3 of 5 users repeatedly use
         observed-session actions on at least 3 separate working days
```

The full-experience cohort excludes users who disabled integration and users
degraded by a safe-merge failure. Active-disable rate and merge-failure rate
are tracked separately and never mixed into the behavioral criterion; if
either is high enough that the full experience cannot generally be
delivered, that is an independent delivery-model failure, and A is not
thereby falsified.

**Validity floor**: the verdict is valid only if the user's primary provider
actually delivered at least structured in-place control — Needs You
resolvable inside Corral — during the window. Delivery limited to
continuation-only voids the verdict and triggers an S3 and
provider-integration re-examination, not a kill.

> Continuation is good enough to avoid a dead end; structured in-place
> control is the minimum to judge whether Corral deserves to exist; live
> synchronized control is the experience we actually want.

If A fails while B is healthy, stop and re-underwrite the runtime thesis
separately. Healthy managed-runtime usage is evidence, not permission to
move the goalposts, and there is no silent pivot into a terminal-multiplexer
competitor.

Anti-masking clause: if the loop is not valuable, do not hide the problem
behind history, remote, or mobile features.

## 7. Not M1

Not release gates: full-text history search and its UI · full history-library
UX · rich artifact browser · tmux-class workspace and split feature sets ·
third-party plugin runtime, permission system, sandbox, marketplace, or
stable external plugin ABI · Tailscale · SSH remote onboarding · mobile and
web · cloud relay · enterprise permissions · universal semantic approvals.

Avoid in M1: a full IDE or editor · a worktree-orchestration platform ·
provider UI reconstruction · cloud accounts · hosted sync · Corral relay
infrastructure · distributed full-text search across machines · advanced
RBAC · forced workflow migration · turning Corral into a history library ·
copying an IDE's chat/session hierarchy into the core model · enabling a
login daemon or listener by default · turning M1 into a tmux rewrite ·
shipping a plugin framework because extension seams exist.

Permanent identity boundaries — fleet/worktree orchestration, agent-to-agent
pipelines, generic notification hub, transcript analytics — are in
`PRODUCT.md` §11.

## 8. Later milestones

```text
M2  supporting depth      historical transcript browsing; SQLite/FTS5
                          search; broader resume/history coverage; diff and
                          file-change summary; lightweight artifacts; richer
                          reason-for-attention; structured approval UI;
                          ergonomics driven by real usage
M3  remote node proof     SSH bootstrap/tunnel; direct protocol between
                          nodes; node identity; pairing; remote runtime
                          control; lightweight remote history metadata;
                          on-demand transcript fetch; Remote Node Mode
                          enablement and service lifecycle
M4  network UX            mDNS discovery; device appearance; Tailscale
                          detection; trust UX; reconnect and liveness
                          hardening; visible reversible background behavior
M5  mobile / web          responsive frontend; Tauri packaging; QR pairing;
                          attention and control loop; diff/result view;
                          terminal fallback; protocol compatibility gates;
                          network-recovery behavior
M6  expansion             NativeResume / ContextHandoff / RuntimeMove as
                          distinct operations; additional providers;
                          further connectivity integrations; a plugin
                          system only after stable CLI/RPC/event semantics,
                          two real internal extension use cases, and a
                          formal trust-model ADR
```

M2 versus M3 ordering is decided on M1 evidence.

## 9. Open evidence questions

Not founder judgement — measurement and spikes
(`2026-08-21-m1-decision-grill.md` §5):

1. S1 emulator selection; S2 hook payload and safe-merge corpus; S3
   live-join channel census.
2. Discovery ground truth: a coverage-audit harness spawning sessions across
   terminal hosts including tmux, which operationally defines "systematic
   blind spot".
3. Provider noise catalog: the baseline of known false-signal patterns that
   defines "avoidable", collected during dogfood.
4. Delivery-health baselines: real disable and merge-failure rates,
   thresholds deliberately unset until data exists.
5. tmux and multiplexer process-tree attribution fidelity.
6. Coexistence durability: do Corral-owned hook entries survive providers
   rewriting their own configuration files?
7. Gate feasibility: does normal dogfood produce 100 trusted transitions in
   14 days? If not, extend the period — the rule is fixed, the data pending.
8. Cohort recruiting for the kill window; must start before it opens.
9. Freshness-rot thresholds and the "recent resumable" window.
