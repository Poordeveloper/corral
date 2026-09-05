# Architecture Benchmarks — evidence ledger

```yaml
read_when:
  - "Reopening or questioning an architecture decision listed in the matrix"
  - "Adding a new reference project or new subsystem evidence"
  - "Writing or revising an ADR that cites reference projects"
  - "A coding agent proposes an architecture different from the plan"
maintenance:
  - "One row per subsystem decision. Update the row when evidence or the decision changes; never delete rows — mark superseded."
  - "Every claim links a source report in docs/references/ or a commit-anchored path."
  - "New reference research merges into this file in the same PR that uses it."
```

Purpose: prevent re-litigating researched decisions. Corral learns from different
projects for different subsystems; no single repository is the product blueprint.

## Reference roles and evidence depth

| Project | Role | Evidence depth | Source report / anchor |
|---|---|---|---|
| Herdr (herdrdev/herdr @624dfd4) | PTY/runtime ownership, daemon lifecycle, detach/reattach, live handoff, runtime/client boundary, screen detection | Tier 1 — deep source review | `herdr-runtime-report.md` |
| Orca (stablyai/orca @59892a2f) | provider hooks, remote/mobile, protocol compat, reconnect/liveness, terminal streaming; cautionary: fragmented PTY ownership | Tier 1 — deep source review | `orca-mobile-remote-report.md` |
| OpenCode (anomalyco/opencode @2859603c, v1.18.19) | session/provider architecture, core/protocol/server/client layering, session ownership | Tier 1 — focused source review | agent report, 2026-08-21 (this file's matrix) |
| Zed (zed-industries/zed @91bf967e) | GPUI desktop, terminal, remote sidecar, agent/session UI, diff boundaries | Tier 1 — focused source review | agent report, 2026-08-21 |
| Pi (badlogic/pi-mono) | TUI interaction, extension/RPC seams, session files, AI coding rules | Tier 2 — targeted | agent report, 2026-08-21 |
| Codex (openai/codex, codex-rs) | Rust workspace discipline, engine/frontend boundary, protocol v1/v2, integration testing | Tier 2 — targeted (evidence quality Tier-1-grade for boundaries) | agent report, 2026-08-21 |
| CC Switch (farion1231/cc-switch v3.20.0) | tray/background lifecycle, packaging/updater, config-mutation safety, AI-PR policy | Tier 2 — targeted | agent report, 2026-08-21 |
| OpenClaw (openclaw/openclaw) | AI-heavy repo governance, review discipline, decision-doc persistence | Tier 2 — targeted, governance only | agent report, 2026-08-21 |
| Wake (iAmCorey/Wake v0.2.1) | history parsing/discovery, FTS5 search, Rust+GPUI at small scale; cautionary: provider-id-as-primary-key | Tier 2 — targeted | agent report, 2026-08-21 |

Correction recorded 2026-08-21: earlier discussion over-weighted Orca because it
was researched first. Orca remains authoritative only for its role row above.
CC Switch is NOT a desktop-UI architecture reference.

## Architecture Reference Matrix

Format per row: strongest reference(s) → observed evidence → Corral decision →
rejected alternative → confidence → remaining gap.

### 1. Session identity and lifecycle
- Strongest: OpenCode (primary), Pi (session tree, leases), Wake (counterexample), Orca (pane-identity cautionary).
- Evidence: OpenCode `ses_`-prefixed time-sortable globally-unique ids; sessions/messages/parts in SQLite; **per-session durable event log with per-aggregate monotonic seq, projections committed in the same transaction** (`packages/core/src/event.ts`); durable admission inbox separated from advisory process-local execution (`specs/v2/session.md`) — the implemented form of "session outlives process"; idempotent client-supplied command ids; parentID child sessions. Pi: JSONL tree in one file (`id`/`parentId`), exclusive/shared session leases (`PiSessionOwnershipError`). Wake: `{provider}:{native_id}` as PRIMARY KEY + path UNIQUE — workable only because read-only; exactly what Corral bans for a control plane.
- Decision (2026-08-21, founder-approved): CorralSessionId UUID primary; bindings with Deterministic/Attested/Manual/Heuristic assurance; session outlives process (ADR 2); **durable SEMANTIC event log only** — `session_events` (Corral-owned facts: SessionCreated/BindingAdded/BindingConfirmed/RunAttached/RunDetached/CommandAccepted…, per-session monotonic seq, projections in-tx) + `command_receipts` (idempotent client ids), with `sessions`/`bindings` as current projections. The log is NOT the system of record for all state: PTY bytes, raw hook events, provider transcripts, and derived status stay out of it; provider files and live runtime state keep their ownership domains. Project identity (derived) allowed only as a grouping *binding*, never a control key. Extended (2026-08-22, founder-approved, ADR 2 D6): `RunStarted`, `RunEnded`, `SessionForkedFrom` join the set; durability follows fact assurance, not object existence; every persistent projection mutation must be derivable from an accepted event; the log is append-only in seq order, with occurrence time distinct from acceptance order.
- Rejected: provider-id/path/cwd-derived primary keys (Wake shape); CRUD-only store with event log retrofitted later (OpenCode paid that migration — bridge machinery still visible in source).
- Confidence: high. Gap: cross-process execution-ownership fencing unsolved in every reference (OpenCode explicitly defers it); Corral defers to M3 with the lease seam reserved.

### 2. Provider abstraction
- Strongest: OpenCode (primary), Codex (model-provider crates), Orca (per-vendor tarpit cautionary).
- Evidence: OpenCode: Provider/Model catalog (models.dev + config merge) with typed capabilities/cost/limits/status; Agent = named config bundle (mode, permission ruleset, model, prompt); recorded-HTTP-fixture contract tests per provider protocol (`packages/llm` + `http-recorder`); permission/question as first-class answerable request entities (stable id, sessionID+toolCallID binding, once/always/reject-with-feedback, live event + list endpoint for late joiners). Orca: 4.7k-line hook listener = cost of normalizing ~20 vendors.
- Decision: provider-neutral core (Provider/Session/Run/Message/Event/Artifact per plan §4); capabilities per provider, `structured_approval` optional; Claude+Codex only in M1; **adopt the answerable needs-input request shape into the protocol vocabulary** (reserve in M1 even if UI answers via terminal); real-format fixture contract tests per provider (already an AGENTS.md rule).
- Rejected: assuming uniform provider capability; boolean-only "needs input" that cannot later carry an answerable request; >2 providers in M1.
- Confidence: high. Gap: no reference models provider capability *degradation* over version skew of the provider CLI itself.

### 3. PTY / process / runtime ownership
- Strongest: Herdr (primary), Zed remote_server (counter-model), Orca (cautionary).
- Evidence: Herdr: one server owns every PTY (thread-per-PTY poll actor), process groups / Windows Job Objects (KILL_ON_JOB_CLOSE), ghostty-vt per pane, detach/reattach + live handoff, e2e-tested (`tests/live_handoff.rs` counts /proc ptmx fds). Zed: client-owned PTYs die on disconnect — the product gap Corral closes; remote_server holds no PTYs and is killed on reconnect. Orca: 4-way split ownership → second protocol, 23-defect endpoint-ownership saga.
- Decision: one corrald owns PTYs + process lifetime + authoritative VT + runtime truth; upgrades via Herdr-style live handoff (Unix); crash = no survival guarantee + no-lying reconciliation; Windows (deferred) = job objects + restore + provider resume, **no ConPTY handoff**.
- Rejected: second permanent PTY daemon; client-owned PTYs; Electron-app-as-runtime.
- Confidence: high (three independent confirmations incl. one by counterexample). Gap closed by ADR 0003 (D6/D7): attach/resync carries up to 2,000 recent rows under an encoded budget, with honest truncation metadata. One deliberate divergence from Herdr, fixed by ADR 0007 (L2): Corral's per-PTY actor ends with the runtime it serves rather than persisting — a finished run's screen is a published value, so a daemon holding many finished sessions holds snapshots, not threads and emulators.

### 4. Daemon / client boundary
- Strongest: Codex (primary), Herdr (guardrail), OpenCode (layering + in-memory transport).
- Evidence: Codex: TUI has **zero dependency on codex-core** — consumes engine via app-server-client + app-server-protocol; boundary enforced by dependency graph + clippy `disallowed-methods`; `app-server-daemon` = shipped daemonized control plane. OpenCode v2: schema←protocol←server over core, generated clients, embedded host runs the same protocol over in-memory transport. Herdr AGENTS.md: migrating away from private UI-coupled socket; "neutral server/API names".
- Decision: corral-core / corral-protocol / corral-client / corrald / surfaces stands as planned; enforce with dependency graph (surfaces must not depend on corral-core internals) + clippy `disallowed-methods` (only corrald's state module opens the DB; only the PTY owner spawns PTYs); in-memory transport reusing the full protocol stack for tests.
- Rejected: server-rendered UI frames over a private wire (Herdr's regret); surfaces importing the engine crate (Codex proves it's avoidable at scale); embedded-daemon-by-default (OpenCode's inverse default — violates one-owner-of-truth).
- Confidence: high. Gap: none material.

### 5. Terminal state and streaming
- Strongest: Zed (two production paths) + Herdr (server-side model + serialization) + Orca (stream protocol details).
- Evidence: terminal-across-a-boundary is raw ANSI bytes into a client-side emulator in all three (Zed ssh terminals; Zed ACP display-only emulator fed by byte chunks with pending-output buffering; Herdr remote ANSI mode; Orca binary stream). Nobody syncs grids. Zed's RPC implements seq+ack+replay-buffer resume; Orca implements snapshot@seq + deltas + resync. Zed exposed the hidden risk: **alacritty has no ANSI re-serializer — snapshot minting requires an emulator that can serialize (ghostty-vt can; Herdr uses it) or a per-epoch raw byte log**. Zed: input encoding depends on client-replica mode bits; resize reflows ⇒ replay divergence.
- Decision (ADR 3): daemon-owned authoritative VT; wire = ANSI snapshot @ seq N + sequenced raw deltas; resync-by-snapshot is the only terminal recovery path; **resize ⇒ new snapshot epoch**; client encodes input bytes from its replica's mode state, daemon accepts raw bytes; scrollback depth + snapshot extent are wire-contract numbers (Zed reference: 10k default/100k max); PTY bytes replayed unmodified (no LF/CRLF munging).
- Rejected: structured grid sync (zero production precedent); server-rendered frames; delta backfill/credit flow control in M1.
- Confidence: high on wire model (triple-confirmed); high on emulator choice as of the S1 measurement, medium on the winner's maturity.
- S1 closed 2026-08-23 (`2026-08-23-s1-vt-serialization.md`): the chain was measured on 20 dimensions. `alacritty_terminal` 0.26 confirmed to have no re-serializer; `termwiz`'s terminal model is not on crates.io. **vt100 0.16.2 serializes, but drops the alternate-screen mode and all scrollback, and models no OSC at all.** **qwertty-term-vt 0.4.0 — a pure-Rust port of Ghostty's formatter — round-trips every dimension except the OSC title**, which it tracks but does not re-emit; so the Zig question the spike was to inform does not need answering. Costs measured: snapshot 424 KB at 10k lines of history, 4.29 MB at 100k, of which 5.5 KB is a per-snapshot palette; the per-epoch byte-log fallback is smaller than serialization for appending output but 243x larger for redrawing output at 10k repaints, so it cannot be the primary mechanism for a product that hosts TUIs. Recommendation: qwertty-term-vt, with its 936 `unsafe` blocks in the untrusted-input path recorded as a known risk. **ADR 3 should bound snapshot extent separately from scrollback depth** — at row 5's own reference numbers one attach ships megabytes.
- ADR 0003 accepted 2026-08-24 (`docs/decisions/2026-08-24-adr3-terminal-snapshot-acceptance.md`): qwertty-term-vt confirmed with a three-layer fuzz gate; snapshot targets up to 2,000 recent rows (experience target) under a 1 MiB encoded target and a 16 MiB hard ceiling (viewport-only overflow ⇒ typed failure); daemon retains 4 MiB/session of scrollback; no history backfill in M1 — omitted history is a fact, not a promise. All four numbers are initial policy defaults, not wire constants.

### 6. Agent status / attention
- Strongest: Herdr (state detection), Orca (hook-state at cost), OpenCode (status vocabulary + request entities).
- Evidence: Herdr v8 rolled hook-driven *state* back to identity-only hooks; remotely-updatable versioned TOML manifests + PTY-activity-as-Working-authority + freshness-merged per-source metadata. Orca sustains hook state behind a 4.7k-line normalizer; hooks carry exact provider identity (both agree). OpenCode: `idle|busy|retry{action{provider,title,label,link}}` status + permission/question requests; every client re-derives attention independently (negative evidence: N clients duplicating the state machine).
- Decision: evidence ladder with hooks authoritative for identity/resume, weighted evidence for state; PTY activity = default Working authority; **attention computed in corrald only — FROZEN** (clients render, never derive); versioned manifest data for screen rules; attention vocabulary is structured from day one (`AttentionItem {reason, source, freshness, action?}` + reserved `NeedsInputRequest`), never a bare boolean; M1 answers via terminal attach (F2, founder-approved), structured approval UI is M2.
- Rejected: hook-state as load-bearing source; client-side attention derivation; unversioned hardcoded detection patterns.
- Confidence: high. Gap: needs-input *request* production in M1 is terminal-evidence-based (manifest blockers), not structured; structured requests arrive with provider integrations that emit them.

### 7. External-session discovery
- Strongest: Orca + Herdr (hook identity), Wake (history discovery), CC Switch (config mutation safety).
- Evidence: hook-carried session identity (session_id + transcript_path) validated by both Orca and Herdr (`is_official_agent_source` assurance gating); Wake maps every provider's on-disk session locations (12 adapters, `AgentAdapter` trait: detect/list/file_ref/quick_meta/parse, mtime+size change detection, tombstones); CC Switch: atomic same-dir tempfile+rename with mode preservation + `toml_edit` + backfill-before-overwrite + pre-mutation backup.
- Decision: hook/provider identity + live runtime corroboration = Attested bindings (control-capable); history/cwd matching = Heuristic (read-only); PR6 global hook config uses CC Switch's write patterns + Herdr's jsonc-CST merge; degrade to read-only when merge safety unprovable; Wake's adapter-trait shape adopted for history discovery (M2).
- Rejected: silent control-capable heuristic binding; config overwrite without backfill/backup.
- Confidence: high. Gap: first-party verification of current Claude/Codex hook payload stability across resume (spike before PR4).

### 8. Desktop architecture (GPUI)
- Strongest: Zed (primary), Wake (small-scale existence proof).
- Evidence: Zed terminal = alacritty-fork in-process, per-terminal IO thread, FairMutex grid, per-frame owned viewport snapshot, 4ms event coalescing, style-batched shaping via LineLayoutCache, custom 3k-LOC Element, entity-per-terminal view-granular invalidation, embedded/standalone terminal modes, `spawn_dedicated` for parsers (macOS GCD 512KiB stacks), gpui not on crates.io (pin a rev; platform crates just split). Wake: complete GPUI+gpui-component app in 10.7k LOC, `ListState`+splice transcript virtualization.
- Decision (pre-PR8 checklist): entity per terminal; custom Element; 4ms coalescing; owned frame snapshot; embedded/standalone mode enum on day one; pinned gpui (as of 2026-09-04 gpui ships on crates.io; Corral pins the exact release `gpui = "=0.2.2"`, no git rev — `docs/decisions/2026-09-04-pr9-spike-grill.md` Q2); client-side emulator fed by daemon ANSI (display-only mode is first-class in Zed itself; qwertty-term-vt chosen as the client engine, grill Q3).
- Rejected: terminal-in-client PTY ownership; composing the grid from div-like elements; unpinned gpui.
- Confidence: high. The ANSI-replica rendering gap was measured by the PR9 spike (`docs/references/2026-09-04-pr9-gpui-integration-spike.md`): paint p95 1.34 ms at 200×60 under the real display link; the spike also found six daemon-side defects the PR9 plan cleared first (S6, S1, S2 fixed; S3/S4/S5 became ADR 0017 and the client rules the plan carries).
- Built: `corral-desktop` (PR9, `docs/plans/2026-09-05-pr9-desktop.md`) materialises every decision above — entity per terminal, custom `TerminalElement`, 4 ms coalescing, `AnyView::cached` invalidation, embedded/standalone hosts, qwertty replica, `gpui = "=0.2.2"`. Platform support is claimed for macOS alone; Linux compiles and its non-rendering tests run in CI, but no display, render or input path is validated there, and the Desktop says *unvalidated* for Linux until it is (`PRODUCT.md` §6). Release paint numbers on the real element, measured from `cargo run --release -p corral-desktop --example frame_harness` under CVDisplayLink at ~60 Hz: paint p95 **0.91–0.95 ms** at 200×60 and **0.20 ms** at 80×24, inside the 8 ms budget (macOS 26.5; the PR9 plan's DoD records the run).

### 9. TUI architecture
- Strongest: Pi (interaction model), Codex tui (testing), Herdr (mouse-first ratatui).
- Evidence: Pi: custom TUI lib with two renderers behind one interface (scrollback-preserving main-screen + alt-screen viewport), named-action keybinding registry (no hardcoded keys), editor component with kill-ring/undo/paste-burst, tmux-driven TUI verification recipe; ~17k LOC library. Codex: ratatui + 684 insta snapshots, TUI-visible changes must include snapshot coverage. Herdr: mouse-first ratatui at scale.
- Decision: Rust TUI (ratatui) as pure protocol client; named-action keybinding registry; insta snapshot coverage mandate; tmux-scripted TUI e2e recipe in AGENTS.md.
- Rejected: god-file interactive mode (Pi's 6.6k-line `interactive-mode.ts` named as anti-pattern by its own repo conventions); server-side rendering.
- Confidence: high. Gap: none for M1 scope.

### 10. Tray / background lifecycle
- Strongest: CC Switch.
- Evidence: exit-request classification (runtime auto-exit vs restart vs user quit); macOS Accessory/Regular activation-policy flips + dock-click Reopen; Windows ghost-tray-icon removal before hard exit; single-instance socket cleanup before restart; autostart strictly opt-in (`auto-launch`, macOS .app-bundle path fix). ~500 lines of platform edge-case handling.
- Decision: tray is a thin client of corrald (attention counts via protocol); adopt CC Switch's lifecycle edge-case catalog when Tray lands (M1 completion, after PR8); zero-background default unchanged. Mechanism (2026-09-05, `docs/plans/2026-09-05-tray.md`, grill Q3): `tray-icon` + `muda` over the objc2 AppKit bindings gpui already links, macOS only; the Desktop process owns the status item (no tray process), the item is shown a pure projection of daemon truth rebuilt only when it changes, callbacks only forward the clicked id to gpui's foreground. Validated by the Design 0 probe's self-driven cases (`docs/references/2026-09-05-tray-probe.md`: creation, windowless cycles, dynamic menu, idle resources); the human click cases are the feature's DoD walk. Watchfulness ⇔ an established status item: no item, no claim, quit on close.
- Rejected: tray process owning any runtime truth; autostart-by-default; a Corral-owned unsafe AppKit boundary crate (its own decision if the safe crates ever fail).
- Confidence: high for the problem catalog and the mechanism's composition with gpui. Known gap: Dock-menu Quit, logout and shutdown reach AppKit's default `applicationShouldTerminate`, which gpui 0.2.2 gives no hook for, so the Quit warning cannot be shown on those paths (plan D4).

### 11. History / search
- Strongest: Wake (primary), OpenCode (storage model).
- Evidence: Wake: 12-provider discovery/parse + FTS5 trigram external-content index + live watch + resume in 10.7k LOC two-crate Rust; derived/rebuildable index (corrupt-DB sidestep + rebuild, only user annotations non-derived); quick-meta-first two-phase scan; `unknown_line_count` as parser-drift signal; <3-char LIKE fallback; seq-contract search jumps. OpenCode: provider files remain source of truth; shadow snapshots.
- Decision (M2): HistoryIndex = derived SQLite+FTS5 trigram, rebuildable; adapter-trait discovery; provider files remain source of truth (plan §9 unchanged, now evidenced); scale risk downgraded — M2 history is a small-crate problem.
- Rejected: index as system-of-record; incremental within-file cursors before evidence they're needed (Wake proves whole-file reparse + mtime/size gating suffices at real scale).
- Confidence: high. Gap: none blocking (M2).

### 12. Protocol evolution
- Strongest: Codex (primary), Orca (rules + window), OpenCode (negative evidence), Herdr/Zed (exact-match alternative and when it's valid).
- Evidence: Codex: frozen v1 / active v2, `#[experimental]` + initialize-time capability opt-in, "never `skip_serializing_if` on payload fields", generated schema fixtures committed + tested, breaking-change checklist enumerating external surfaces. Orca: version window + absent⇒0 kill-switch + three wire rules + cross-version test dialing the previous release. OpenCode: **no negotiation ⇒ clients grew endpoint-shape sniffing** (`server-protocol.ts` probes health-endpoint shape) — the failure Corral's hello avoids. Herdr/Zed: exact-match works only when one side fully provisions the other (same binary / version-named sidecar) — invalid for Corral's mixed-version invariant.
- Decision: hello with protocol_version + min_compatible + capabilities from PR1 (now multi-source, not Orca-copied); unknown-tolerant wire invariant + future-input tests; opcode permanence; committed generated schema fixtures per version (Codex practice); split recovery by stream kind — terminal = snapshot resync; session events = durable per-session log + `after` cursor (OpenCode practice).
- Rejected: endpoint sniffing; exact-match-only compat; silent semantic reinterpretation.
- Confidence: high. Gap: none for local M1; cross-version release-dial test deferred to M3+ as planned.

### 13. Remote / Mobile future boundaries
- Strongest: Orca (mobile/E2EE/reconnect/pairing), Zed (ssh sidecar + reattach proxy + heartbeat state machine), Herdr (remote thin-client bridge).
- Evidence: Orca report (pairing offer schema, E2EE v2 design, reconnect ladders, notification seq replay). Zed: daemonized remote server + unix-socket reattach proxy + pid file; heartbeat 5s×5 + explicit degraded-state enum; version-named sidecar provisioning. Herdr: run the remote machine's own binary as bridge; managed ssh config keepalives + per-attach ControlMaster.
- Decision: M3+ scope. Seams kept in M1: tri-state liveness vocabulary, resync-by-snapshot, capability-gated evolution, endpoint (not socket) wire concept. When remote arrives: Noise-style crypto (not Orca port); SSH bootstrap may use Herdr/Zed bridge patterns; reconnect numbers start from Orca's calibrated values.
- Rejected: any of this leaking into local M1.
- Confidence: high that deferral is safe. Gap: intentional — revisit at M3 ADR.

### 14. Plugin / extension seams
- Strongest: Herdr (manifest model), Pi (API shape + UI bridging), Codex (ext/* crates as internal seams).
- Evidence: Herdr: out-of-process argv packages, `herdr-plugin.toml` (min_herdr_version, actions/events/panes), entire-CLI-as-plugin-API, trusted-user-code, plugins survive handoff. Pi: 40+ event hooks, registerTool/Command, extension-UI request/response bridging across process boundaries. Codex: first-party features as separate `ext/*` crates against internal APIs.
- Decision: unchanged — M0/M1 preserves seams only (semantic CLI/RPC/events usable without Desktop); security model intentionally undecided; Herdr manifest model remains the leading candidate shape.
- Rejected: in-process trusted-TS extension model as the security answer; any plugin runtime in M1.
- Confidence: high. Gap: intentional (deferred ADR).

### 15. Governance / testing practices (cross-cutting)
- Strongest: OpenClaw + Codex + Pi + CC Switch.
- Evidence: OpenClaw: scope-by-violated-invariant; bug-fix net-LOC≤0 default; evidence-map review; `git log -p -S` premise verification; schema bumps require human acceptance; `read_when:` frontmatter on doctrine docs; per-author PR caps. Codex: wiremock SSE mock-model harness + request capture; integration-test mandate for agent-behavior changes; workspace deny-lints; disallowed-methods as boundary law; 500/800 LoC module rules. Pi: faux-provider-only tests; regression naming `<issue>-<slug>`; tmux TUI harness. CC Switch: AI-contribution policy text.
- Decision: adopt into AGENTS.md/ENGINEERING_WORKFLOW incrementally as rules are earned (per plan §15 philosophy); immediately adoptable: `read_when:` frontmatter on ADRs/plans, schema-bump human gate, disallowed-methods enforcement, mock-provider integration harness for PR4+.
- Confidence: high. Gap: none.
