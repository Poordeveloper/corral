# Herdr (herdrdev/herdr) — engineering reference report for Corral

Reviewed at commit `624dfd4796559042ec13ccf4d4b54374902ab81d` (2026-08-20, v0.8.2)
of `github.com/herdrdev/herdr`. Apache-2.0, ~31k stars. All file paths relative to
that repo's root. Companion to `orca-mobile-remote-report.md`; the "Disposition"
section at the end records per-subsystem Corral reuse verdicts and the
contradictions this source review surfaced against earlier Corral assumptions.

Unlike Orca (TypeScript/Electron — design-only reuse), Herdr is a single Rust
crate on a stack congruent with corrald (tokio, portable-pty, bincode, serde),
so **direct absorption of code is a real option**, not just design copying.

---

## 1. Stack + layout

- Single Rust binary crate `herdr` (edition 2021), `src/` ~227k LOC across 270
  files + `tests/` ~19k LOC. One binary serves as server, client, CLI, and
  remote bridge (subcommand-dispatched in `src/main.rs`).
- Key deps (`Cargo.toml`): tokio (multi-thread rt), ratatui 0.30 + crossterm
  0.29 (TUI client), `interprocess` 2.4 (Unix sockets + Windows named pipes,
  one abstraction), `portable-pty =0.9` **vendored + patched**
  (`vendor/portable-pty`, `[patch.crates-io]`), bincode 2 (client wire),
  serde_json (API), `jsonc-parser` with CST (format-preserving edits of user
  JSON configs), `schemars` (published API JSON schema), clap.
- **VT engine = vendored `libghostty-vt` 1.3.2** (Ghostty's Zig VT core, C ABI;
  `vendor/libghostty-vt`, pinned by `libghostty-vt.vendor.json`, local patches
  documented in `libghostty-vt.patches.md`). `build.rs` invokes Zig per target
  (macOS/Linux/Windows, glibc+musl) with SIMD flags — a **Zig toolchain build
  dependency**. FFI bindings: `src/ghostty/bindings.rs` + `mod.rs` (~8.3k LOC).
- Windows deps: windows-sys (JobObjects, Pipes, Console, ToolHelp, …), wmi,
  widestring (SDDL security descriptors for named pipes).
- Governance: root `AGENTS.md` encodes production principles — AppState is pure
  data separated from runtime; render is pure; platform code isolated in
  `src/platform/<os>.rs` (no `cfg` in core); detection decoupled (reads a
  screen snapshot, never parser internals); multiplicative-performance
  discipline (cost = per byte/event × panes × clients, `just
  bench-render-scale`, release smoke vs current stable binary); and a
  **runtime/client boundary guardrail**: "Herdr is migrating toward a
  server-owned runtime protocol with the TUI as one client… do not add new
  shared behavior that only works through the private TUI client socket; use
  neutral server/API names, not sidebar/row/card/widget."

## 2. Daemon/server architecture

- tmux-shaped client/server in one binary (`src/main.rs`): default launch =
  `server::autodetect::auto_detect_launch()` — probe for a running server,
  spawn `herdr server` detached if absent, attach as client. `herdr server` =
  headless server; hidden `herdr client`; `--no-session` = the pre-split
  **monolithic escape hatch** (single process, no persistence) kept alive for
  testing; `--session <name>` = named servers with per-session socket/data
  dirs; nested-launch guard via `HERDR_ENV=1`.
- Server core: `src/server/headless.rs` (11,729 lines) runs the full App state
  machine **and server-side view computation** — the server renders ratatui
  `FrameData` and streams frames; clients are thin displays. This is the
  coupling their own guardrail now fights; treat it as the anti-pattern half of
  an otherwise correct server-owns-everything design.
- Two endpoints, both 0600: JSON **API socket** (`herdr.sock` — CLI, agents,
  plugins, integrations) and private binary **client socket**
  (`herdr-client.sock` — TUI attach). Path derivation + env overrides in
  `src/server/socket_paths.rs`; activation/staleness in `src/ipc.rs`:
  connect-probe — connect fails ⇒ stale ⇒ remove and bind; connect succeeds ⇒
  `AddrInUse` ("herdr server is already running"). Simpler than Orca's
  link/rename ownership dance; accepts the remove-then-bind race as fine for a
  single-user local daemon.
- Server survives client exit (`prefix+q` detach); `server stop` /
  `reload-config` / manifest reload via API; clients: App, AppDirectGraphics,
  TerminalAttach (`ClientLaunchMode`, `src/protocol/wire.rs`).

## 3. PTY ownership + lifecycle

- The server owns every PTY. Unix: hand-rolled libc implementation —
  `src/pty/backend/unix.rs` (openpty/spawn, 94 lines),
  `src/pty/actor/unix.rs` (~1.5k) = **one dedicated OS thread per PTY** in a
  `poll()` loop over {pty fd, wake pipe} (`src/pty/fd.rs`), with nonblocking
  fds, wake-writer for cross-thread nudges, resize via `TIOCSWINSZ` with pixel
  sizes, and handoff pause/resume built into the actor. Windows: ConPTY via
  vendored portable-pty (`src/pty/backend.rs` `spawn_with_portable_pty`).
- Child lifecycle: Unix process groups; Windows **Job Objects with
  `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`** (`src/platform/windows.rs:602,703`) —
  server death deterministically kills the shell and every descendant.
- `TerminalRuntime` (`src/terminal/runtime.rs`) wraps `PaneRuntime`
  (`src/pane.rs`, 4.5k) — an explicit in-progress migration from pane-coupled
  to terminal-layer ownership.

## 4. Terminal state ownership

- Authoritative screen state lives **in the server**, one ghostty-vt instance
  per pane; default scrollback cap 10MB/pane (Ghostty's default,
  `scrollback_limit_bytes`). OSC title + OSC 9;4 progress tracked as
  first-class metadata; kitty graphics; kitty keyboard protocol flags tracked
  per pane (and carried through handoff).
- Detection reads a **dedicated bottom-buffer snapshot** ("detection source"),
  never the user-scrollable viewport (users scroll it; AGENTS.md rule), exposed
  for debugging via `herdr agent read <pane> --source detection`.
- Local client rendering = server-computed semantic frames; remote rendering =
  server-diffed ANSI (`RenderEncoding::TerminalAnsi`,
  `src/protocol/render_ansi.rs`).

## 5. Detach/reattach + persistence

- Detach = client disconnect; server persists. Reattach replays current state.
  Real e2e coverage: `tests/detach_reattach.rs` (detach persists server, clean
  disconnect, reattach shows current state, TTY size), `tests/multi_client.rs`,
  `tests/broken_pipe.rs`, `tests/client_mode.rs`, `tests/server_headless.rs`.
- Persistence (`src/persist/`): `SessionSnapshot` **v3** JSON (+ separate
  `SessionHistorySnapshot`) — workspaces/tabs/layout nodes/pane identity,
  public pane/tab numbering, worktree membership; legacy-format migration
  structs decode older snapshots; writes are tmp + rename atomic
  (`src/persist/io.rs`; no fsync). Plugin registry persisted separately with
  manifest-reload warnings (`src/persist/plugin_registry.rs`).
- **Cold-restart continuity is provider-native resume, not byte replay**:
  `[session] resume_agents_on_restore = true` re-launches supported agents into
  their native conversations from `PersistedAgentSession` refs
  (`src/agent_resume.rs`: `AgentSessionRef` id|path with length caps, refs
  accepted **only from official integration sources** —
  `is_official_agent_source` — an assurance gate; `AgentResumePlan` builds
  argv + dedupe key). Raw screen history across full restarts is experimental
  and off (`[experimental] pane_history = false`).

## 6. Live upgrade / handoff / FD transfer

- **Unix-only.** Every symbol is `#[cfg(unix)]`; on Windows
  `perform_live_handoff` returns "live handoff is only supported on Unix"
  (`src/server/headless.rs:1444-1450`).
- Trigger: `herdr update --handoff` (`src/update.rs`) → checks the running
  server's JSON-API `status.capabilities.live_handoff` (capability-gated across
  versions), installs the new binary, calls `server.live_handoff` with
  `import_exe` + `expected_version`/`expected_protocol`, waits for the
  replacement to confirm. Servers too old for handoff fall back to
  stop/restart+restore plans.
- Sequence (`src/server/headless.rs:1229-1435` + `src/server/handoff.rs`):
  1. Bind pid-scoped 0600 handoff socket + fresh token; refuse if panes > 64
     (`MAX_FDS_PER_HANDOFF`).
  2. Disconnect all clients; reject pending connects.
  3. **Quiesce**: pause each PTY reader (2s deadline) so no output is lost.
  4. Capture persist snapshot + per-pane `HandoffRuntimeState`
     (`src/handoff_runtime.rs`): child_pid, dims + cell px, kitty keyboard
     flags/ANSI, input state, title, and ≤8KB `initial_history_ansi`
     (`MAX_REPLAY_BYTES_PER_PANE`) — **skipped for panes with a persisted
     agent session**; those TUIs get a redraw nudge on first client attach
     instead (`nudge_child_redraw_after_handoff`).
  5. Spawn `herdr server --handoff-import <sock> <token>` from the **new**
     binary, detached.
  6. Importer: token line auth → JSON manifest → validates
     `HANDOFF_VERSION == 1`, `expected_protocol == its PROTOCOL_VERSION`,
     `expected_version == its build version` → "validated".
  7. `dup()` each PTY master; single `SCM_RIGHTS` sendmsg carries all fds;
     importer rebuilds runtimes from fds (`TerminalRuntime::from_handoff_fd`)
     → "restored".
  8. Old server removes its public sockets **only if it still owns them**
     (inode identity check); importer binds them, reports "ready".
  9. "committed" → old marks runtimes preserved (children NOT killed on drop)
     → importer acks "owned" (500ms tolerance) → old exits.
  - **Failure at any step fails back**: kill+reap the import child, wait for
    its sockets to close, re-bind own public sockets, unpause readers
    (`restore_public_sockets_after_failed_handoff`,
    `rollback_handoff_before_commit`). Timeouts: 30s ready/commit.
- Deliberate non-goals (doc comment, `src/handoff_runtime.rs`): in-flight
  requests, waits, subscriptions, client sockets, pane-to-pane messages are
  dropped — "clients reconnect and retry".
- Known losses: scrollback beyond 8KB per shell pane; anything past the
  64-pane cap refuses the handoff entirely (explicit error telling the user to
  close panes or restart normally).
- Tests (`tests/live_handoff.rs`, 16 e2e tests against real binaries):
  `live_server_holds_one_pty_master_fd_per_pane` (counts ptmx fds in
  /proc/pid/fd), pane process **IO continuity across handoff**, keyboard
  protocol preservation, plugin registry preservation, named-session socket
  path preservation, leaked-socket-env cases, replacement-pid tracking.

## 7. Agent detection / status architecture

Three cooperating layers, with an explicit history of rebalancing:

- **Screen detection (primary for state)** (`src/detect/`): states
  `Idle | Working | Blocked | Unknown`; a rule engine over **versioned,
  remotely-updatable TOML manifests** — one per agent, 20 agents
  (`src/detect/manifests/*.toml`; claude, codex, cursor, gemini, opencode,
  copilot, grok, …). Manifest header: `version = "2026.08.19.1"`,
  `min_engine_version = 2`, `updated_at`, aliases; background updates from
  herdr.dev (`manifest_check`, `src/detect/manifest_update.rs`) plus
  `server.reload_agent_manifests` API — **detection survives agent-UI drift
  without a binary release**. Rules: priority + named region (`osc_title`,
  `osc_progress`, `bottom_non_empty_lines(N)`, `after_last_horizontal_rule`,
  `prompt_box_body`, `last_non_empty_above_prompt_box`, `whole_recent`) +
  AND/OR/NOT `contains`/`regex` gates + `visible_idle/blocker/working` +
  `skip_state_update` (agent-owned transcript viewers must not clobber state).
  AGENTS.md mandates evidence-based manifest changes captured from the real
  detection buffer.
- **PTY activity is the normal Working authority** (comment in
  `src/detect/mod.rs`) — output flow, not patterns, drives `Working`;
  `visible_working` is diagnostic/fallback.
- **Provider-native integrations** (`src/integration/`, version 8): managed
  hook scripts for Claude/Codex/Kimi (`assets/*/herdr-agent-state.sh|.ps1`),
  TS extensions for Pi/OMP, opencode config. **The current hooks report ONLY
  session identity** (`SessionStart` → `pane.report_agent_session` with
  `session_id` + `transcript_path`); `HOOK_REMOVALS`
  (`src/integration/claude_settings.rs`) actively uninstalls the previous
  generation's state hooks (PostToolUse/PreToolUse/UserPromptSubmit/
  SubagentStop→working, PermissionRequest→blocked, Stop→idle,
  SessionEnd→release). The shim documents why: "SubagentStop is a completion
  event… Claude recap/away-summary can emit it after the main turn has already
  stopped. Never let it revive an idle pane." **Herdr tried hook-driven state
  in production and rolled it back to identity-only.**
- Hook shim hygiene (the ADR-4 checklist, implemented): `set -eu` with
  `exit 0` on every failure path; guards on `HERDR_ENV`, `HERDR_SOCKET_PATH`,
  `HERDR_PANE_ID`, python3 presence; 0.5s socket timeout; all exceptions
  swallowed; never starts the server; endpoint discovery = env vars injected
  into the pane (`apply_pane_base_env`) — pane-scoped, no global endpoint
  files (herdr only cares about panes it owns).
- Config mutation done right: `claude_settings.rs` edits the user's Claude
  settings via **jsonc CST editing** (format/comment preserving), per-event
  ensure/remove with matching on managed command variants, extensive fixture
  tests — the exact reference for Corral PR6's merge/uninstall problem.
- Arbitration & freshness: per-source metadata reports with TTL, seq, and
  per-field `reported_at`, merged freshest-wins (`src/terminal/metadata.rs`);
  a currently-visible screen blocker may override a non-blocked integration
  state (`AgentDetection.visible_blocker` doc); effective state lives on
  server-owned terminal state (`pane_effective_state`,
  `src/server/headless.rs:1944`).

## 8. Client/server boundary + protocol/versioning

Three distinct protocols, versioned differently on purpose:

- **Private client wire** (`src/protocol/wire.rs`): bincode, u32-LE length
  prefix, 2MB frame cap (32MB graphics, 16MB clipboard-image);
  `PROTOCOL_VERSION: u32 = 20`; `check_client_version` = **exact match only**,
  both skew directions rejected with explicit upgrade messages. Server-rendered
  `FrameData` (SemanticFrame) or pre-diffed ANSI (TerminalAnsi). UI-coupled by
  admission; their guardrail is steering new work away from it.
- **JSON API socket** (`src/api/`): string method names (`workspace.create`,
  `agent.send_keys`, `pane.report_agent_state`, `server.live_handoff`, …),
  request `{id, method, params}`; schemars-generated **published schema**
  (`docs/next/api/herdr-api.schema.json`, shipped in the crate); `status`
  reports `protocol` + a `capabilities` object (e.g. `live_handoff`); the CLI
  hard-gates on protocol equality per request with a structured JSON error and
  "restart the Herdr server / upgrade the client" guidance
  (`src/cli/protocol_guard.rs`). Long-poll waits (`pane.wait_for_output` with
  regex+timeout, `events.wait`) + subscriptions + event hub
  (`src/api/wait.rs`, `subscriptions.rs`, `event_hub.rs`) power agent-native
  orchestration ("wait until another agent is genuinely blocked").
- **Handoff manifest**: own `HANDOFF_VERSION = 1` + exact
  expected-version/protocol gates set by the updater.
- **Cross-version skew is architecturally avoided, not negotiated**: same
  binary is client+server locally; `--remote` runs the *remote machine's own
  binary* as a bridge (`remote-client-bridge`) against its local server socket
  and pipes ANSI over SSH stdio (`src/remote/attach.rs` — "Remote thin-client
  launcher over SSH command stdio"; managed ssh config with keepalive
  fallbacks + per-attach ControlMaster socket; remote binary auto-updated from
  herdr.dev manifests). The wire never crosses a version boundary.

## 9. Plugin architecture

`docs/next/website/src/content/docs/plugins.mdx` + `src/plugin_command.rs`,
`src/plugin_paths.rs`, `src/persist/plugin_registry.rs`, `src/api/schema/plugins.rs`:

- A plugin = directory + `herdr-plugin.toml` manifest: id, version,
  `min_herdr_version`, platforms, `[[build]]`, `[[startup]]`, `[[actions]]`
  (with contexts), `[[events]]` (on = server event names), `[[panes]]`, link
  handlers. Any argv command in any language.
- **The entire CLI is the plugin API** — no SDK, no restricted command set;
  `HERDR_BIN_PATH` keeps invocation portable across Unix sockets and Windows
  named pipes; raw socket API available.
- Trust model: explicitly trusted user code — install preview with
  confirmation, `--yes`, `--ref` pinning; manifest validated; per-plugin
  config/state dirs; **no sandbox, no capability system** ("Third-party
  plugins come from their authors, not Herdr").
- Registry persisted; manifests re-validated on reload with warnings instead
  of drops; plugins survive live handoff (tested).
- Windows batch entrypoints run through `cmd.exe /d /c` with explicit
  resolution (`src/plugin_command.rs`) — the same trap Orca documents.

## 10. Cross-platform / Windows

- Real native Windows support, shipped as a separate **preview channel**
  (`[update] channel` — "Windows preview builds default to preview").
- `src/platform/windows.rs` (~4k lines): named pipes with SDDL descriptors;
  Job Objects with KILL_ON_JOB_CLOSE; process-tree snapshots via
  ToolHelp/wmi with caching + pid-reuse-safe `ProcessSignature`
  (pid + creation time); `DETACHED_PROCESS`/`CREATE_NO_WINDOW` daemon spawn;
  ConPTY fallback key encoding; ConPTY cursor-flicker workaround (`host_cursor
  = "auto"` draws Herdr's own cursor on Windows/WSL); Korean IME toggling;
  UNC extended-length paths.
- **No live handoff on Windows**. Continuity across a Windows server
  replacement = snapshot restore + provider-native agent resume. Server death
  kills panes deterministically (job object) — the honest-physics version of
  our crash commitment.

## 11. Tests around runtime lifecycle + handoff

- Style: black-box e2e against real spawned server binaries with isolated
  config/runtime dirs and socket-env overrides; JSON API driven; polling
  helpers with deadlines; `/proc` fd counting for resource invariants; a
  process-shared test lock for serialization. Files: `tests/live_handoff.rs`
  (16), `tests/detach_reattach.rs`, `tests/multi_client.rs`,
  `tests/client_mode.rs`, `tests/server_headless.rs`, `tests/broken_pipe.rs`,
  `tests/api_ping.rs`, `tests/auto_detect.rs`, `tests/cli/*`,
  `src/server/headless/tests/` (unit-level server tests).
- Performance is governed, not vibes: `just bench-render-scale` (1 vs 15+
  panes scaling deltas), `just bench-release-smoke` vs the current stable
  binary before releases (AGENTS.md).

---

## Disposition — Corral reuse verdicts per subsystem

Scale: **reuse directly** (vendor/port with minimal change) · **absorb/refactor**
(port the code, reshape to Corral's model) · **copy design only** · **reject**.

| Subsystem | Verdict | Notes |
|---|---|---|
| PTY layer (`src/pty/*`, vendored portable-pty patches) | **absorb/refactor** | Thread-per-PTY poll actor with wake pipe, handoff pause built in; small, hardened. Take the vendoring approach for portable-pty (Windows/ConPTY) too. |
| Live handoff (`server/handoff.rs`, `handoff_runtime.rs`, `perform_live_handoff`) | **absorb/refactor** (Unix) | Two-phase commit, token auth, SCM_RIGHTS, socket-identity checks, quiesce, fail-back; e2e-proven. Port protocol + choreography into corrald; revisit the 64-pane cap and the 8KB reseed (Corral may serialize full VT state instead). |
| Terminal state (vendored libghostty-vt + `src/ghostty/` bindings) | **reuse directly** (candidate for ADR 3) | Production-proven Rust-hosted VT with serialization, OSC title/progress, kitty kbd/graphics; resolves our emulator question. Cost: Zig toolchain in the build. Evaluate against pure-Rust alternatives (alacritty_terminal/wezterm-term) before committing ADR 3. |
| Detection engine + manifests (`src/detect/*`) | **absorb/refactor** | Region-based AND/OR/NOT rule engine over versioned, remotely-updatable TOML; 20 agent manifests are reusable data. This is the strongest available implementation of Corral's "screen detection as fallback evidence" layer. |
| Provider integrations (`src/integration/*`) | **copy design only** (absorb `claude_settings.rs` merge machinery) | SessionStart-identity-only shim + fail-open hygiene = ADR 4 implemented; jsonc-CST settings merge/uninstall = PR6's exact problem solved. Corral's hook usage is broader (needs-input evidence), so the shim's scope differs. |
| Agent resume (`src/agent_resume.rs`, `resume_agents_on_restore`) | **absorb/refactor** | Session refs with source-assurance gating, argv plans, dedupe. Direct fit for Corral's Attested bindings + NativeResume. |
| Private client wire (`src/protocol/wire.rs`, server-rendered frames) | **reject** | Exact-version, UI-coupled, server-side-rendered ratatui frames — the coupling Herdr's own guardrail is retiring; opposite of Corral's semantic protocol. |
| JSON socket API (`src/api/*`) | **copy design only** | String methods + published schemars schema + `status.capabilities` + long-poll waits + event subscriptions: adopt shapes; Corral's protocol adds the min-compat window + unknown tolerance Herdr deliberately lacks. Copy the structured protocol-mismatch error UX. |
| Persistence (`src/persist/*`) | **copy design only** | Snapshot versioning + legacy migration + tmp/rename atomicity + what-to-persist inventory (layout, terminal metadata, agent session refs, plugin registry). Corral is SQLite-backed by frozen decision. |
| Server core (`server/headless.rs` monolith) | **reject** | 11.7k-line server that also computes UI views; keep corrald protocol-first. Its loop-event/render-impact patterns are still worth reading. |
| Singleton/activation (`ipc.rs`, `socket_paths.rs`, `autodetect`) | **absorb/refactor** (ADR 1 input) | Connect-probe stale-socket recovery + AddrInUse rejection + spawn-then-attach autodetect + per-session paths; `interprocess` crate unifies Unix socket / named pipe. |
| Plugin model | **copy design only** | Confirms the v1.9 plan's Herdr reference in source: manifest + min-host-version + entire-CLI-as-API + trusted-user-code. Still deferred for Corral (M0/M1 seams only). |
| Remote/SSH (`src/remote/*`) | **copy design only** (M3) | Thin-client-over-SSH-stdio with the remote's own binary as bridge = version skew avoided architecturally; managed ssh config with keepalive fallbacks + per-attach ControlMaster. A genuine alternative to versioned-protocol remoting for the SSH bootstrap path. |
| Windows platform layer (`src/platform/windows.rs`) | **absorb/refactor** (when the Windows track opens) | Job objects (KILL_ON_JOB_CLOSE), SDDL named pipes, pid+creation-time process identity, ConPTY quirks, cmd.exe batch traps — a map of every trap, in Rust. |
| Lifecycle test style (`tests/*`) | **copy design only** | Real-binary e2e + /proc fd assertions + deadline polling; the template for corrald PR3/handoff tests. |

## Contradictions vs. earlier Corral assumptions (from the Orca-era verdict)

1. **"Provider-native hooks are the primary semantic source for agent *state*"
   — contradicted for the state half.** Herdr ran hook-driven state
   (integration versions ≤7 mapped PostToolUse→working,
   PermissionRequest→blocked, Stop→idle, …) and **rolled it back**: v8 installs
   only `SessionStart` and `HOOK_REMOVALS` uninstalls the state hooks, citing
   event-semantics drift (late SubagentStop reviving idle panes). Orca sustains
   hook state, but at the cost of a 4.7k-line per-vendor listener. The two
   production references agree 100% on hooks for **identity/resume** and split
   on hooks for **state**. Consequence for Corral: keep the evidence-ladder
   architecture (it already arbitrates), but amend the Plan §7/§8 wording so
   the Attention Engine (PR7) must be fully functional on screen+PTY-activity
   evidence alone, with hook state transitions as *additional* evidence — not
   the load-bearing source. PTY output activity as the default `Working`
   authority is a Herdr lesson to adopt outright.
2. **Remotely-updatable detection manifests are missing from our plan.**
   If screen evidence carries more weight (per #1), its known weakness — agent
   UI drift — needs Herdr's fix: versioned manifest data with
   `min_engine_version`, reloadable at runtime. Adopt the format/engine in
   PR7; a remote update channel can come later.
3. **Windows continuity: drop the "ConPTY handoff spike" framing.** Herdr,
   with a complete native Windows port, ships **no Windows handoff** and uses
   KILL_ON_JOB_CLOSE + snapshot restore + provider resume as the upgrade/crash
   story. ADR 5's re-entry plan should pre-decide the same model rather than
   spike FD-style handoff on Windows.
4. **Handoff scrollback**: live handoff re-seeds the new VT with ≤8KB of ANSI
   per pane (agent panes: none — redraw nudge instead); deep scrollback does
   not survive upgrades. Corral's ADR 3/handoff design must decide explicitly:
   accept the same bounded loss (consistent with M1's bounded in-memory
   scrollback) or transfer serialized VT state. Neither Orca nor Herdr
   transfers full VT state.
5. **Not a contradiction — a strong confirmation**: one server owning PTYs +
   process lifetime + authoritative VT + runtime truth is exactly Herdr's
   production shape, and their AGENTS.md guardrail ("server-owned runtime
   protocol, TUI as one client, neutral names, don't deepen private-socket
   coupling") independently converges on Corral's frozen client/daemon
   boundary rules. The one part of Herdr to *not* replicate is server-side UI
   rendering over a private exact-version wire — which is the part they
   regret.

## Verdicts on the two standing questions

- **Is "one corrald owns PTY + process lifecycle + authoritative terminal
  state + runtime truth" consistent with Herdr's production learning?** Yes —
  it is literally Herdr's architecture, validated at 31k-star production
  scale, including the migration pressure *toward* a cleaner protocol boundary
  that Corral starts with. No second PTY daemon exists in Herdr either; the
  monolithic mode survives only as a test escape hatch.
- **Is Herdr's live handoff robust enough to base Corral upgrades on?** Yes,
  as the design basis on macOS/Linux: two-phase commit with fail-back,
  quiesced readers, socket-identity-guarded takeover, version/protocol gates,
  capability-gated invocation, and real e2e tests (fd counting, IO continuity,
  input-protocol preservation). Port it; revisit two limits (64-pane cap, 8KB
  reseed) against Corral's session model. **Windows changes the conclusion**:
  no FD-transfer analogue is attempted even by a mature native port — Corral's
  Windows upgrade model should be restore + provider-native resume from day
  one, per finding #3.
