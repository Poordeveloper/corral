# PR1 Founder Decision Record — Activation Grill

> Status: founder-accepted decisions from the 2026-08-22 PR1 grill of
> ADR 0001 (corrald activation) and the PR1 walking-skeleton plan.
> Materialized by `docs/adr/0001-corrald-activation.md` (which cites this
> record as acceptance evidence), the revised
> `docs/plans/2026-08-22-pr1-corrald-walking-skeleton.md`, and one
> ARCHITECTURE §4 amendment (S3). Six scenario rounds (S1–S6); each ruling
> below is final founder judgement. Companion records:
> `2026-08-21-m1-decision-grill.md`, `2026-08-21-workflow-governance-grill.md`.

## S1 — Canonical rendezvous is environment-invariant (B′)

- The three-layer default resolution (override → `$XDG_RUNTIME_DIR` →
  dotdir) is **rejected**: environment-dependent default rendezvous can
  silently partition clients of the same node into multiple activation
  domains, violating the one-primary-daemon invariant.
- Canonical default endpoint: `~/.corral/run/corrald.sock`; canonical
  singleton lock: `~/.corral/run/corrald.lock`. Neither may be affected by
  `XDG_RUNTIME_DIR`, shell, login/session type, cron, or ssh environment.
  (S6d sharpens `~` to the OS-account home, never `$HOME`.)
- `CORRAL_ENDPOINT` is a **connection override, not an instance
  namespace**. If set and the endpoint is unreachable: explicit
  endpoint-unavailable/configuration error — no silent fallback, no
  auto-spawn at a second namespace. Auto-activation is always governed by
  the canonical singleton identity.
- Invariant: **there is exactly one canonical auto-activatable primary
  corrald per (host, effective OS user)**. Environment differences may not
  change its identity or rendezvous path. Endpoint overrides may redirect
  a client but may not implicitly create another primary daemon.
- Test/dev multi-instance is a future explicit namespace feature, never
  smuggled through `CORRAL_ENDPOINT`.
- Evidence correction: tmux is a reference for stable UID-scoped
  rendezvous existing in mature tools, not proof of rejecting
  environment-dependent endpoint selection (`TMUX_TMPDIR`, `-S` exist).

## S2 — Lock, probe, readiness facts, wedge recovery

- **(a) SH probe.** The client may probe the canonical lock with
  `flock(LOCK_SH | LOCK_NB)`; it answers exactly one question: *does a
  canonical primary-daemon lock owner exist right now?* EWOULDBLOCK ⇒
  owner exists ⇒ the client MUST NOT spawn; retry canonical connect only.
  SH success ⇒ release immediately ⇒ the client MAY attempt activation.
  Any other lock/open error (e.g. EACCES) is a
  configuration/permission/filesystem error, never "daemon exists", and
  never permits a spawn. Concurrent probe-free-then-both-spawn is legal:
  spawned daemons race the canonical EX lock, one wins, losers exit, and
  every client converges by retrying the canonical endpoint — a loser's
  exit is not an activation failure. **Lock probe grants permission to
  attempt activation; it does not grant daemon ownership.** The daemon's
  EX acquisition uses a bounded blocking wait, so transient SH probes
  cannot masquerade as a second primary.
- **Lock-file invariant.** Corral never unlinks or recreates the canonical
  lock file in normal operation; it is a stable rendezvous inode — daemons
  only acquire/release the flock. Socket pathname cleanup happens only
  under the held EX lock; the lock file itself never enters cleanup.
  Same-user deletion/replacement of the lock pathname is filesystem
  corruption: PR1 does not promise singleton correctness against it (two
  inodes ⇒ two successful flocks). This threat boundary is stated, not
  hidden.
- **(b) Three facts, never one "active".** (1) *Primary owner exists* =
  canonical EX lock held — proves an ownership lease, not reachability or
  readiness. (2) *Rendezvous reachable* = connect succeeds — proves a
  connectable listener, not compatibility or readiness. (3) *Protocol
  ready* = the minimum PR1 handshake succeeds. Client success = connect +
  successful handshake; activation eligibility = canonical lock ownership.
  Daemon startup ordering: acquire EX flock (bounded wait) → only the lock
  winner may clean the stale canonical socket pathname → bind → listen →
  enter protocol-serving state. No PID file, readiness file, or endpoint
  metadata. Listen/connect is not readiness publication; a successful
  handshake is the client-visible readiness evidence. Half-start (lock
  held, socket absent/unreachable): the client MUST NOT spawn; it retries
  connect/handshake within the shared activation deadline.
- **(c) Deleted rendezvous: bounded eventual recovery, not self-heal.** If
  the canonical socket pathname is destroyed while a daemon lives:
  existing connections continue; the daemon keeps the lock; new clients
  cannot spawn a replacement; they retry until deadline, then report
  owner-present-but-unreachable. Recovery arrives only when all
  established connections close and idle exit releases the lock; a
  long-lived connection may delay it indefinitely — accepted for PR1
  (external filesystem corruption; no managed work exists; not worth
  polling/inotify/rebind/kill machinery). Rejected: B2 (filesystem
  watching / listener reconstruction), B3 (client kill authority). Known
  later-phase requirement, recorded without prescribing a mechanism: once
  managed work can keep corrald alive independently of client count, the
  idle-exit recovery path is no longer sufficient for a lost canonical
  rendezvous; that phase must explicitly decide whether such corruption is
  recoverable and by what mechanism.
- **(d) Typed activation failures**, no stable CLI exit codes in PR1:
  `OwnerPresentButUnreachable { lock_path, endpoint, deadline }` and
  `SpawnedDaemonDidNotBecomeReady { endpoint, deadline, spawn_result }`.
  Exit-code taxonomy is a CLI compatibility surface deferred to M1
  release. One **overall activation deadline** shared across
  probe/spawn/connect/handshake — never per-stage budgets that sum.
- Invariants: **Socket absence is never evidence that activation is safe;
  only absence of a canonical lock owner permits an activation attempt.**
  **Connect success is reachability evidence, not daemon readiness.**

## S3 — Bootstrap handshake and compatibility

- **(a) Client-first hello.** After connect, the client must send the
  hello request first. Minimum semantics: `ClientHello { protocol_version,
  min_compatible_peer_version, capabilities }`; `ServerHello {
  protocol_version, min_compatible_peer_version, capabilities,
  compatibility_result }`. Wire field names may distinguish client/server,
  but corral-protocol implements **one symmetric predicate**:
  `compatible(local, remote) iff remote.protocol_version >=
  local.min_compatible_peer_version AND local.protocol_version >=
  remote.min_compatible_peer_version`. Both sides evaluate independently
  (daemon on ClientHello, client on ServerHello); divergent results are an
  internal protocol bug ⇒ fail the connection. Rejected: server-first
  banner. Capability absence is a feature-eligibility matter, never
  protocol incompatibility.
- **(b) Deterministic incompatibility.** Daemon: reply with an
  incompatible result expressible in the bootstrap envelope (carrying its
  own version/min/capabilities), then close. Client: `IncompatibleDaemon {
  ours, theirs, endpoint, activation_context }` — immediate terminal; no
  retry, no fallback, no second daemon activation, no kill authority.
  Messages state facts and direction ("client protocol 3 requires server
  >= 2; connected daemon protocol 1"), never auto-decide
  upgrade/downgrade/kill. An incompatible daemon reached right after this
  client's own activation attempt is still `IncompatibleDaemon`
  (`activation_context`: ExistingPrimary | ActivationAttempted) — it
  became reachable; it is merely unusable by this client. If a freshly
  spawned owned child ever needs special cleanup, that ownership case is
  grilled separately; kill authority is not smuggled into handshake
  policy.
- **(c) Pending vs established.** hello is the only legal first message;
  a valid non-hello first request ⇒ ProtocolViolation ⇒ close. Pre-hello
  connections have a bounded handshake deadline (10 s default —
  implementation/runtime policy, not wire contract); timeout ⇒ close, not
  a daemon failure. The daemon maintains `pending_handshakes` (deadline;
  no client lease; no idle influence) and `established_clients` (enter on
  handshake success; participate in client-count lifetime semantics) —
  raw accepted connections must not gain keepalive power (repeated-connect
  starvation). Unparseable framing ⇒ close without a typed wire error (no
  framing, no pretend semantics).
- **(d) Failure taxonomy layering** — resolution/configuration →
  activation → transport reachability → handshake/protocol.
  `IncompatibleDaemon` is terminal-immediate and does not consume the
  activation deadline. The three typed failures do not close the whole
  surface: invalid explicit endpoint, permission/filesystem error, spawn
  executable failure, malformed bootstrap, protocol violation, and
  internal bug remain distinct. Frozen: **activation may succeed while
  daemon use fails at protocol negotiation.**
- **Hello field classes (ARCHITECTURE §4 amendment).** Required bootstrap
  identity fields (`protocol_version`, `min_compatible_peer_version`)
  missing or type-invalid ⇒ MalformedHello/ProtocolViolation ⇒ close —
  never "treat as protocol 0", because the peer's version is simply
  unknown. Backward-compatible optional/additive fields absent ⇒ the
  documented default (e.g. capabilities ⇒ empty set); unknown future
  fields are ignored. Malformed bootstrap and an old-but-valid peer are
  different facts; the prior "absent field ⇒ protocol 0 kill-switch"
  sentence is amended accordingly.
- Invariants: **protocol incompatibility is deterministic and is never
  retried into compatibility; transport reachability does not imply
  protocol readiness; a transport peer gains daemon-lifetime influence
  only after completing the bootstrap handshake.**

## S4 — Idle lifecycle, committed shutdown, signals, restart state

- **(a) Idle eligibility**: `established_clients == 0` continuously for
  the idle grace (default 60 s — PR1 runtime tuning, not a wire contract
  or public compatibility surface). Pendings never start/reset/block the
  timer, but a pending that establishes before shutdown commit becomes an
  established client and cancels the countdown from that instant.
  **`CORRAL_IDLE_EXIT_SECS` rejected as a public environment variable**:
  the spawner's shell-local environment must not decide the user-wide
  daemon's lifecycle (the S1 disease reintroduced). Tests use test-only
  configuration/injected clock/constructor parameters. Future idle policy,
  if ever needed, arrives via an explicit user-wide configuration surface.
  The ADR records only the scope boundary: *future daemon-owned work may
  add reasons that make the daemon ineligible for idle exit independently
  of established client count* — no algorithms, no PR numbers.
- **(b) Committed shutdown (X, with linearization).** Explicit lifecycle
  `Running → ShuttingDown → Exited`. Checking idle eligibility and taking
  the Running → ShuttingDown transition is one serialized/atomic commit
  decision. Before commit: an establishment cancels the countdown. After
  commit: no pending may promote, no connection may cancel; shutdown never
  reverts (there is no ShuttingDown → Running). The prior draft rule "a
  connection accepted before the listener closes cancels shutdown" is
  rejected; replaced with: *a newly established client may prevent idle
  shutdown only if establishment completes before the daemon commits the
  Running → ShuttingDown transition; once shutdown is committed, it is
  never cancelled.* Shutdown path: commit → stop accepting/close listener
  → reject/close pending handshakes → close established connections →
  best-effort unlink of the canonical socket pathname → process exit →
  kernel releases the canonical flock. The lock is held until exit, so
  during the cleanup window clients see owner-present + unreachable and
  S2 forbids a second daemon.
- **(c) Signals.** SIGTERM/SIGINT enter the same committed shutdown path;
  the only difference is entry: immediate commit, no idle eligibility, no
  grace, regardless of established count. No graceful-goodbye wire
  message; no waiting for in-flight requests. An established client's
  in-flight request fails as a typed transport failure
  (`DaemonConnectionLost`) and is never automatically replayed —
  activation retry semantics apply only to establishing a usable daemon;
  replaying established RPCs needs command idempotency semantics PR1 must
  not assume. A client still in activation/bootstrap when shutdown kills
  its connection may continue its activation state machine within the
  remaining overall deadline. SIGKILL/panic/crash: no cleanup guarantee —
  kernel releases the flock, a stale socket pathname may remain, the next
  canonical lock winner owns cleanup (S2). SIGHUP: no assigned semantics.
  Exit status 0 for idle exit and handled-signal shutdown is operational
  behavior, not a stable CLI contract.
- **(d) Restart state.** Frozen: **PR1 has no durable Corral semantic
  state** — no session registry, user facts, attention state, counters,
  client state, activation history, or protocol diagnostics survive
  restart; in-process counters die with the process. Runtime artifacts
  (the stable lock pathname, socket pathname, stale socket after abnormal
  death) are rendezvous artifacts, not semantic state — so the scope guard
  reads "PR1 introduces no durable application or user state whose
  semantic meaning must survive daemon restart", not "no cross-process
  state". Restart invariant: a newly started corrald reconstructs nothing
  from its predecessor to serve PR1 correctly. PR1 creates no SQLite,
  event log, persisted counters, or registry recovery.
- Invariants: **a pending transport connection has no daemon-lifetime
  right — only successful establishment does; a client may prevent idle
  shutdown only before the Running → ShuttingDown commit; once shutdown is
  committed, no connection can cancel it; PR1 daemon restart loses no
  durable Corral semantic state, because PR1 owns none.**

## S5 — PR1 wire surface

- **(a) Serving surface**: bootstrap `hello`; established RPC `ping`
  (trivial acknowledgement, no product facts) and `session.list` (empty
  list; proves request/response dispatch and typed results, no registry
  semantics). Nothing else. **No ghost wire surface**: PR1 MUST NOT assign
  wire representations, discriminants, serialized variants, or
  compatibility fixtures for subscribe/live-event/durable-event messages
  it does not serve. Future stream concepts may live in ARCHITECTURE
  prose/glossary/domain vocabulary; the first phase that serves the
  behavior introduces its compatibility-facing wire types. *If a type can
  be received from the wire and decoded, it is protocol surface —
  "types-only" does not exempt it.*
- **(b) Error model.** Framing-level failure (unrecoverable frame
  boundary, over safety limit) ⇒ close, no typed reply. Complete frame
  but undecodable envelope ⇒ close (deliberate fail-closed bootstrap
  policy for PR1). Valid envelope, unknown request method ⇒
  `MethodNotFound` preserving the request id; connection remains usable.
  Known method, invalid params ⇒ typed request error; connection usable.
  Unknown notification ⇒ ignore (+ optional process-local counter).
  Unknown additive fields ⇒ tolerated per compatibility policy.
  Connection-state legality: hello only before establishment (anything
  else ⇒ ProtocolViolation ⇒ close); repeated hello after establishment ⇒
  ProtocolViolation ⇒ close (hello is a bootstrap transition, not an
  idempotent RPC). Directionality: PR1 daemon sends only responses —
  no server-initiated requests or notifications; a response frame with no
  matching daemon-originated request ⇒ ProtocolViolation. Max frame size:
  an implementation safety limit, not a stable wire number, never a
  hidden compatibility break — far above legitimate PR1 messages; a
  future feature that could exceed it must solve limit compatibility
  explicitly.
- **(c) Baseline vs capabilities.** Protocol 1 baseline = bootstrap
  framing/envelope semantics, hello/version negotiation, ping,
  session.list, baseline typed-error behavior. PR1 ServerHello capability
  set: **empty**. A later client must not infer optional feature support
  from `protocol_version == 1`; it needs an explicit compatibility signal
  (likely a feature capability such as `session-events.v1`).
  **Rejected as permanent constitution**: "all additive methods forever
  use capabilities; only removal bumps the version". Frozen instead:
  version/compatibility range governs the baseline contract; capabilities
  negotiate optional feature contracts not guaranteed by the baseline; a
  capability is a feature contract, not a method-name bitmap; future
  evolution chooses the narrowest compatible mechanism that never makes
  peers assume unsupported behavior. **MethodNotFound is a compatibility
  safety net, not the feature-negotiation mechanism.**
- **(d) No daemon instance/boot ID in PR1.** PR1 has no durable semantic
  state, no mutating RPC, no cross-request transaction; connection death
  itself reveals daemon loss; reconnect performs a full fresh handshake.
  Rejected-for-now: daemon instance/boot ID — *PR1 has no semantic
  operation whose correctness depends on distinguishing two daemon
  processes across reconnects*. It "can be added compatibly later if a
  concrete semantic consumer requires it" (not "zero-cost"). Later
  snapshot-epoch or durable-cursor identity decisions are not pre-bound
  either way.
- **(e) Read-only guard.** Frozen: **PR1 exposes no
  application-state-mutating RPC** (hello mutates connection protocol
  state, not Corral application state). No create/update/delete,
  acknowledge, control/answer/approve, registry or durable mutation, or
  replay-duplicable commands. Worded as a scope guard, not a permanent
  rule: *the first application-mutating RPC must land together with the
  command identity/replay/idempotency semantics required by its owning
  phase; PR1 intentionally introduces none.* Established-request recovery
  stays simple because of this: a connection dying before a ping/list
  response fails the request honestly; no transparent replay machinery.

## S6 — Execution surface

- **(a) Binary resolution: sibling-only.** Auto-activation resolves
  corrald only as the sibling binary of the currently running corral
  executable (resolve the real executable location → sibling `corrald`).
  No `CORRAL_CORRALD_BIN`, no PATH/shell/XDG lookup — a shell-local
  environment must not decide which daemon binary the whole OS user talks
  to. Missing/not executable ⇒ typed install-integrity/spawn error
  ("corrald was not found beside corral; reinstall or repair the
  installation"), no fallback. A sibling that starts but is
  wire-incompatible is S3 `IncompatibleDaemon`, never reinterpreted as
  install integrity. Development needs no activation override: run corrald
  directly, or reach a test daemon via `CORRAL_ENDPOINT`.
- **(b) Detach ownership.** corrald never daemonizes by fork/double-fork;
  no public `--foreground` (direct invocation is simply a foreground
  process logging to stderr). Auto-activation spawns the sibling in an
  internal auto-start mode (internal marker/argument; not a stable CLI
  contract): the spawner sets stdin/stdout to null, stderr to the daemon
  log destination when available, starts the child, and retains/reaps the
  Child handle while the spawning surface lives; the fresh child performs
  its own session detachment (setsid via the chosen safe OS abstraction)
  before starting the async runtime, then enters the single-process
  lifecycle. No fork, no parent daemon, no PID handoff — Running →
  ShuttingDown → Exited remains the only lifecycle. Process hygiene rule,
  frozen: **the user-wide daemon's inherited shell environment is never
  authoritative configuration for future Corral product behavior**; later
  managed commands carry their execution context explicitly.
- **(c) Test injection: two levels, no production backdoor.** Runtime
  policy is explicit in code: `DaemonPolicy { idle_grace,
  pre_hello_deadline }`, `ClientActivationPolicy { activation_deadline }`;
  production entrypoints construct fixed production defaults; tests
  construct other values directly. Process-level integration tests may use
  a narrow internal test-settings input **only in a test-support
  build/configuration**, containing only the approved timing knobs
  (idle_grace, pre_hello_deadline), typed schema (no open config map),
  unavailable/rejected in production packaging, never part of normal
  auto-activation, never a user configuration mechanism. Rejected: a
  generic always-present `--internal-config <json>` backdoor. Tests
  needing short daemon timing start corrald directly through the harness.
- **(d) Resolution/configuration failure table + two corrections.**
  **Correction 1: canonical home is never derived from `$HOME`.** The
  canonical rendezvous is user-wide, so "~" means the OS-account home of
  the effective OS user (account database), not whatever HOME string the
  shell inherited — otherwise S1 returns under another variable. Frozen:
  same host + same effective OS user → same account home → same
  `~/.corral` runtime paths; unresolvable account home ⇒
  ConfigurationError ⇒ no activation; HOME may not override canonical
  daemon identity. **Correction 2: a non-socket object at the socket
  pathname is never deleted.** Only the canonical lock winner may remove a
  stale socket pathname, and only after inspection confirms it is a Unix
  socket artifact; regular file/directory/symlink/unexpected object ⇒
  filesystem/corruption error, fail closed — stale cleanup must not become
  a file-deletion primitive. Table: empty/relative/oversized explicit
  endpoint ⇒ InvalidExplicitEndpoint (terminal, no fallback, no spawn);
  unresolvable account home ⇒ ConfigurationError; canonical path over the
  Unix-socket limit ⇒ ConfigurationError (may suggest `CORRAL_ENDPOINT`
  for externally managed use); run-dir create/open failure
  (EACCES/ENOSPC/non-directory/other) ⇒ filesystem/permission error, no
  spawn; lock open/flock error other than NB contention ⇒
  filesystem/permission error, never owner-present; confirmed stale
  socket + EX-lock winner ⇒ may unlink and bind; unexpected object ⇒
  corruption error, no deletion; missing/non-executable sibling ⇒
  InstallIntegrity/Spawn error. Filesystem hardening is PR1 correctness:
  user-private runtime/log directories; lock/socket creation must not
  follow unexpected symlink substitutions; normal code never
  deletes/recreates the canonical lock file.
- **(e) Crates: corral-rendezvous joins.** The activation state machine
  (endpoint-resolution use, lock probe, spawn, shared deadline, retry,
  handshake orchestration, typed client-facing failures) belongs in
  `corral-client`; surfaces must not reimplement it. But canonical
  rendezvous/singleton primitives are not client-owned: `corral-client`
  and `corrald` need identical definitions of canonical paths, OS-user
  home resolution, lock/socket artifact rules, and filesystem safety —
  `corrald → corral-client` is the wrong direction and duplication risks
  client/daemon disagreement on singleton identity. PR1 therefore
  introduces the narrow shared crate `corral-rendezvous` (canonical
  node/user rendezvous paths; OS-user home resolution; lock/socket
  artifact rules; low-level singleton/rendezvous helpers) — justified now
  because the invariant already has two independent PR1 consumers.
  Dependency direction: corral → corral-client → corral-rendezvous → OS
  abstraction; corral-client → corral-protocol; corrald →
  corral-rendezvous, corrald → corral-protocol. No corrald → corral-client
  edge; no corral-core dependency in PR1; corral-core unchanged.
- **(f) Dependencies approved**: serde + serde_json (wire/envelope
  serialization; reject handwritten JSON); tokio (concurrent local
  connections, timers, signal handling, shutdown serialization; reject
  bespoke thread/timer machinery); rustix (flock/setsid/Unix primitives
  preserving the unsafe boundary; reject direct libc; nix valid but not
  selected); clap (CLI surface expands immediately after PR1; handwritten
  argv would be intentional short-lived code); tracing +
  tracing-subscriber (daemon diagnostics; reject a bespoke abstraction
  over log). Scoping: clap in CLI-facing binaries; tracing-subscriber at
  executable initialization; libraries emit events but never own global
  subscriber setup. All are third-party additions ⇒ HUMAN_REVIEW_REQUIRED
  ⇒ human merge for PR1.
- **(g) Logging.** Canonical auto-start log:
  `<OS-account-home>/.corral/log/corrald.log` (again: not `$HOME`);
  user-private directory and file; append. Auto-started daemon: stdout →
  null, stderr/tracing → daemon log. Direct developer-run corrald: stderr
  stays on the terminal, no forced file logging. No rotation in PR1
  (low volume, no payload logging, daemon normally idles out); rotation/
  retention recorded as a later operational requirement, not durable
  state. Frozen: **failure to open the diagnostic log never grants/denies
  primary ownership or changes semantic daemon state — logging is not a
  correctness authority**; log setup is best-effort (safe sink on
  failure; surface the failure as diagnostics via the spawning surface);
  "disk log exists" is never part of readiness. PR1 logs do not dump raw
  wire/session payloads by default; diagnostic metadata suffices.

## Closing boundary

The PR1 decision frontier is closed. Remaining unknowns are ordinary
implementation choices (type names, module layout, test-helper encoding,
tracing formatting, internal constants beyond the frozen defaults) and
must not change: user-wide daemon identity, singleton semantics,
activation authority, handshake contract, lifecycle semantics, wire
surface, durable-state scope, or human-visible compatibility behavior.

Windows: nothing here decides native Windows activation; ADR 0005's
WSL2-as-a-node path reuses these Unix mechanics unchanged. Evidence item
for ADR 0005 re-entry: measure native-Windows vs WSL provider usage
before committing the re-entry design.
