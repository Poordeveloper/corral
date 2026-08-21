# Corral Development Plan v2.0

> v2.0 does not expand the v1.9 product scope. It freezes **Corral Architecture v1** after the Orca engineering review: provider-native hooks are authoritative for session identity/resume, with agent state decided by ranked, freshness-gated evidence; the PR0–PR8 implementation sequence and its ADR schedule are recorded; M1 platform scope is fixed to macOS + Linux; the corrald crash/upgrade commitment is stated; terminal snapshot/delta and wire-compatibility semantics are fixed. The Orca review itself is preserved at `docs/references/orca-mobile-remote-report.md`.
>
> (v1.9 formalized the engineering-governance benchmark: GPUI/Rust/Tokio/SQLite direction, Engineering Workflow, root AGENTS, canonical verification, focused PR/AI contribution discipline, and dimension-specific references. That content is unchanged.)

## 1. Product Positioning

Corral is an open-source, user-owned control center for coding-agent sessions.

Corral treats a **Session as the unit of AI work**, not as a chat transcript or terminal pane. Transcript, runtime, artifacts, control, provider, and execution location are facets of a Session, not its identity.

Public-facing positioning:

> **Every coding agent. One place.**

Core product discipline:

> **See every session. Know what needs you. Take control.**

Supporting message:

> **Work anywhere. Manage everything.**

Multi-machine operation is an important expansion capability, not the top-level product definition.

Corral should not require users to move their development workflow into a new IDE, terminal, cloud account, or hosted service.

Users should be able to keep working with:

- Claude Code
- Codex
- OpenCode and future CLI agents
- Ghostty / terminal / tmux
- Zed / VS Code / other editors
- local machines
- SSH servers
- GPU boxes
- Tailscale or other user-owned networks

Corral discovers, indexes, connects, and controls the work that already exists.

The same logical Session can be surfaced through Desktop, Terminal/TUI, Tray, Mobile, Web, and CLI. Changing surface does not create a new Session, and neither provider nor execution location defines Session identity.

Corral is a **session-first, multi-surface system**, not a desktop app with auxiliary commands. Desktop and Terminal/TUI are both first-class work surfaces; Tray is a lightweight persistent attention surface; Mobile is an intervention surface. Every surface talks to the same `corrald` and the same Session identity model.

### Non-goals

Corral is not:

- a replacement IDE
- a coding-agent launcher only
- an AI SaaS platform
- a VPN product
- a hosted device-management system
- a cloud relay service

### Core principles

- no Corral account system
- no Corral-hosted device directory
- no Corral-provided relay/network infrastructure
- no hosted user session history
- users own their machines, identities, network paths, and data
- coding agents are the initial product wedge
- architecture must not hardcode one provider or one runtime
- zero-workflow-change discovery is a first-class product requirement
- Desktop and Terminal/TUI are first-class surfaces; Terminal must not be reduced to a debug/fallback UI
- installation should be simple, while background persistence and network reachability remain explicitly user-controlled

### Product hierarchy

Corral must keep four layers separate:

```text
CORE
  See every session
  Know what needs you
  Take control

SURFACES
  Desktop
  Terminal/TUI
  Tray
  CLI

SUPPORTING CAPABILITIES
  Recent transcript
  History
  Search
  Diff
  Artifacts

EXPANSION
  Remote nodes
  SSH
  LAN discovery
  Tailscale
  Mobile/Web
  Future plugins
```

A Surface is not a separate product capability. A supporting capability must improve the core loop rather than become a competing product identity. Expansion features earn their place only after the one-machine core loop is valuable.

---

# 2. Product Wedge and Demo

Corral's first release exists to prove one daily-use loop:

```text
Multiple Claude/Codex sessions are running
  ↓
Corral discovers them automatically
  ↓
One view shows Needs You / Running / Done
  ↓
User opens the right Session
  ↓
Continue / send input / interrupt / resume
```

The first magic moment is not remote access, history search, or a terminal grid:

> **I no longer hunt through terminals to find out which coding agent needs me.**

The first demo should therefore be local and brutally simple:

```text
Ghostty: Claude · rustdesk · Running
Terminal: Codex · corral · Running
VS Code terminal: Claude · game · Needs You

        ↓
Corral Tray: Needs You 1
        ↓
Open Corral
        ↓
Claude · game
"Allow this command?"
        ↓
Take control
        ↓
Running
```

For a session Corral did not launch, **Take control** is realized through a capability ladder (§6): preferred is live synchronized control — Corral joins the same live provider session as a second interaction surface while the original terminal stays fully usable; then structured in-place responses; then a disclosed managed continuation (same CorralSession, new run); Jump to the original terminal is the escape hatch, not the normal path. Corral does not inject input into a terminal it does not control.

Remote and mobile later extend this exact loop:

> **The same Session and attention model continue when the user leaves the desk.**

They are multipliers of the core product, not substitutes for proving it.

---

# 3. Product Experience References

Corral should selectively learn from existing products without inheriting their product model.

## LocalSend — discovery

Useful idea:

> Open the app and nearby devices simply appear.

Corral should make machines visible without requiring IP-address or port knowledge.

Discovery does not imply trust.

## AirDrop — trust and pairing

Useful idea:

> Discover first, explicitly trust once, authenticate automatically later.

Corral should use local node identities and QR/fingerprint-based pairing.

## VS Code Remote SSH — remote-machine adoption

Useful idea:

> Use an existing SSH environment and make the remote machine feel local to the product.

Corral should understand `~/.ssh/config`, existing keys, jump hosts, and custom ports, then bootstrap `corrald` automatically.

## VS Code Agents / Sessions — category and Session-model signal

VS Code is useful here less as an implementation template and more as evidence of a broader product shift:

> **A Session is becoming a first-class unit of AI development work, not merely chat history.**

Useful ideas:

- model Session as a work unit rather than a transcript container
- expose the same Session in multiple UI surfaces
- treat status, workspace, harness, and file changes as Session attributes
- distinguish Active / Completed / Archived lifecycle states
- use a session list as an agent-first work surface
- separate Session identity from local/background/remote execution

Do not inherit:

- VS Code's IDE/project/workspace boundary
- Copilot-specific session or harness semantics
- Chat/Session hierarchy that only makes sense inside one IDE

Corral should extend this model from one IDE to:

> **one control plane across all coding agents, terminals, and machines.**

## Wake — history implementation reference

Wake is a very new personal project. Corral should study it seriously, but **Wake must not become a source of product direction**. Its main value is implementation and UX learning for history/search, not deciding Corral's product north star.

Useful parts:

- provider history-format knowledge
- fixtures and parser behavior
- SQLite/FTS5 experience
- transcript browsing/search interaction ideas

Do not inherit:

- Wake's overall architecture
- Wake's Session model
- scanner/index coupling
- database schema
- the product assumption that a history library should be the primary entry point

Principle:

> **Wake is an implementation reference, not a product compass.**

## Herdr — runtime mechanics and extension architecture

Useful runtime parts:

- PTY ownership
- process supervision
- persistent terminal state
- input/resize
- agent detection
- agent status
- provider-session detection
- live server/runtime handoff

Herdr's plugin architecture also deserves deliberate study. Its strongest ideas are architectural rather than marketplace-specific:

- a plugin is an ordinary executable package, not a language-specific in-process ABI
- a declarative manifest defines metadata and entrypoints
- the host owns installation, manifest validation, invocation context, events, keybindings, panes/surfaces, and logs
- plugins own implementation language, dependencies, files, config, and durable state
- plugins call the host through the normal CLI/socket API instead of a separate plugin SDK
- manifest entrypoints include actions, startup hooks, events, panes, and link handlers
- compatibility is declared explicitly with a minimum host version
- config/state directories are separated from managed plugin source
- GitHub repositories can remain the distribution unit; a marketplace can be only an index rather than a package host

Herdr's current security/trust model is also worth studying, but it is **not a committed Corral choice**. Herdr treats third-party plugins as trusted local code running with the user's OS authority; plugin capabilities are not a strong sandbox boundary. That model is simple and developer-friendly, but Corral must reassess it before exposing third-party plugins.

For now, Corral only commits to learning the **out-of-process + manifest + stable host API** ideas. It must not copy Herdr's pane/workspace ontology, and it must not prematurely hard-code "the full CLI equals plugin authority" as the long-term security model.

If Corral later exposes plugins, the extension model should be Session-centric. Candidate extension points include:

```text
SessionAction
AttentionHook
ProviderIntegration
HistorySource
ArtifactRenderer
NotificationSink
DiscoveryProvider
TransportProvider
```

Phase 1 (M0/M1) preserves these extension seams only. It **does not implement a plugin runtime, plugin manager, marketplace, dynamic ABI, plugin permission system, sandbox, or stable third-party plugin API**. The external plugin contract should be decided only after Corral's own CLI/RPC/event model and real extension needs have stabilized.

Corral should own its runtime layer long-term. Herdr is a mature implementation source, not the product boundary.

A detailed source-level engineering review of Herdr's runtime, with per-subsystem reuse verdicts, is kept at `docs/references/herdr-runtime-report.md`. Because Herdr is Rust on a corrald-congruent stack, several subsystems are absorb/port candidates rather than design-only references.

## Orca — mobile and remote engineering

Useful parts:

- QR pairing
- E2EE session establishment
- WebSocket/RPC behavior
- reconnect and liveness handling
- Wi-Fi/cellular and foreground/background recovery
- protocol-version compatibility
- terminal subscribe/resubscribe behavior
- mock runtime/server testing
- mobile connection diagnostics

Do not inherit Orca's IDE/worktree/tab/leaf product model, Electron desktop stack, or full headless runtime as Corral's default runtime.

A detailed engineering review of Orca's mobile/remote implementation, with per-area port-vs-reference verdicts, is kept at `docs/references/orca-mobile-remote-report.md`.

---


## Current technology-choice status

```text
Desktop        GPUI             FROZEN
Runtime        Rust + Tokio     FROZEN
TUI            Rust             FROZEN
Storage        SQLite           FROZEN
Mobile         Tauri 2          current choice; validate details in M5
Plugin model   undecided        DEFERRED
Remote crypto/transport details later decision
```

`FROZEN` means the roadmap does not spend M0 re-running framework competitions. Integration spikes still validate how the chosen technology should be used. GPUI work studies Zed and validates Corral's terminal/diff/Tray/corrald boundaries rather than comparing Electron or Tauri again.


# 4. Core Domain Model

## Session = AI Work Unit

Corral's core object is not a transcript, terminal, or provider-native conversation. It is a logical unit of AI work:

```text
Session
├── Identity
├── History / Transcript
├── Context
├── Runtime / Execution
├── Control
├── Artifacts / File changes
└── Attention state
```

Rules:

- Transcript is a facet, not the Session itself.
- Provider is a facet/binding, not Session identity.
- Execution node/location is a facet/binding, not Session identity.
- Desktop/Mobile/Web/CLI are surfaces, not separate Sessions.
- Future NativeResume, ContextHandoff, and RuntimeMove are distinct operations and must not be collapsed into one vague `resume`.

## Provider-neutral core

Core names:

```text
Provider
Session
Run
Message
Event
Artifact
```

Avoid core types such as:

```text
CodingAgent
CodingSession
AgentAdapter
```

A provider may expose different capabilities. Corral should not assume every provider offers the same control surface.

Example capabilities:

```text
history
search
live_status
create
send_message
resume
interrupt
terminal
artifacts
structured_approval
```

`structured_approval` is optional.

The universal concept is:

> **Attention / Input Required**

not provider-specific semantic approval UI.

---

# 5. Session Identity and Facets

A Corral Session is not canonically "terminal bytes" or "structured events".

It is a logical identity with independently available facets.

```text
                 CorralSession
                      │
          ┌───────────┼────────────┐
          │           │            │
      History       Runtime     Structured
       Facet         Facet       Control?
          │           │            │
     provider ID    PTY/pane     provider API
     history path   terminal     approval/chat
```

## Primary key

Use a globally unique Corral-generated ID:

```text
CorralSessionId = UUID
```

Do not use provider session IDs as Corral's primary key.

Do not make `(node_id, session_id)` the logical identity. `node_id` scopes external bindings; it is not part of the logical session identity.

## Binding edges

External identities are bindings attached to the Corral Session:

```text
ProviderSessionBinding
RuntimeBinding
TerminalBinding
HistoryBinding
```

Every binding should record at least:

```text
corral_session_id
node_id
kind
provider/runtime
external_id
created_at
provenance
assurance
evidence_source
observed_at
```

## Binding assurance

Use discrete assurance levels instead of a generic floating confidence score:

```text
Deterministic   # corrald spawned/owns the runtime; identity holds by construction
Attested        # live provider-native evidence proves the binding
                # (e.g. a hook event carrying the exact provider session
                # identity, corroborated by an observed process)
Manual          # user explicitly linked
Heuristic       # cwd/time/process/history correlation only
```

Only `Deterministic`, `Attested`, or `Manual` bindings may drive cross-facet control.

`Heuristic` bindings may create suggestions but must not silently attach history to a live runtime in a way that enables control. Whether a heuristic match came from provider history files ("claimed") or from cwd/time correlation ("inferred") is recorded as evidence detail, not as a separate assurance level.

Assurance is re-evaluated when evidence changes; it is not a one-time stamp. A binding's evidence (`evidence_source`, `observed_at`) is what justifies its current assurance.

## Durable state model (SQLite)

Corral persists **Corral-owned semantic facts** with a per-session **durable semantic event log** — the log is **not the system of record for all state**:

```text
SQLite
├── sessions            # current projection / fast reads
├── bindings            # current binding graph
├── session_events      # Corral-owned durable semantic events,
│                       # per-session monotonic seq; projections
│                       # committed in the same transaction
└── command_receipts    # client-supplied command ids / idempotency

provider history files  # remain the provider history source of truth
live runtime state      # remains corrald's live truth (advisory,
                        # never persisted as fact)
```

The event log records only semantic facts Corral must order, replay, and keep consistent — e.g. `SessionCreated`, `BindingAdded`, `BindingConfirmed`, `RunAttached`, `RunDetached`, `CommandAccepted`. It records **none of**: PTY bytes, raw hook events, provider transcripts, derived status. Clients resume durable streams with a per-session `after` sequence cursor. Mutating commands accept client-supplied ids; exact reuse returns the same receipt (retry-safe writes).

Why now: retrofitting an event log under a CRUD store later is a full storage migration (OpenCode paid exactly this cost); adding it from scratch is a small increment. Why not further: a generic event framework is not the product.

## Corral-launched sessions

Binding must be deterministic from the start:

```text
Corral allocates CorralSessionId
        ↓
runtime creates exact PTY/process
        ↓
record runtime binding
        ↓
launch provider
        ↓
obtain/verify provider-native session ID
        ↓
record provider binding
        ↓
history appears and joins by provider-native identity
```

Provider IDs do not need to equal Corral IDs.

## Externally launched sessions

If the runtime provides an authoritative provider-session reference, Corral may auto-link it to matching history.

If Corral only has heuristic evidence such as:

- same provider
- same cwd
- similar start time
- recent history-file mtime

then Corral must not silently create a control-capable binding.

The UI may show a reversible suggestion:

```text
Possible history match
[Link]
```

Use the terms **link / unlink**, not merge. Corral does not merge or destroy provider data.

---

# 6. Observed vs Managed Sessions

Corral must support both workflows.

```text
Session
├── Observed
└── Managed
```

## Observed

Launched outside Corral and discovered later.

Possible capabilities:

- history
- live status
- runtime attachment if deterministically identified
- terminal control if a compatible runtime owns it

## Managed

Launched through Corral's runtime.

Expected capabilities:

- deterministic session identity
- persistent runtime
- terminal control
- create/send/interrupt/resume
- reliable attention state

Corral must not require users to adopt Managed Sessions before the product is useful.

## Session outlives process (resume lineage)

A CorralSession is not a process and not a single provider run. When an agent process exits and the same provider session is resumed (provider-native resume or Corral-driven resume), the result is the **same CorralSession** with a new run/binding recorded — never a new CorralSession:

```text
CorralSession A
├── run/binding #1   (process #1, exited)
└── run/binding #2   (process #2, resumed)
```

This is a core Corral invariant, fixed by ADR 2 (PR2). NativeResume / ContextHandoff / RuntimeMove (M6) remain distinct operations; only NativeResume continues the same CorralSession by definition.

## Controlling an Observed session (capability ladder)

Control of a live Observed session follows a fixed preference ladder (founder decision record: `docs/decisions/2026-08-21-m1-ux-contract.md`). Corral always offers the highest capability the provider/version actually supports; users see only the resulting actions, never the ladder.

1. **Live synchronized control** (preferred): Corral joins the same live provider session as a second synchronized interaction surface. No fork, no resume, no PTY takeover — the original terminal stays fully usable, and the user answers Needs You or sends input from Corral. One session, one run, one list row.
2. **Structured in-place control**: where only structured interactions (permission/question responses) can be routed, Corral still resolves those centrally. For an interaction already blocked awaiting the user, Corral may hold a first-response lease of at most 15 seconds; timeout, daemon loss, or delivery failure fails open immediately to the provider-native surface. A provider that cannot reliably return the interaction after the lease does not qualify for this level.
3. **Continue in Corral**: when live control is unavailable but provider resume is, Corral creates a managed continuation — the same CorralSession with a new run (the resume-lineage path above). With the original runtime exited this is the normal continuation. With the original runtime still live, the UI must disclose that the continuation no longer synchronizes with the original surface; the left-behind branch never silently disappears from the list, its pending attention is never faked as resolved, and after the user's explicit choice it may be muted from attention until new activity re-enters it. Both live branches are independently actionable list entries.
4. **Jump**: focus the original execution surface. An escape hatch, not the normal control path.

Corral never injects input into a terminal it does not control. "Take control" remains the product promise in the north star; it is not a UI verb or a runtime transition name (UI verbs: Open / Respond / Continue in Corral / Jump).

## Session Lifecycle / Archive Semantics

Execution state and user-organization state must stay separate:

```text
Execution state:
Running / NeedsInput / Blocked / Done / Unverifiable / Exited

Library state:
Active / Archived
```

Rules:

- `Done` means the work/execution completed; it does not mean the user wants it hidden.
- `Archived` removes a Session from the normal active surface while preserving identity and history.
- `Deleted` explicitly removes Corral-owned metadata/index associations; it does not imply deletion of provider-owned history.
- For Observed Sessions, deleting inside Corral must not modify Claude/Codex source data by default.

---

# 7. Attention Model

The primary home-screen question is:

> **What needs me now?**

Normalize live state into product-level concepts such as:

```text
Running
NeedsInput
Blocked
Done
Unverifiable
Exited
```

Important rules:

- `Needs You` is the highest-priority product aggregation and should not be buried under workspace/machine/provider grouping.
- `Unverifiable` is a first-class state: disconnect, timeout, or loss of reachability must not be inferred as `Exited`.
- Corral should consume authoritative runtime/provider status when available.
- Corral should not duplicate a runtime's agent-state detector by parsing terminal ANSI if the runtime already provides structured status.

## Evidence authority

Agent status is **evidence with source and freshness**, not an oracle. Every status observation records at least:

```text
Evidence
├── source        # which detector produced it
├── observed_at
└── assurance / confidence
```

Evidence sources are ranked; a higher source wins only while its evidence is fresh:

```text
provider-native hook/event            # highest-assurance evidence while fresh
    ↓
explicit runtime/provider signal
    ↓
in-band signal (e.g. OSC status sequences)
    ↓
terminal/screen detection
    ↓
history/process heuristics
```

Rules:

- Lower layers are fallback evidence, not equal authority.
- Hook/provider evidence splits by kind. For **identity/resume** it is authoritative — both production references (Orca, Herdr) agree. For **turn state** it is one weighted evidence source, never the load-bearing one: Herdr ran hook-driven state in production and rolled it back to identity-only hooks (late/ambiguous lifecycle events reviving idle panes); Orca sustains hook state only behind a large per-vendor normalization layer.
- PTY output activity is the default authority for `Working`; pattern/hook evidence refines it.
- The attention engine must remain fully functional on screen + PTY-activity evidence alone; hook state transitions are additive evidence, never a dependency.
- Screen-detection rules are versioned manifest data (Herdr-style TOML with `version` / `min_engine_version`), loaded at runtime so agent-UI drift is fixable without a binary release. M1 ships the engine + manifest format; a remote manifest-update channel is deferred.
- No cached evidence may permanently outrank fresher contradicting runtime evidence: if a hook last said `Working` but the process is gone, state degrades to `Exited` / `Unverifiable` — the stale hook claim does not win.
- Status restored from persistence with no live signal since is marked unconfirmed and treated as immediately stale by freshness gates until a live event confirms it.
- Hooks drop events (no receiver up, interrupts, crashes); the model must tolerate missed transitions rather than assume a complete event stream.
- **Attention is computed in `corrald` only.** Desktop/TUI/Tray render the daemon's attention state; no client derives its own (N clients independently re-deriving attention is a documented reference failure mode).
- Attention/status vocabulary is structured from day one — never a bare boolean:

```text
AttentionItem
├── reason
├── source
├── freshness
└── action?          # extensible actionable payload
                     # (e.g. {provider, title, label, link})

NeedsInputRequest    # reserved answerable entity
├── id
├── session_id
├── provider/tool context
└── allowed_actions?
```

  M1 answers needs-input by attaching the terminal and using the provider's own TUI; structured approval UI (approve once / always / reject-with-feedback) is M2. The vocabulary is reserved now because attention booleans cannot be upgraded into answerable requests compatibly.

Structured semantic approvals are an enhancement:

```text
NeedsInput
   ├── generic terminal/input fallback
   └── StructuredApproval?  # provider capability
```

The product remains functional even when a provider does not expose semantic approval metadata.

---

# 8. Runtime Architecture

## Direction

Corral should own the runtime layer.

Do not make a standalone Herdr process a permanent architectural dependency.

Recommended implementation strategy:

> Absorb/selectively port mature Herdr runtime mechanics into a Corral-owned runtime, then simplify the model around Corral's needs.

This is different from rewriting terminal behavior from scratch.

### Keep / reuse from Herdr where useful

- cross-platform PTY mechanics
- terminal state
- process supervision
- detach/persistence behavior
- input and resize
- agent detection
- agent status
- provider-session detection
- proven edge-case handling

### Do not inherit by default

- Herdr's pane/workspace/tab **product ontology**
- the assumption that a terminal multiplexer is Corral's product center
- standalone-product configuration surface
- Herdr-specific wire types in Corral core

### Runtime ambition

Corral is not positioned as a "better Herdr", but that does not place a ceiling on its runtime, terminal, persistence, agent-detection, or control capabilities. Wherever those capabilities serve the Corral Session/Attention model, Corral should aim to equal or surpass specialized runtime tools.

Principle:

> **Do not inherit Herdr's product model. Do not inherit Herdr's limitations either.**

Terminal/TUI may support splits, attach/detach, persistent PTYs, session switching, agent status, and deep keyboard-first work. The difference is that terminal/pane objects remain part of a Session's RuntimeFacet rather than becoming Corral's top-level product ontology.

Target structure:

```text
corrald
├── session registry
├── history providers
├── attention aggregation
├── runtime
│   ├── PTY/process supervision
│   ├── terminal state
│   ├── agent detection/status
│   └── provider-session detection
├── protocol server
└── identity
```

One node, one primary daemon.

## Runtime abstraction

Keep a narrow internal boundary so the design can be validated against another implementation if useful:

```text
RuntimeBackend
├── CorralRuntime       # production/default
└── experimental adapters
```

An Orca runtime adapter may be used as a temporary architecture spike to validate neutrality, but Orca should not be a production dependency or M1 requirement.

## Terminal state and streaming semantics

`corrald` owns the authoritative VT screen state for every managed terminal (one bounded emulator per session) and answers terminal queries (DA/DSR/OSC) when no client is attached, so unattached agents never stall on an unanswered query.

Wire semantics, fixed from the first local protocol (ADR 3, PR3):

```text
subscribe
   ↓
snapshot @ sequence N        # ANSI replay serialization of the authoritative
                             # buffer — not a structured cell grid; the wire
                             # must not encode any client's rendering model
   ↓
sequenced raw deltas N+1 …
```

Recovery has exactly one path in M1:

```text
gap / decode failure / client too slow
   ↓
discard incremental state
   ↓
request a fresh snapshot
```

Additional wire rules fixed by ADR 3 (source-evidence driven):

- **Input encoding is client-side**: the client encodes keystrokes/mouse into escape sequences using its replica emulator's live mode bits (APP_CURSOR, bracketed paste, mouse modes); the daemon accepts raw input bytes. The wire stays dumb.
- **Resize ⇒ new snapshot epoch**: resize reflows the emulator, so replaying pre-resize bytes into a resized replica diverges. A resize invalidates replay; clients discard and take a fresh snapshot. Sub-cell size changes are ignored; pending resizes coalesce.
- Scrollback depth and snapshot extent are **wire-contract numbers** (reference points: 10k lines default / 100k max).
- Daemon-sourced PTY bytes are replayed **unmodified** — no LF/CRLF or other munging between daemon log and client parser.
- The emulator choice is deferred to spike S1 (§16); snapshot minting requires an emulator that can serialize its state to ANSI (or a per-epoch raw byte log) — this capability, not familiarity, decides the selection.

Explicitly deferred until remote/mobile requires them: ACK/credit flow control, remote backpressure, viewport claiming, paired parking, and any large binary opcode surface. M1 keeps bounded in-memory scrollback only — no persisted scrollback.

## Provider integration — native hooks

Provider-native hooks/events (Claude Code hooks, Codex notify/events) are **authoritative for provider session identity and native resume** in M1, and one weighted evidence source for agent state under §7's ladder. Screen/terminal detection + PTY activity remain first-class state evidence, not an afterthought: Orca demonstrates hook-driven state in production (behind a large normalization layer), while Herdr tried it and rolled back to identity-only hooks — Corral's attention semantics must not depend on hook state transitions (see `docs/references/orca-mobile-remote-report.md` and `docs/references/herdr-runtime-report.md`).

Two integration modes, deliberately sequenced:

```text
Managed sessions (PR4/PR5)
  launch-scoped hook injection
  per-launch settings/env pointing at corrald
  NO mutation of the user's global agent configuration

Externally launched sessions (PR6)
  managed global hook configuration
  install / version / merge / uninstall with lock and owner identity
  if safe coexistence with the user's existing hook config cannot be
  proven, degrade to read-only heuristic discovery — never risk the
  user's own configuration
```

Hook delivery is a second versioned wire protocol (shim → local endpoint → corrald), fixed by ADR 4 before PR4. Hard invariants:

```text
corrald down → shim fails open within milliseconds
the shim never starts corrald
the shim never blocks, slows, or breaks the user's agent
a broken Corral must never make Claude/Codex worse
```

Hook events fired while corrald is not running are lost by design; external sessions are re-discovered on the next corrald start via history/process scan.

## Installation and daemon lifecycle

### One-command installation

The macOS product contract is:

```bash
brew install --cask corral
```

One command installs:

```text
Corral.app
corral        # CLI / interactive TUI
corrald       # runtime / node daemon binary
```

Users should not need to understand or separately install Desktop, CLI/TUI, and daemon components. Linux uses native distribution mechanisms with the same one-install-action principle. Windows is deferred beyond M1 (see §16 platform scope) and, when it arrives, preserves the same principle.

### Zero-background-by-default

**Installing the daemon binary does not mean enabling an always-on login service by default.**

Default Local Mode:

```text
install Corral
    ↓
no login service registration
no network listener
no mDNS/Tailscale discovery advertisement
    ↓
first launch of Corral.app or corral
    ↓
lazily start/attach to corrald
```

This avoids surprising ordinary local users with an unsolicited login daemon or network listener merely because they installed a developer tool.

If Managed Sessions/PTYs still need persistence, closing Desktop/TUI must not terminate that work; `corrald` may continue hosting the runtime. When there are no clients, no managed work, and Remote Node Mode is disabled, `corrald` may exit.

### Remote Node Mode — explicit opt-in

Only after the user explicitly enables:

> **Remote Access / Make this machine available to Corral**

should the machine become an always-available Corral Node. This may:

- register a macOS LaunchAgent or equivalent per-user service;
- start `corrald` automatically at login;
- enable the user-approved LAN/Tailscale listener;
- enable mDNS/peer discovery;
- allow paired devices to reach the node while Desktop/TUI is closed.

Remote Mode must be reversible and should remove the corresponding persistence/network behavior when disabled.

### Client recovery

In either Local Mode or Remote Node Mode, any local client should be able to:

```text
connect corrald
   ↓ unavailable
recover/start corrald
   ↓
attach
```

Users should never need to launch `corrald` manually.

### Runtime continuity

Committed M1 guarantee hierarchy:

```text
closing Desktop/TUI/Tray     never terminates managed work
corrald planned upgrade      live handoff (Herdr-style FD/state transfer);
                             if takeover fails, the upgrade ABORTS and the old
                             corrald keeps serving — never proceed-and-drop
corrald unexpected crash     M1 does NOT guarantee managed-session survival
```

Riders that keep "no guarantee" honest:

- **No-lying reconciliation**: on the next start after a crash, every session that was live is re-verified against the OS and reported `Exited` (with cause when determinable) or `Unverifiable` — never silently dropped, never shown as stale `Running`. Persistence (PR2) must be written to support this.
- **Crash never kills work corrald does not own**: externally launched sessions hold their own PTYs; a control-plane crash leaves them untouched, and they are re-bound on restart.
- Live handoff is a **platform capability, not a protocol guarantee**; the wire never promises it.

A separate runtime-host/PTY-keeper process is explicitly rejected for M1 and reconsidered only if implementation evidence forces it (Orca's daemon endpoint-ownership history is the cautionary reference).

---

# 9. History and Search

Provider-owned files/databases remain the source of truth.

```text
Provider history
      ↓
HistorySource
      ↓
HistoryParser
      ↓
Normalized history/session records
      ↓
HistoryIndex?
```

Suggested conceptual separation:

```text
HistorySource
HistoryParser
HistoryWatcher
SessionResumer
HistoryIndex
```

The parser should not know whether SQLite exists.

## Index

M1:

```text
SQLite + FTS5
```

Use it only as a derived `HistoryIndex`, not as Corral's universal system database.

Reasons:

- mature
- rebuildable
- simple transactional behavior
- good enough for first-release scale
- FTS5 trigram is useful for code, substring search, and CJK

Possible later backend:

```text
TantivyHistoryIndex
```

Only after benchmark/search-quality evidence justifies it.

## Incremental parsing

Append-only formats should use source cursors where possible:

```text
SourceCursor
- file_id
- offset
- size
- mtime
```

Full reparse only on truncation, replacement, file identity change, or parser anomaly.

## Remote history

Do not synchronize full transcripts by default.

Aggregate lightweight metadata where useful:

```text
session id
title
provider
repo/cwd
host
created/updated time
status
message count
```

Fetch full transcripts on demand from the origin node.

Privacy principle:

> **Session history stays on the machine where it was created unless the user explicitly moves it.**

---

# 10. Connectivity Model

Corral owns application identity, pairing, and session control.

Corral does not own the user's network infrastructure.

Separate:

```text
Discovery
Authentication
Authorization
```

and also separate:

```text
Discovery
Direct Transport
Remote Bootstrap/Tunnel
```

## Direct transport

Examples:

```text
Local
LAN
Tailscale
future user-owned network paths
```

At the application layer they all carry the Corral Protocol.

## SSH

SSH is not the same semantic category as LAN/Tailscale.

Treat it primarily as:

```text
RemoteBootstrap / Tunnel
```

Use SSH to:

- parse existing `~/.ssh/config`
- use existing keys
- support jump hosts/custom ports
- bootstrap/install/start `corrald`
- establish an initial Corral connection or tunnel where useful

Desktop can use SSH directly. Browser/mobile clients generally cannot assume SSH, so a node intended for mobile access still needs a mobile-reachable network path such as LAN, Tailscale, or user-provided ingress.

## LAN discovery

Use mDNS/Bonjour for unauthenticated discovery only:

```text
_corral._tcp.local
```

Advertise only minimal node/service metadata.

Do not advertise session/project/history contents before trust.

## Tailscale

Tailscale is the first recommended remote network option, not an architectural dependency.

Corral should detect existing Tailscale connectivity and present devices rather than VPN terminology.

Tailscale membership does not equal Corral authorization.

---

# 11. Node Identity and Pairing

Every `corrald` generates a local node keypair:

```text
node_private_key
node_public_key
node_id
```

First pairing should use an explicit trust flow, with QR/fingerprint support.

A pairing payload may contain:

```text
node/public identity
endpoint hints
short-lived pairing capability/token
protocol version
```

Do not encode reusable raw passwords.

After pairing, use mutual authenticated cryptography at the Corral application layer even when the underlying transport is already encrypted.

Orca's mobile E2EE/reconnect implementation is a useful reference for the engineering problems, but Corral should implement the protocol in its shared Rust client/core, preferring a standard Noise-style construction (e.g. the `snow` crate) over porting Orca's custom E2EE design. Orca's value here is the checklist of problems — pairing offers, key pinning, replay protection, transcript binding, reconnect — not the cipher construction itself.

Initial permissions should remain coarse:

```text
view sessions
send input
terminal
files
structured approval  # only if provider supports it
```

Avoid early RBAC complexity.

---

# 12. Corral Protocol

The protocol must serve multiple clients:

```text
GPUI Desktop
Terminal/TUI
Tray
Tauri Mobile
Web Client
CLI
```

Core client/protocol/identity/crypto logic should live in shared Rust crates where practical.

Suggested layout:

```text
crates/
├── corral-core
├── corral-protocol
├── corral-client
├── corral-identity
├── corral-crypto
├── corral-history
└── corral-runtime
```

## Extension seams

Corral core modules should be replaceable/extensible behind internal contracts from the beginning, without turning those contracts into a public plugin ABI prematurely.

Initial internal seams:

```text
Provider
HistorySource
RuntimeProvider
DiscoveryProvider
TransportProvider
NotificationSink
ArtifactRenderer
```

Rules:

- keep extension boundaries out-of-process-friendly
- keep CLI/RPC semantics usable without the Desktop client
- events should carry stable semantic meaning, not UI implementation details
- pass explicit invocation context rather than letting extensions scrape global state
- keep extension-owned config/state separate from Corral-managed source/cache
- treat compatibility and unknown/unsupported fields as normal version-skew conditions
- do not load arbitrary native dynamic libraries into `corrald` in M1
- do not promise a stable external plugin API before the product model itself is stable
- do not assume that possession of a local RPC endpoint should imply future plugin authority
- do not assume that same-OS-user identity should imply trusted plugin/client identity

### Plugin security — deferred decision, not Phase-1 implementation

Phase 1 records the threat model and irreversible constraints only. It does not implement a third-party plugin security stack.

The principle to preserve now is:

```text
transport identity
!= application identity
!= authorization
```

A Unix socket or named pipe may later prove that the peer belongs to the current user, but that alone must not imply full Corral control authority.

Before third-party plugins are exposed, make a separate ADR based on real use cases. Candidate directions include, but are not limited to:

```text
A. Trusted native plugin
   ordinary executable
   user explicitly trusts the code

B. Out-of-process + scoped capability RPC
   actor/provenance + per-operation authorization

C. Sandboxed extension
   WASM / restricted runtime / explicit hostcalls

D. Hybrid
   trusted native plugins + sandboxed marketplace extensions
```

This plan **does not preselect the final model**. In particular, do not build a capability broker, WASM runtime, plugin sandbox, permission UI, or marketplace signing in M0/M1 merely to defend against hypothetical future malicious plugins.

The long-term target remains that a future `corral-plugin.toml` is layered on top of mature semantic CLI/RPC/event boundaries instead of forcing a core rewrite.

## Version compatibility

Plan for desktop/daemon and mobile release skew from the beginning. The hello handshake carries, both ways, from PR1:

```text
PROTOCOL_VERSION
MIN_COMPATIBLE_CLIENT_VERSION
MIN_COMPATIBLE_SERVER_VERSION
capabilities              # flat string set, e.g. terminal.stream.v1
```

Rules:

- An absent compatibility field means **unknown**, never a known negative; the evaluator treats it as protocol 0 with explicit, documented kill-switch semantics.
- Breaking changes bump the version; additive optional fields/methods stay backward compatible. A shipped opcode/frame/discriminant number is permanent and is never reused, even if the feature behind it is removed.
- **Unknown-tolerant wire (invariant)**: wire protocols must tolerate additive evolution. Unknown methods, notifications, fields, and discriminants must each have an explicit compatibility behavior. The implementation technique — string method IDs, extensible envelopes, `Unknown(raw)`, `serde(other)`, capability negotiation — is chosen per protocol shape, not mandated globally.
- Defined unknown-input policy: unknown method → explicit error; unknown notification → ignore and count; unknown binary opcode → drop and count (silent drops otherwise present as hangs).
- Every wire type ships a **future-input test**: decoding succeeds against a fixture containing an unknown field and an unknown discriminant, with the defined behavior asserted.
- New stream opcodes, and semantics old peers cannot interpret, are gated behind capabilities. Changing published *content* can break old clients even with no schema change; review it as a wire change.
- **Recovery splits by stream kind** and one model is never generalized to the other: terminal streams recover only by discarding local state and requesting a fresh snapshot; durable session-event streams resume by per-session `after` sequence-cursor replay (see §5 Durable state model).
- An incompatible client/server pair fails clearly rather than silently corrupting behavior.

Cross-version tests that dial the previous release against the working tree are introduced once independently upgrading clients/nodes actually exist (M3+); the rules above are enforced from PR1.

---

# 13. First-class Local Surfaces

Corral is not a desktop app plus auxiliary commands. Multiple peer surfaces access the same Session system.

```text
                         corrald
                            │
          ┌─────────────────┼─────────────────┐
          │                 │                 │
     GPUI Desktop      Terminal/TUI          Tray
          │                 │                 │
 overview / rich UI    deep work / PTY   attention / quick actions
```

## Desktop

Technology:

```text
Rust + GPUI
```

Desktop remains native even though mobile/web use a web frontend. Reasons:

- desktop terminal quality matters
- keyboard-heavy workflows
- high-frequency all-day use
- rich diff/history/artifact presentation
- native Rust integration
- direct fit with the Corral runtime/protocol stack

Do not switch desktop to Electron/Tauri merely to reuse mobile UI.

Primary Desktop surfaces:

```text
Needs You
Running
Recent
History
Search
Devices
```

Session detail may expose:

```text
Transcript
Activity
Files
Diff
Terminal
```

Machine, workspace, and provider are primarily filters/attributes, not the top-level hierarchy. Default ordering should answer `What needs me now?`.

## Terminal / TUI

Terminal/TUI is a first-class work surface, not a fallback. Users can run:

```bash
corral
```

and enter an interactive TUI that can:

- show Needs You / Running / Recent;
- create Managed Sessions;
- attach/switch sessions;
- attach to persistent PTYs hosted by `corrald`;
- split panes;
- send input / interrupt / resume;
- inspect history/diff/status quickly;
- detach while work continues.

M1 does not need to clone Herdr's full workspace/tab product model. The first release only needs a strong `list / needs / new / attach / switch / control` loop.

Principle:

> **Corral is session-first, not terminal-first — but Terminal is first-class.**

## CLI

Non-interactive CLI shares the same client/core with the TUI:

```bash
corral ls
corral needs
corral new
corral attach <session>
corral search <query>
```

This serves scripting, automation, and agent-native control.

## Tray

Tray is Corral's lightweight persistent attention surface, especially useful when the main window is closed but the user still wants awareness of agent work.

Initial Tray responsibilities may include:

```text
Needs You count
Running count
recent status changes
open/focus session
new session
quick interrupt / resume where safe
notifications
open Desktop
Remote Node Mode on/off/status
quit / stop-background-work choices
```

Tray must make background behavior **visible and controllable**. It must not silently promote Local Mode into an always-on network service.

On macOS, closing the main window may leave the Tray app running; true Quit semantics and Managed-runtime persistence must be explicit. Remote Node Mode login persistence remains an explicit opt-in user service and is not enabled merely because Tray exists.

# 14. Mobile and Web Client

## Technology

Use:

```text
Tauri 2
+ responsive web frontend
+ shared Rust client/protocol/crypto/identity
```

The web frontend should be reusable for:

```text
iOS Tauri app
Android Tauri app
self-hosted/browser client where appropriate
```

## Mobile role

Mobile is an intervention client, not a full desktop IDE.

Primary surfaces:

```text
Needs You
Running
Done
Session detail
Send prompt
Interrupt
Resume
Start session
Quick diff/result
Terminal fallback
```

Terminal on mobile may use xterm/WebView because it is a fallback surface rather than the main all-day interaction.

Desktop terminal quality must not be reduced for renderer unification.

## Mobile reliability requirements

Learn from Orca's production experience and explicitly handle:

- QR pairing
- foreground/background transitions
- Wi-Fi ↔ cellular transitions
- reconnect backoff
- half-open sockets
- liveness detection
- subscription replay
- terminal resubscribe
- protocol-version skew
- diagnostic logs
- mock server/client testing

## Networking constraint

A mobile/web client still needs a reachable path to `corrald`.

Supported product philosophy:

```text
LAN
Tailscale
user-provided ingress/private network
```

Corral does not promise zero-config Internet reachability without infrastructure and does not operate its own relay.

---

# 15. Engineering Governance — control vibe coding with workflow, not a giant prompt

Corral treats AI-assisted development as an engineering process from M0.

> **AI lowers the cost of writing code. It does not lower the cost of review, understanding, verification, or long-term ownership.**

Detailed process lives in `docs/ENGINEERING_WORKFLOW.md`. Root `AGENTS.md` contains only hard invariants, scope rules, routing, verification, and high-frequency AI failure guards.

Repository governance baseline:

```text
AGENTS.md
PRODUCT.md
ARCHITECTURE.md
ROADMAP.md
CONTRIBUTING.md
docs/ENGINEERING_WORKFLOW.md
docs/adr/
docs/plans/
scripts/verify
scripts/verify-fast
```

Core discipline:

- one task/issue -> one explicit goal -> one coherent semantic scope;
- focused feature PRs; no unrelated refactors;
- cross-module owner-boundary repair is allowed when required by the same violated invariant;
- architecture changes require ADRs before implementation;
- search for existing concepts before introducing traits/modules/state/protocol fields;
- no speculative abstractions or cosmetic single-use helpers;
- comments explain non-obvious WHY/invariants/ownership/lifecycle, not ordinary code;
- reviews are concise and findings-first; `No material findings.` is valid;
- substantive changes prefer fresh-context review;
- tests prove observable contracts, with integration/compatibility/lifecycle emphasis for agent/session/runtime/protocol work;
- `./scripts/verify` is the completion gate;
- multi-agent/shared checkouts forbid blanket staging/reset/clean/stash operations that can damage other work.

Change-size numbers are review pressure, not lints: re-evaluate staging near ~800 non-mechanical changed lines; complex logic should usually be staged below ~500 when safe. A larger coherent invariant is allowed when splitting would be unsafe and the reason is explicit.

Rules grow from repeated expensive mistakes and durable invariants rather than a prewritten catalog of hypothetical preferences. External AI-contribution policy belongs in `CONTRIBUTING.md`.

# 16. Roadmap

## Frozen implementation sequence (Architecture v1)

The M0/M1 work below is executed in this PR order:

```text
PR0  repository governance; canonical verify scripts; benchmark-ledger
     maintenance rule; split canonical PRODUCT / ARCHITECTURE / ROADMAP
     out of this plan
PR1  corrald walking skeleton; local IPC; lazy activation;
     singleton / stale-endpoint semantics;
     protocol hello / version / capabilities;
     live-stream vs durable-event vocabulary (after-cursor replay);
     corral ping / list
PR2  CorralSessionId; SessionBinding; evidence / assurance model;
     SQLite with durable semantic event log + command receipts;
     idempotent client-supplied command ids;
     needs-input request + actionable-status vocabulary;
     resume lineage semantics (ADR 2)
PR3  PTY/process ownership in corrald; authoritative VT state;
     terminal snapshot + sequenced deltas; resize ⇒ snapshot epoch;
     advisory exclusive/shared lease seam;
     corral new -- bash; corral attach; detach / reattach (ADR 3)
PR4  Claude managed sessions; launch-scoped hook injection;
     NO global config mutation (ADR 4)
PR5  Codex managed sessions; launch-scoped hooks;
     second provider validates the Provider abstraction
PR6  externally launched Claude/Codex discovery;
     managed global hook integration (merge/version/uninstall/lock;
     atomic backfill-before-overwrite writes);
     unsafe binding degrades to read-only
PR7  daemon-side Attention Engine; versioned screen-detection manifests
     (Herdr-style TOML) + PTY-activity evidence;
     CLI/TUI Needs You / Running / Recent;
     full See → Know → Control loop provable without Desktop
PR8  GPUI Desktop — first graphical session/attention/control surface
     (entity-per-terminal; custom Element; embedded/standalone modes;
     pinned gpui rev)
```

Scheduled ADRs, each closing before or inside its PR:

```text
ADR 1  corrald activation: endpoint location, singleton claim,
       stale-endpoint recovery, idle exit                    → PR1
ADR 2  resume lineage: Session outlives process              → PR2
ADR 3  terminal snapshot format: ANSI replay + seq deltas    → PR3
ADR 4  hook delivery: shim → endpoint → corrald;
       versioning; fail-open budget                          → PR4
ADR 5  platform scope: Windows deferral + re-entry trigger   → PR0
```

Spikes, each closing before its consumer:

```text
S1  VT serialization spike — select the emulator (ghostty-vt vs
    alacritty + own serializer vs wezterm-term) by proving the chain
    PTY bytes → VT → authoritative state → ANSI snapshot → client
    parser → identical screen, across: scrollback, resize, alternate
    screen, cursor state, OSC title/color, colors, wide chars,
    Unicode, query/reply, snapshot restore.
    No emulator is committed before this closes.       → ADR 3 / PR3
S2  hook payload verification — Claude/Codex session identity
    (session_id / transcript_path) stability across resume,
    verified first-party against current CLI versions  → PR4
```

Sequencing consequences for the M0/M1 lists below:

- The GPUI integration spike is not on the critical path (ADR 3 keeps the wire independent of any client's rendering model); it runs shortly before PR8 rather than in early M0.
- Tray, packaging, and one-command install are M1 completion work after PR8; they are not part of PR0–PR8.
- The core loop must be demonstrable at PR7 through CLI/TUI, before any Desktop work.

## M0 — Architecture / Foundation

**Repo operating system**

- derive canonical `PRODUCT.md`, `ARCHITECTURE.md`, and `ROADMAP.md` from v1.9;
- root `AGENTS.md` with hard invariants/scope/routing/verification only;
- `docs/ENGINEERING_WORKFLOW.md`;
- `CONTRIBUTING.md` with focused PR and AI-contribution policy;
- `docs/adr/` and `docs/plans/`;
- canonical `scripts/verify-fast` and `scripts/verify` entry points.

**Architecture / runtime**

- Rust workspace;
- core domain model;
- `CorralSessionId`;
- binding model + assurance rules;
- local `corrald`;
- explicit dependency direction: semantic core/protocol -> daemon/runtime; Desktop/TUI/Tray/CLI are clients and do not own runtime truth;
- local protocol + mixed-version compatibility contract;
- code-level Herdr absorption spike for PTY ownership, supervision, persistence, detection, and handoff: decide what to absorb, refactor, or discard.

**First-class surfaces**

- GPUI Desktop shell; framework choice is frozen;
- GPUI integration spike for terminal surface, live session subscription, basic diff, and Tray/window lifecycle. The spike decides how to implement GPUI, not whether to use GPUI;
- Terminal/TUI client shell;
- Tray lifecycle/attention contract;
- macOS one-command packaging contract;
- Local Mode / Remote Node Mode daemon lifecycle.

**Deferred extension architecture**

- extension-seam + plugin threat-model ADR informed by Herdr/Pi/OpenCode;
- no public plugin API, permission system, sandbox, or marketplace in M0/M1.

Goal: freeze expensive-to-reverse boundaries and establish a repository operating system that keeps AI-assisted development from continuously expanding scope.

## M1 — See / Know / Control

Initial providers:

- Claude Code
- Codex

**See**

- automatically discover active/existing Claude/Codex sessions
- unify Observed and Managed sessions under `CorralSessionId`
- show provider, project/worktree hints, runtime location, and recency without making them identity

**Know**

- reliable Running / Needs You / Done/Exited/Unverifiable semantics
- Tray attention count and notifications
- recent transcript/context inspection sufficient to understand why a Session needs attention

**Control**

- create a new Managed Session
- attach/open the correct runtime
- send input/prompt
- interrupt
- provider-native resume when available
- deterministic runtime binding for Corral-launched work
- native terminal control

**Surfaces required for the loop**

- Desktop session/attention view
- Tray: Needs You/Running, notification, quick open/create
- minimal Terminal/TUI: list / needs / new / attach / switch / control
- CLI equivalents for automation and debugging
- one-command install for Desktop + CLI/TUI + `corrald` (macOS cask; Linux native packaging, same one-action principle)
- default Local Mode: no login service, no remote listener, no discovery broadcast

**Platform scope**

- M1 targets macOS + Linux only.
- Windows is deferred by ADR 5 with an explicit re-entry trigger (user-demand evidence or a cohort requiring native support). The first Windows step is WSL2-as-a-node, reusing the Unix runtime; native ConPTY ownership comes after. The Windows continuity model is pre-decided from Herdr's production evidence: job-object child lifecycle, no live handoff — upgrades and crashes recover via snapshot restore + provider-native resume. Do not attempt an FD-style ConPTY handoff.
- Nothing Unix-shaped may leak into the protocol or domain model: endpoints — not sockets/FDs — are the wire-level concept; platform behavior stays behind platform modules.

Not M1 release gates:

- full-text history search / SQLite FTS UI
- full history-library UX
- rich artifact browser
- full tmux-class workspace/split feature set
- third-party plugin runtime / permission system / sandbox / marketplace / stable external plugin ABI
- Tailscale
- SSH remote onboarding
- mobile/web
- cloud relay
- enterprise permissions
- universal semantic approvals

Success criterion:

> A user running several coding agents starts opening Corral instead of hunting through terminals to find what is running and what needs attention.

If this loop is not valuable, do not use more history, remote, or mobile features to hide the problem.

## M2 — Supporting Depth

Add only features that make the core loop faster or more trustworthy:

- historical transcript browsing
- SQLite/FTS5 search
- better resume/history-source coverage
- diff/file-change summary
- lightweight artifacts/results
- richer session context and reason-for-attention
- TUI/Desktop ergonomics driven by real usage

Goal:

> Improve understanding and continuation of a Session without turning Corral into a history library or IDE.

## M3 — Remote Node Proof

Add:

- SSH bootstrap/tunnel workflow
- direct Corral Protocol between nodes/clients
- node identity
- pairing
- remote runtime/session control
- lightweight remote history metadata
- on-demand transcript fetch
- explicit Remote Node Mode enablement
- macOS per-user LaunchAgent / equivalent service lifecycle proof

Goal: prove the same See / Know / Control loop survives crossing machine boundaries.

## M4 — Network UX

Add:

- mDNS discovery
- LocalSend-style device appearance
- Tailscale detection/reachability
- AirDrop-style trust UX
- reconnect/liveness hardening
- visible/reversible Remote Mode and background-behavior UX

Goal:

> Users see machines, not network configuration.

## M5 — Mobile/Web

Add:

- responsive frontend
- Tauri iOS/Android packaging
- QR pairing
- Needs You / Running / Done
- prompts / interrupt / resume / create
- diff/result view
- terminal fallback
- protocol compatibility gates
- mobile network-recovery behavior

Goal:

> **The same attention/control loop still works when the user is away from the desk.**

## M6 — Expansion

Future Session continuation/movement must distinguish:

```text
NativeResume
  same provider / same native session

ContextHandoff
  transfer task/context/artifacts into another provider/session

RuntimeMove
  move or continue execution on another node/runtime
```

Do not collapse all three into one generic `resume`.

Agent-to-agent session orchestration — letting one agent inspect/create/message other Sessions — is a later research direction, not M1/M2 scope.

A public plugin system is also an expansion feature, not an early milestone. Before shipping it, Corral should already have stable CLI/RPC/event semantics and at least two real internal extension use cases. Herdr's out-of-process manifest model is the primary architecture reference, but the **plugin trust/security model remains intentionally undecided**. At that stage, make a formal ADR based on real extension needs and choose among trusted native, scoped capability RPC, sandboxed extensions, or a hybrid model.

Potential providers:

- Gemini CLI
- Copilot CLI
- OpenCode
- other agent runtimes

Potential connectivity integrations:

- Cloudflare Mesh
- Twingate
- other user-owned network products

Potential runtime experiments:

- additional runtime backends/adapters only if they validate a real need

---

# 17. Scope Discipline

Corral's long-term user breadth should not turn M1 into a platform rewrite.

Avoid in M1:

- full IDE/editor
- worktree orchestration platform
- provider UI reconstruction
- cloud accounts
- hosted sync
- Corral relay infrastructure
- mobile client
- distributed full-text search across all machines
- advanced RBAC
- forced workflow migration
- turning Corral into a Wake-style history library
- copying a VS Code Chat/Session hierarchy into the core model just because VS Code has one
- enabling a login daemon or network listener by default immediately after installation
- turning M1 into a full tmux/workspace rewrite merely to chase Herdr
- shipping a plugin framework just because extension seams exist

The product should earn expansion by making the one-machine **See → Know → Control** loop valuable first.

Product-decision priority:

```text
real high-frequency user control needs
    > Corral's own product principles
    > category signals from major platforms such as VS Code
    > implementation/UX references from new independent projects such as Wake
```

---

# 18. Architectural Summary

```text
                           Clients

             ┌──────────────┼──────────────┐
             │              │              │
     GPUI Desktop   Terminal/TUI   Tray   Tauri Mobile   Web/CLI
          │              │          │          │            │
          └────────────── Corral Protocol ───────────────────┘
                            │
                         corrald
                            │
       ┌────────────────────┼────────────────────┐
       │                    │                    │
 Session Registry       History Layer       Runtime
       │                    │                    │
 CorralSessionId       provider files      PTY/process
 + bindings            + parser/index      + agent status
       │                                         │
       └────────────── Attention Model ──────────┘
                            │
                    Running / Needs You / Done / Unverifiable
```

Remote connectivity:

```text
Direct transport:
Local / LAN / Tailscale / user network

Bootstrap/tunnel:
SSH

Application trust:
Corral node identity + pairing + authenticated protocol
```

---

# 19. Final Product Principle

Orca asks users to work inside a new agentic development environment.

Corral should make the opposite promise:

> **Keep working however you already work. Corral will find the sessions, tell you what needs attention, and let you continue from anywhere.**

That is the product boundary to protect as the implementation grows.

Reference boundaries should also remain explicit:

- VS Code validates that the **Session / Agent Control Plane category is emerging**.
- Wake is a reference for **history/parser/search implementation details**.
- Herdr is a source for **runtime mechanics and extension/plugin architecture**.
- Orca is a reference for **mobile/remote engineering**.

No single reference project should determine the Corral roadmap.

v1.9 freezes these principles:

- **Corral is session-first, multi-surface.** Desktop, Terminal/TUI, Tray, and Mobile are different surfaces over one Session system;
- **One-command installation.** One install action delivers Desktop, CLI/TUI, and daemon binary;
- **Zero-background-by-default.** Default Local Mode does not register a login service or expose a network listener;
- **Remote is explicit opt-in.** Login persistence, discovery, and remote listeners activate only in Remote Node Mode;
- **Tray makes background state visible.** Tray is an attention/quick-control surface, not a hidden path to enabling remote behavior;
- **Terminal is first-class.** Corral may match or surpass Herdr's runtime/TUI experience while Session/AI work remains the product ontology.
- **Core before breadth.** History/Search are supporting capabilities; Remote/Mobile are expansion. M1 exists to prove See → Know → Control.
- **Extension seams before plugin product.** Learn from Herdr's out-of-process manifest model, but Phase 1 preserves seams and the threat model only; it does not implement a plugin framework or preselect the final trusted-native/capability/sandbox security model.

Corral's product discipline is:

> **See every session. Know what needs you. Take control.**

Public-facing shorthand:

> **Every coding agent. One place.**
