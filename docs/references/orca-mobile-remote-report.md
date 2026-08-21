# Orca (stablyai/orca) — engineering reference report for Corral

Reviewed at commit `59892a2f134f2e0252cabb74be4632602e112936` (2026-08-20, v1.4.178-rc.2)
of `github.com/stablyai/orca`. All file paths below are relative to that repo's root.

Scope of the review: Orca as an engineering reference for mobile/remote — QR pairing,
E2EE, WebSocket/RPC, reconnect/liveness, protocol versioning, terminal
subscribe/resubscribe, mock-server testing, and agent-status detection. Corral
explicitly rejects Orca's IDE/worktree/tab product model and Electron stack; see the
"Disposition" section at the end for what Corral adopted, adapted, and rejected.

---

## 1. Stack + layout

- **License**: MIT, Copyright 2026 Lovecast Inc. (`LICENSE`).
- **Size**: 276 MB checkout, ~16,549 files; 15,689 TS/TSX files (~9,268 non-test),
  ~3.1M LOC TS total; `mobile/src` ~161k LOC, `src/relay` ~63k, `src/shared` ~173k.
  Extremely flat, one-concept-per-file naming; near-1:1 test-to-source.
- **Layout** (pnpm workspace, `package.json` name `orca`):
  - `src/main` — Electron main process ("the runtime"), Electron ^43, node-pty ^1.1,
    ws ^8.21, ssh2, zod ~4.4, tweetnacl, @xterm/headless + addon-serialize
    (main-process authoritative terminal model), qrcode, sqlite via `src/main/sqlite`.
  - `src/renderer` — React 19 + xterm 6.1-beta (patched,
    `docs/reference/xterm-patch-regeneration.md`) + Tailwind/shadcn; `src/preload`;
    built by electron-vite (rolldown-vite).
  - `src/cli` — bundled `orca` CLI (talks to runtime over Unix socket / WS;
    `src/cli/runtime/websocket-transport.ts`).
  - `src/relay` — **SSH remote-host agent**: Node script SCP'd to remote hosts, framed
    JSON-RPC over SSH stdio, keeps PTYs alive on a Unix socket through a grace period,
    `relay.js --connect` re-bridges (`src/relay/relay.ts` header).
  - `src/main/daemon` — **local terminal daemon**: separate Node process owning
    node-pty PTYs so they survive app/serve restarts; NDJSON RPC over Unix socket,
    hello handshake + own protocol version (`daemon-protocol-version.ts`),
    endpoint-ownership protocol documented in `src/main/daemon/AGENTS.md`.
  - `src/shared` — transport/protocol/domain code shared by main, CLI, relay, mobile
    (mobile imports it directly).
  - `mobile/` — Expo 55 / React Native 0.83 / expo-router; terminal = xterm.js
    6.1-beta + webgl addon inside a WebView (prebuilt bundle,
    `mobile/scripts/build-terminal-webview-engine.mjs`); crypto = tweetnacl +
    @noble/hashes; expo-notifications, expo-camera (QR scan), expo-secure-store.
  - `native/` — small platform native helpers.
  - `tests/e2e` — Playwright Electron e2e + `tests/e2e/cross-version-wire/`.

## 2. Runtime/server architecture

- **No standalone daemon binary for VPS mode.** Headless = `orca serve`: the full
  Electron app running under auto-started Xvfb on Linux
  (`docs/reference/headless-linux-server.md` — 860 lines of systemd/AppImage/FUSE
  ops guidance, `--port`, `--pairing-address`, `--json` ready contract, exit code 3 =
  profile lock). Upgrade = replace AppImage + restart; explicitly no auto-update
  headless.
- **PTY ownership is layered**: local terminal daemon (`src/main/daemon/daemon-server.ts`,
  `daemon-pty-adapter.ts`) owns PTY processes across app restarts; Electron main
  attaches/adopts (`daemon-client.ts`, `serve-update-handoff.ts`); SSH hosts own their
  PTYs via `src/relay/pty-handler.ts`. The main process holds the authoritative
  terminal *screen model* (headless xterm + serialize addon;
  `orca-runtime.ts:12603 serializeAuthoritativeTerminalBuffer`).
- **Client connectivity**: one semantic RPC server in `src/main/runtime/runtime-rpc.ts`
  (1,809 lines; deliberately a single auditable security boundary) with transports:
  `rpc/unix-socket-transport.ts` (CLI, token-authed), `rpc/ws-transport.ts` (`ws` lib;
  port 6768 default; loopback-until-paired, all-interfaces on `orca serve` — STA-2370),
  HTTP long-poll (web client, `LONG_POLL_CAP = 16`, keepalive 10s), plus a hosted
  **Relay** path (director/cell cloud service) for NAT traversal
  (`src/main/runtime/relay/`, `docs/reference/relay-regional-placement.md`).
- **Message format**: JSON request `{id, deviceToken, method, params}` →
  `{id, ok, result|error, streaming?}`; zod-validated per-method schemas
  (`rpc/methods/*`, `rpc/schemas.ts`); dispatch via `rpc/dispatcher.ts`; mobile
  restricted by `MOBILE_RPC_METHOD_ALLOWLIST` (runtime-rpc.ts:~180). Everything
  mobile-facing is wrapped in the E2EE channel. Terminal data is a separate binary
  sub-protocol multiplexed on the same socket.
- Desktop→remote-server ("HUB") uses the same pairing offer (`scope: 'runtime'`), same
  e2ee_hello handshake (`src/shared/remote-runtime-request-websocket.ts`), plus a
  shared-control connection with logical subscriptions
  (`src/shared/remote-runtime-shared-control-*.ts`, ~30 files).

## 3. QR pairing + E2EE

- **Not Noise, not libsodium — custom protocol on tweetnacl (NaCl) + HKDF**, two
  generations.
- **Pairing offer** (`src/shared/mobile-relay-pairing-offer.ts`, `pairing.ts`): zod
  schema v2 `{v:2, endpoint, deviceToken, publicKeyB64, pairedDeviceId?,
  scope: 'mobile'|'runtime', relay?}`; relay block = `{v:1, directorUrl, cellUrl,
  assignmentEpoch, relayHostId, inviteToken (43-char base64url), inviteExpiresAt
  (≤10 min TTL + 30s clock-skew leeway), e2eeFraming:2}`. Serialized as JSON →
  base64url → `orca://pair?code=...` (query param, not fragment — Android
  camera-intent reliability), rendered to QR data-URL via `qrcode`
  (`src/main/runtime/mobile-pairing-qr.ts`); paste-pair accepts bare base64. Size caps
  in `mobile-pairing-protocol-limits.ts`.
- **Identity/auth**: desktop static X25519 keypair persisted with hardened perms
  (`src/main/runtime/e2ee-keypair.ts`, `src/shared/secure-file.ts`); per-device random
  24-byte hex token in a JSON device registry (`src/main/runtime/device-registry.ts` —
  pending-token coalescing, explicit rotate-on-regenerate, revocation, lastSeen
  write-coalescing, `pairingReach` this-computer vs network); unpaired-auth throttle
  (`rpc/unpaired-device-auth-throttle.ts`); mobile stores creds in expo-secure-store
  (`mobile/src/transport/pairing-keychain.ts`).
- **E2EE v1 (legacy, direct WS)** (`src/shared/e2ee-crypto.ts`, client flow in
  `mobile/src/transport/rpc-client.ts:349-477`): plaintext
  `e2ee_hello{ephemeral clientPubKey}` → server `e2ee_ready` → static-ephemeral ECDH
  `nacl.box.before` → encrypted `e2ee_auth{deviceToken}` → encrypted
  `e2ee_authenticated` (or `e2ee_error` / WS close 4001). Every frame = random
  24-byte nonce ‖ nacl.box ciphertext. Ephemeral client key per connection = forward
  secrecy; server key pinned by QR = server auth; client authenticated only by the
  device token inside the tunnel. No transcript, no replay protection.
- **E2EE v2 (required on Relay, negotiated on direct)**: handshake adds 32-byte nonces
  both ways, exact-key validation, and a **length-prefixed canonical transcript**
  (`src/shared/mobile-e2ee-v2-contract.ts` — `encodeMobileE2EEV2Transcript`, domain
  `orca-mobile-e2ee/v2/transcript`, context binds transport + relayHostId to kill
  mix-and-match/downgrade); key schedule = HKDF-SHA256(sharedSecret,
  salt=SHA256(label‖nonces), info=label‖transcriptHash) → 96 bytes = directional keys
  + sessionId (`mobile/src/transport/mobile-e2ee-v2-key-schedule.ts`; desktop twin
  `src/main/runtime/rpc/mobile-e2ee-v2-key-schedule.ts`); framing = nacl.secretbox
  with **deterministic nonce** (sessionId[0..12]‖ver‖direction‖payloadKind‖0‖u64
  counter) + 42-byte header re-verified inside plaintext, strict per-direction
  counters = replay/reorder protection (`src/shared/mobile-e2ee-v2-framing.ts`).
  Server session state machine: `rpc/e2ee-channel.ts`,
  `rpc/mobile-e2ee-v2-desktop-session.ts`; wiring/auth glue
  `rpc/mobile-socket-wiring.ts` (relay connections must resolve the same deviceId
  E2EE-side as relay-side).
- Contract-fixture tests: `src/shared/mobile-e2ee-v2-contract.test.ts`,
  `mobile-e2ee-v2-fixtures.ts`, `mobile-e2ee-legacy-fixtures.ts`,
  `rpc/e2ee-integration.test.ts`.

## 4. Reconnect / liveness

- **Server→client (WS ping reaper)**:
  `src/main/runtime/rpc/remote-runtime-server-heartbeat.ts` — protocol-level
  `socket.ping()` every sweep (~15s), any inbound frame marks alive (WeakSet),
  **3 consecutive missed probes → terminate** ("one unanswered probe is UNKNOWN, not
  death"); stalled-tick forgiveness (a delayed timer never charges a missed probe,
  and never forgives banked misses).
- **Desktop-client→server**: `src/shared/remote-runtime-socket-liveness.ts` — ping
  every 10s, dead if no inbound within 25s (level-triggered; catches half-open
  tunnels that never emit `close`, #7718/#7489).
- **Mobile client** (`mobile/src/transport/rpc-client.ts`): backoff ladder
  `[500,1000,2000,4000,8000,15000,30000,60000]` ms; after 12 attempts → 90s
  "trickle" forever (never park — a wedged VPN fires no revival event); attempt
  counter resets **only on authenticated** (`e2ee_authenticated`), not on socket open
  (#10119); `CONNECT_TIMEOUT_MS` 12s (RN onopen can never fire),
  `HANDSHAKE_TIMEOUT_MS` 5s, request timeout 30s; auth-rejection budget 3 before
  latching `auth-failed` (#5200); synthesized close events for RN sockets that never
  deliver onclose (`closeAndSynthesize`).
- **App-level liveness watchdog**
  (`mobile/src/transport/rpc-session-liveness-watchdog.ts`): idle 20s → app-level
  probe RPC (`mobile-liveness-` id prefix), 8s timeout, 3 misses → force reconnect;
  "unfair window" skip when JS was suspended (elapsed > 1.5× timeout charges nothing).
- **Foreground/background**: `notifyForeground()` on AppState active — probes
  immediately if connected (half-open detected ≤24s), abandons stale dials, redials
  instantly without pardoning counted failures; jitter helper
  `src/shared/reconnect-jitter.ts` (+0–20%, one-sided) against thundering herd.
- **UX escalation**: `mobile/src/transport/connection-health.ts` — 3 attempts →
  "Can't connect", 12 attempts (~6 min) + never-connected-or-stale≥60s →
  "unreachable / re-pair?", Tailscale-specific hint; `auth-failed` outranks everything.
- **Relay path**: separate full-jitter exponential backoff 500ms·2ⁿ capped 30s, floor
  250ms; host-offline poll 5–15s; gated reprobe 60s→15min escalating
  (`mobile/src/transport/mobile-relay-retry-delays.ts`,
  `mobile-relay-reconnect-controller.ts`, endpoint hysteresis + direct-upgrade
  controller for relay→direct migration).
- **Replay after reconnect**: RPC subscriptions are client-tracked and re-sent after
  every re-auth (`markStreamsForReplay()` + replay loop in rpc-client.ts:445-468);
  server re-emits current snapshot, tagged `_replayedAfterReconnect` client-side so
  monotonic freshness gates accept it (`src/shared/runtime-subscription-replay.ts`,
  #7718); push-notification catch-up via global monotonic `notificationSeq` + epoch
  watermark, idempotent `getMissedSince`
  (`src/main/runtime/mobile-notification-replay.ts`).

## 5. Protocol versioning / compatibility

- `src/shared/protocol-version.ts`: `RUNTIME_PROTOCOL_VERSION = 3`,
  `MIN_COMPATIBLE_RUNTIME_CLIENT_VERSION = 2`,
  `MIN_COMPATIBLE_RUNTIME_SERVER_VERSION = 2` + explicit bump rubric (bump on
  removal/meaning-change/framing-or-auth change; never for additive optional) + ~50
  string **runtime capabilities** (`terminal.binary-stream.v1`,
  `terminal.paired-parking.v1`, `worktree.create-idempotency.v1`, …) advertised via
  `getStatus()` and per-stream handshake echoes.
- Evaluation is symmetric two-sided min-version (`src/shared/protocol-compat.ts`
  `evaluateRuntimeCompat`): absent field ⇒ protocol 0 (deliberate kill-switch
  semantics documented), `client-too-old` verdict takes precedence; mobile mirror at
  `mobile/src/transport/protocol-compat.ts`.
- Doctrine doc `docs/reference/remote-wire-compatibility.md`: Rule 1 — new optional
  JSON field safe (zod `.strip()`); Rule 2 — new stream opcode NOT safe: decoders
  drop unknown opcodes silently, so it must be capability-negotiated (worked example
  `SetOutputPaused`), opcode numbers permanent; Rule 3 — *content* changes break old
  clients with zero schema change; plus tri-state absent≠null≠value guidance
  (`agentWait`).
- **Enforced by test**:
  `tests/e2e/cross-version-wire/cross-version-terminal-wire.unit.test.ts` runs the
  current working tree against the newest release tag in both skew directions over a
  scripted terminal journey (subscribe, input, hide/reveal, drop, reconnect).
- Separate version domains: daemon protocol
  (`src/main/daemon/daemon-protocol-version.ts`), relay version marker
  (`src/shared/relay-version-marker.ts`), hook protocol
  (`ORCA_HOOK_PROTOCOL_VERSION` in `agent-hook-types.ts`), pairing offer `v:2`, e2ee
  framing `2`, CLI envelope (`src/cli/runtime/envelope-schema.ts`).

## 6. Session model

- Orca's unit is **workspace (git worktree or folder) per repo per execution host**,
  not a session. `src/shared/workspace-session-schema.ts`: persisted state =
  `tabsByWorktree` (terminal tabs → split-pane layout leaves → `ptyId` bindings with
  `incarnationId`), field-level zod salvage on load. Terminal identity:
  `paneKey = ${tabId}:${leafId}` (`stable-pane-id.ts`), PTY incarnations
  (`pty-incarnation.ts`), host-scoped (`execution-host.ts`). Liveness vocabulary for
  remote PTYs is a mandated tri-state `live/unverifiable/exited`
  (`pty-liveness-verdict.ts`, `docs/reference/ssh-execution-boundary.md`).
- **Agent status**: states `working | blocked | waiting | done`
  (`src/shared/agent-status-types.ts`; `AgentStatusEntry` carries prompt, tool
  name/input, interactivePrompt (full AskUserQuestion JSON), lastAssistantMessage,
  subagent roster, provider session id for resume, `restoredUnconfirmed` staleness
  flag; history capped at 20; stale after 30 min).
- **Detection = native agent hooks, explicitly never terminal-title/screen parsing**
  (header comment, agent-status-types.ts:1-3). Orca installs managed hook configs
  into each CLI (Claude Code hooks, Codex, Gemini, Grok, opencode, etc. —
  `src/main/agent-hooks/managed-hook-*`, relay-side
  `src/relay/managed-hook-installer.ts`); the CLIs POST lifecycle events to a local
  HTTP endpoint whose coordinates are written to per-scope endpoint files
  (`src/shared/agent-hook-endpoint-file.ts`); one canonical transport-agnostic
  listener parses/normalizes ~20 vendors' payloads
  (`src/shared/agent-hook-listener.ts`, 4,756 lines + per-vendor test suites) and
  runs identically in Electron main and in the SSH relay
  (`src/relay/agent-hook-server.ts`). Secondary in-band channel: OSC 9999 escape
  sequences parsed from PTY streams (`src/shared/agent-status-osc.ts`) for
  hidden/model-owned terminals; a guarded interrupt fallback synthesizes `done` when
  a cancellation hook is missed. `waiting` = needs-input; interactive prompt payloads
  captured verbatim.
- Status fan-out to mobile: worktree-level session-tabs snapshots with a decorative
  heartbeat re-emit at half the 30-min stale lease
  (`src/main/runtime/mobile-session-tabs-agent-status-heartbeat.ts`), projection in
  `rpc/methods/session-tab-agent-status-projection.ts`.

## 7. Terminal streaming

- **Model**: full snapshot + sequenced raw deltas, not screen-diffing. Binary frame:
  16-byte header `[kind=0x74][ver=1][opcode][pad][streamId u32 LE][seq u64 LE]` +
  payload (`src/shared/terminal-stream-protocol.ts`); opcodes `Output,
  SnapshotStart/Chunk/End, Resized, Error, Input, Resize, Subscribe, Unsubscribe,
  SnapshotRequest, Metadata, Ack(13), ClaimViewport(14), OutputSpan(15),
  SetOutputPaused(16), WriteUnavailable(17)` — with wire-compat renumbering comments.
  Frames ride inside the E2EE channel on the same WS as RPC (`terminal.multiplex.v1`).
- Server: `terminal.subscribe` RPC → snapshot = ANSI-serialized authoritative buffer
  from main-process headless xterm + serialize addon (scrollbackAnsi + screen,
  `rpc/methods/terminal.ts:639-731`, 3,826 lines) anchored at `snapshotSeq`;
  subsequent PTY chunks stream with seqs, replay trimmed against snapshotSeq
  (`terminal-multiplex-resync-replay-trim.test.ts`); Ack-credit flow control
  (`src/shared/terminal-multiplex-flow-control.ts`, benchmarked), round-robin
  fairness across subscribers, output pause + viewport claim; unsubscribe keyed
  `${terminal}:${clientId}` so two phones don't evict each other; "paired parking"
  capability lets mobile unmount xterm and re-reveal losslessly. SSH path has its own
  credit ledger (`src/relay/pty-source-credit-*`).
- Mobile rendering: real xterm.js (webgl) in a WebView; subscription
  bookkeeping/rebind in `mobile/src/transport/rpc-client-terminal-subscription.ts`,
  binary frame routing `rpc-client-terminal-binary-frame.ts`, stream-health drives
  resubscribe (`mobile/src/session/mobile-session-tabs-stream-health.ts`), foreground
  recovery re-requests snapshots (`mobile/src/terminal/terminal-foreground-recovery.ts`);
  full-buffer re-serialize + replay on cols change (mobile can't rewrap hard-wrapped
  restored lines, terminal.ts:795).

## 8. Testing

- **Mock server for mobile dev**: `mobile/scripts/mock-server.ts` +
  `mock-server-{encryption,key-pair,rpc-handlers,session-tabs-fixture,
  terminal-fixtures,terminal-stream,git-state,account-*,native-chat-scenario}.ts` —
  standalone `ws` server implementing the real E2EE handshake + realistic fixtures
  incl. terminal streams; persistent keypair via `MOCK_SERVER_KEY_FILE`; contract
  tests pin it to the real protocol (`mobile/src/mock-server-*.test.ts`).
- **Server-side harnesses**: `src/main/runtime/runtime-rpc-test-harness.ts`,
  `runtime-rpc-mobile-ws-test-harness.ts` (real WS + real E2EE against the real
  dispatcher: auth, allowlist, terminal streaming, pairing-offer, revocation tests in
  `src/main/runtime/runtime-rpc-*.test.ts`, `mobile-subscribe-integration.test.ts`).
- **Cross-version wire tests** (see §5); Playwright Electron e2e (incl. daemon
  lifecycle/reconnect specs); relay integration tests (`src/relay/integration.test.ts`,
  pty attach-replay/revive/backpressure); emulator-driven mobile flows. Vitest
  everywhere.

## 9. Verdict per area (port to Rust vs read as checklist)

Blanket constraint: everything is TypeScript on Node/Electron/Expo — **zero directly
reusable code** for a Rust corrald; the reuse question is design-level.

- **Pairing/E2EE — adapt the design, reimplement in Rust; do not port v1.** The v2
  design (pinned static X25519 in QR, ephemeral peer key, dual 32-byte nonces,
  length-prefixed transcript hash, HKDF-SHA256 directional keys + sessionId,
  deterministic-nonce secretbox with counter replay protection, context binding to
  transport/relayHostId) is a sound hand-rolled AKE, but it is exactly the shape
  Noise XK/IK already standardizes — in Rust, `snow` (Noise) gives the same
  properties with less bespoke risk. If mirroring Orca instead, the v2 contract +
  fixtures files are a precise spec, and its versioned-schema QR offer (zod v2,
  TTL + clock-skew leeway, scope field, size caps) and DeviceRegistry semantics
  (pending-token coalescing, rotate-on-leak, revocation, reach
  widening-never-narrowing) are directly transplantable ideas. Skip v1
  (static-static box, no transcript, no replay protection) — Orca itself is
  migrating off it.
- **Reconnect/liveness — checklist gold; trivial to reimplement.** The valuable
  artifacts are the calibrated numbers and edge cases, not the code:
  3-missed-probes-is-evidence on both ends; stalled-tick non-charging/non-forgiveness;
  reset-attempts-only-on-authenticated (#10119); trickle-never-park (wedged VPN);
  auth-retry budget before latching (#5200); RN synthesized-close and stale-dial
  abandonment; one-sided jitter; seq+epoch notification replay;
  `_replayedAfterReconnect` tagging. Each carries an issue number proving it was
  earned. Port the state machines' *contracts* (many are pure, dependency-injected,
  and small — the watchdog is 196 lines) into Rust directly from the tests.
- **Protocol versioning — adopt the scheme nearly verbatim.** A single small integer
  + min-client/min-server window + absent⇒0 kill-switch + string capability set +
  per-stream opcode negotiation + the three written rules + a cross-version wire test
  that dials the previous release: a complete, transport-agnostic compatibility
  regime that maps 1:1 onto Corral's needs and costs little. The cross-version test
  harness idea (released binary vs working tree, both skew directions) is the single
  highest-value practice to copy.
- **Terminal subscribe — adopt the wire model, replace the buffer machinery.** The
  frame format (16-byte header, streamId+u64 seq, snapshot anchored at snapshotSeq,
  replay trim, Ack credits, capability-gated opcodes, composite
  `${terminal}:${clientId}` subscription keys, paired-parking) is a clean spec worth
  mirroring in Rust. But Orca's snapshot source is a headless xterm.js + serialize
  addon in the Node main process — in Rust, substitute a VT emulator crate for the
  authoritative screen model; that half is not portable and is the bulk of the
  3.8k-line terminal.ts. The query-reply capability gate
  (`terminal.query-reply-input.v1`) is a subtle correctness item to keep.
- **Status detection — adopt the architecture, not the code.** The core decision —
  provider-native hooks as the authoritative needs-input/working signal, OSC 9999 as
  in-band fallback, explicit `restoredUnconfirmed` staleness, never title/screen
  parsing — is the right reference. The implementation (`agent-hook-listener.ts`,
  4.7k lines of ~20 vendor-specific extractors + subagent rosters + transcript
  reconciliation) is a maintenance tarpit tied to each CLI's hook payloads; treat it
  as a catalog of per-provider payload quirks (its vendor test fixtures are
  real-format and directly reusable as Rust test fixtures), and port only the vendors
  Corral supports.

Additional cautionary findings: Orca's "headless server" being the whole Electron app
under Xvfb (with AppImage/FUSE/systemd/rollback pain filling an 860-line ops doc,
`docs/reference/headless-linux-server.md`) is the strongest available argument *for* a
real Rust daemon; and Orca's split PTY ownership (renderer model + local daemon + SSH
relay + serve mode) forced a separately-versioned daemon protocol, an
endpoint-ownership rename dance (`src/main/daemon/AGENTS.md` — 23 defects from
liveness-then-act races), and cold-restore checkpointing — read those two files before
designing corrald's PTY handoff.

---

## Disposition — decisions adopted into Corral (Development Plan v2.0)

Adopted / adapted:

- Provider-native hooks as the primary agent-status evidence source; screen detection
  demoted to fallback evidence (Plan §7 Evidence authority, §8 Provider integration).
- Evidence-with-freshness model (`source`, `observed_at`, assurance), stale-hook
  degradation, restored-unconfirmed semantics (Plan §5/§7; AGENTS.md Runtime truth).
- Binding assurance `Deterministic / Attested / Manual / Heuristic`; control requires
  the first three (Plan §5).
- Single corrald owning PTYs + process lifetime + authoritative VT state + runtime
  truth; second PTY daemon rejected; upgrade via live handoff with fail-back; crash
  commitment with no-lying reconciliation (Plan §8 Runtime continuity).
- Protocol foundation from PR1: version + min-compatible + capabilities +
  snapshot@seq/sequenced deltas + resync-by-snapshot; unknown-tolerant wire invariant
  with future-input tests; opcode permanence (Plan §12; AGENTS.md Protocol/Tests).
- ANSI-replay snapshot format, corrald answers terminal queries when unattached
  (Plan §8 Terminal state; ADR 3).
- Hook shim hard invariants: millisecond fail-open, never start corrald, never slow
  the agent (Plan §8; AGENTS.md; ADR 4).
- PR0–PR8 sequence with launch-scoped hook injection (PR4/PR5) quarantined from
  global hook config mutation (PR6, degrade-to-read-only) (Plan §16).
- M1 platform scope macOS + Linux; Windows deferred, WSL2-as-a-node first (Plan §16;
  ADR 5).

Rejected (verified against this review):

- Electron-app-as-runtime / Xvfb headless model.
- A second permanent PTY daemon (absent forcing implementation evidence).
- Porting Orca's custom E2EE — prefer a Noise-style construction (`snow`) when remote
  arrives; keep Orca as the problem checklist.
- ~20-provider hook complexity — M1 is Claude + Codex only.
- Mobile/Relay/reconnect sophistication leaking into local M1 architecture.
- Orca's workspace/worktree-centric ontology — Corral remains session-first.

Deferred-with-seams (local M1 decisions that make remote cheap later): tri-state
liveness vocabulary, resync-by-snapshot as the only recovery path, capability-gated
stream evolution.
