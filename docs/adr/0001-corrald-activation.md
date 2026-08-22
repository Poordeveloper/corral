---
status: accepted
read_when:
  - changing how clients locate, start, or attach to corrald
  - touching the canonical endpoint, singleton lock, daemon lifecycle, or shutdown
  - designing the hook shim's endpoint contract (ADR 4)
  - adding a client surface that needs a daemon connection
  - adding any activation, spawn, or daemon-lifetime configuration knob
---

# corrald activation: canonical rendezvous, flock singleton, committed lifecycle

Lazy activation and may-exit-when-idle are accepted architecture
(`ARCHITECTURE.md` §7); this ADR fixes their mechanics. Scheduled by
`ROADMAP.md` §3 for PR1. Every decision below was grilled and ruled by the
founder in `docs/decisions/2026-08-22-pr1-activation-grill.md` (S1–S6);
that record is the acceptance evidence and carries the full debate.

**The invariant.** There is exactly one canonical auto-activatable primary
corrald per (host, effective OS user). Environment differences may not
change its identity or rendezvous path. Endpoint overrides may redirect a
client, but may not implicitly create another primary daemon. Test/dev
multi-instance is a future explicit namespace feature.

**D1 — Canonical rendezvous (environment-invariant).** Canonical endpoint
`<account-home>/.corral/run/corrald.sock`; canonical lock
`<account-home>/.corral/run/corrald.lock`. `<account-home>` is the
OS-account home of the effective OS user resolved from the account
database — **never `$HOME`**, never XDG, shell, session type, cron, or ssh
environment. Unresolvable account home ⇒ ConfigurationError ⇒ no
activation. `CORRAL_ENDPOINT` is a **connection override, not an instance
namespace**: the client connects there first; if unreachable, it reports
an explicit endpoint-unavailable error — no silent fallback, no auto-spawn
at a second namespace. Auto-activation is always governed by the canonical
singleton identity. The account home is read through `uzers`
(`get_effective_uid` → `get_user_by_uid` → `home_dir`), the narrow safe
wrapper approved in
`docs/decisions/2026-08-22-pr1-dependency-and-test-seam.md` (D-EX1);
direct libc is rejected because it would widen the unsafe boundary, and
`home`/`dirs`-style crates because they prefer `$HOME`, which is the defect
this decision exists to close.

**D2 — Singleton claim.** The canonical lock is held with `flock` EX
(bounded blocking wait, ~5 s) for the daemon's lifetime; the flock is the
singleton truth, the socket file only the rendezvous. The lock file is a
stable rendezvous inode: normal operation never unlinks or recreates it —
daemons only acquire/release the flock. Threat boundary, stated: same-user
deletion/replacement of the lock pathname is filesystem corruption; PR1
does not promise singleton correctness against it (two inodes ⇒ two
flocks). A daemon that loses the claim exits cleanly, touching nothing.

**D3 — Probe and stale cleanup.** A client probes with
`flock(LOCK_SH | LOCK_NB)`; the probe answers exactly one question — does
a canonical primary lock owner exist right now? EWOULDBLOCK ⇒ owner exists
⇒ the client MUST NOT spawn; retry canonical connect only. SH success ⇒
release immediately ⇒ the client MAY attempt activation (permission to
attempt, never ownership). Any other lock/open error (EACCES, …) is a
configuration/permission/filesystem error — never "daemon exists", never a
spawn licence. Only the EX-lock winner may remove a stale socket pathname,
and only after filesystem inspection confirms a Unix socket artifact; a
regular file, directory, symlink, or unexpected object there ⇒
filesystem/corruption error, fail closed, no deletion. Clients never
unlink anything.

**D4 — Activation state machine and the three facts.** Client order:
connect canonical endpoint → on an activatable failure, probe the lock →
spawn only if no owner → retry connect + handshake under **one overall
activation deadline** (never per-stage budgets that sum). Three facts are
never conflated: *primary owner exists* (EX lock held — an ownership
lease), *rendezvous reachable* (connect succeeds — a connectable
listener), *protocol ready* (the minimum handshake succeeds). Client
success = connect + successful handshake; activation eligibility =
canonical lock ownership; **socket absence is never evidence that
activation is safe**. Concurrent cold-start clients may all spawn; daemons
race the EX lock, one wins, losers exit (a loser's exit is not an
activation failure); clients converge by retrying. Daemon startup:
EX flock (bounded wait) → clean confirmed-stale socket pathname → bind →
listen → protocol-serving state. No PID file, readiness file, or endpoint
metadata; a successful handshake is the client-visible readiness evidence.

**Failure taxonomy** (layered: resolution/configuration → activation →
transport reachability → handshake/protocol):

| Condition | Result |
|---|---|
| `CORRAL_ENDPOINT` empty / relative / over socket-path limit | InvalidExplicitEndpoint — terminal, no fallback, no spawn |
| account home unresolvable | ConfigurationError — terminal |
| canonical path over socket-path limit | ConfigurationError (may suggest `CORRAL_ENDPOINT` for externally managed use) |
| run-dir create/open failure (EACCES/ENOSPC/non-directory/…) | filesystem/permission error — no spawn |
| lock open/flock error other than NB contention | filesystem/permission error — never owner-present |
| lock held + endpoint never usable before deadline | `OwnerPresentButUnreachable { lock_path, endpoint, deadline }` |
| spawn permitted + no usable daemon before deadline | `SpawnedDaemonDidNotBecomeReady { endpoint, deadline, spawn_result }` |
| handshake: valid but incompatible peer | `IncompatibleDaemon { ours, theirs, endpoint, activation_context }` — terminal-immediate, no retry/fallback/spawn/kill |
| non-socket object at socket pathname | filesystem/corruption error — no deletion |
| sibling corrald missing / not executable | InstallIntegrity/Spawn error |

No stable CLI exit codes in PR1 (a CLI compatibility surface; mapped
before M1 release). A wedged rendezvous (lock held, socket destroyed) gets
**bounded eventual recovery, not self-heal**: existing connections
continue, new clients report OwnerPresentButUnreachable, and recovery
arrives when idle exit releases the lock. Known later-phase requirement,
recorded without prescribing a mechanism: once managed work keeps corrald
alive independently of client count, this recovery path is insufficient
for a lost canonical rendezvous.

**Bootstrap handshake.** Client-first hello is the only legal first
message. `ClientHello { protocol_version, min_compatible_peer_version,
capabilities }` / `ServerHello { …, compatibility_result }`; one symmetric
predicate — `compatible(local, remote) iff remote.protocol_version >=
local.min_compatible_peer_version AND local.protocol_version >=
remote.min_compatible_peer_version` — evaluated independently by both
sides; divergent results are an internal protocol bug ⇒ fail the
connection. Required identity fields missing/type-invalid ⇒ MalformedHello
⇒ close (never "protocol 0" — malformed bootstrap and an old-but-valid
peer are different facts; `ARCHITECTURE.md` §4 is amended accordingly).
Optional/additive fields absent ⇒ documented default (capabilities ⇒
empty set); unknown fields ignored. Incompatibility is deterministic:
typed incompatible response carrying the daemon's versions, close; the
client reports facts and direction, never auto-decides
upgrade/downgrade/kill — and this holds even for a daemon the client's own
activation just spawned. The daemon distinguishes `pending_handshakes`
(bounded pre-hello deadline, 10 s default runtime policy; no client lease;
no idle influence) from `established_clients` (enter on handshake success;
the only connections that count for daemon lifetime). Unparseable framing
⇒ close without a typed reply. Capability absence is feature eligibility,
never incompatibility. PR1's capability set is empty; protocol-1 baseline
and the served surface are scoped in the PR1 plan under the frozen **no
ghost wire surface** rule (S5): PR1 assigns no wire representation to
behavior it does not serve, and exposes no application-state-mutating RPC.

**D5 — Idle lifecycle.** Idle eligibility: `established_clients == 0`
continuously for the idle grace (default 60 s — runtime tuning, not a wire
contract, not a public compatibility surface; **no public environment
variable**: a spawner's shell-local environment must not decide the
user-wide daemon's lifecycle). Pendings never start/reset/block the timer;
a pending that establishes before shutdown commit cancels the countdown.
Scope boundary only: future daemon-owned work may add reasons that make
the daemon ineligible for idle exit independently of established client
count.

**D6 — Committed shutdown and signals.** Lifecycle `Running →
ShuttingDown → Exited`; checking idle eligibility and taking Running →
ShuttingDown is one serialized atomic commit. A newly established client
may prevent idle shutdown only if establishment completes before that
commit; once committed, shutdown is never cancelled (no ShuttingDown →
Running). Path: commit → close listener → reject/close pendings → close
established connections → best-effort unlink of the canonical socket →
exit (kernel releases the flock; the lock is held to exit, so the cleanup
window still reads owner-present and forbids a second daemon).
SIGTERM/SIGINT enter the same path with immediate commit — no grace, no
goodbye message, no waiting for in-flight requests; an established
in-flight request fails as `DaemonConnectionLost` and is never
automatically replayed (replay needs command idempotency semantics PR1
does not assume); a client still in activation may continue its state
machine within the remaining deadline. SIGKILL/crash: no cleanup
guarantee — flock released by the kernel, stale pathname owned by the next
lock winner. SIGHUP: no semantics. Exit 0 on idle/signal shutdown is
operational behavior, not a stable contract. PR1 has **no durable Corral
semantic state**: a new corrald reconstructs nothing from its predecessor;
lock/socket pathnames are rendezvous artifacts, not semantic state.

**D7 — Spawn and process model.** Auto-activation resolves corrald
**sibling-only**: the real location of the running corral executable →
sibling `corrald`. Missing/not executable ⇒ InstallIntegrity error
("corrald was not found beside corral; reinstall or repair the
installation") — no `CORRAL_CORRALD_BIN`, no PATH lookup, no silent
alternate source; a sibling that starts but is wire-incompatible is
`IncompatibleDaemon`, not install integrity. corrald never daemonizes by
fork; there is no public `--foreground` — direct invocation is an
ordinary foreground process logging to stderr. Auto-activation spawns the
sibling in an internal auto-start mode (internal marker, not a stable CLI
contract): the spawner nulls stdin/stdout, points stderr at the daemon log
destination when available, and retains/reaps the Child handle while it
lives; the fresh child performs its own session detachment (setsid via
the safe OS abstraction) before starting the async runtime — no fork, no
parent daemon, no PID handoff. Hook shims never start corrald (AGENTS.md
§Runtime truth). Process hygiene: the user-wide daemon's inherited shell
environment is never authoritative configuration for Corral product
behavior. Logging: `<account-home>/.corral/log/corrald.log`, user-private,
append, no rotation in PR1 (a later operational requirement); auto-start
stdout → null, stderr/tracing → log; developer-run corrald keeps stderr on
the terminal. Log setup is best-effort and **logging is never a
correctness authority** — failure to open the log neither grants nor
denies ownership, never joins readiness, and falls back to a safe sink
surfaced as diagnostics. No raw wire/session payload dumping by default.

**Crate ownership.** The activation state machine lives in
`corral-client`; canonical rendezvous/singleton primitives live in the
narrow shared crate `corral-rendezvous` (canonical paths, OS-user home
resolution, lock/socket artifact rules, singleton helpers) — it has two
independent PR1 consumers, so it is not speculative. Direction: corral →
corral-client → corral-rendezvous → OS abstraction; corral-client →
corral-protocol; corrald → {corral-rendezvous, corral-protocol}. No
corrald → corral-client edge; no corral-core dependency in PR1.

**Test injection.** Two seams, deliberately different kinds, both behind
the non-default `test-support` feature. Normal production binaries do not
recognize either seam's environment variables; only explicit test-support
builds do — a boundary a machine can check, not a claim about build
profiles.

*Runtime policy* is explicit in code (`DaemonPolicy { idle_grace,
pre_hello_deadline }`, `ClientActivationPolicy { activation_deadline }`);
production entrypoints construct fixed defaults and tests construct values
directly. Process-level tests may supply those timing knobs as typed
scalars, never an open configuration map, never part of normal
auto-activation. No generic hidden `--internal-config` backdoor.

*The rendezvous namespace seam* is not a policy knob and not a
configuration surface: `CORRAL_TEST_ROOT` names a whole alternative Corral
root, so a test can exercise real resolution, locking, socket binding and
sibling auto-spawn without writing into the developer's own account.
Production `corral_root` is `<account home>/.corral`; a test-support build
resolves `CORRAL_TEST_ROOT` instead when set, and it must be absolute.
Substitution, never fallback — neither root is reached for when the other
fails. Everything downstream is unchanged: same path-length limit, same
private directory creation, same lock and socket artifact rules, and client
and daemon resolve through the same `corral-rendezvous` function, so the
seam cannot make them disagree about which daemon is primary. An
environment variable is the right carrier because an auto-spawned child
inherits the namespace without adding a parameter to the production
activation protocol. Ruled in
`docs/decisions/2026-08-22-pr1-dependency-and-test-seam.md` (D-EX2).

## Platform scope

This ADR decides activation for macOS and Linux on the host-OS execution
domain, including the future WSL2-as-a-node step, which reuses these Unix
mechanics unchanged. Native Windows activation is a separate future
decision under ADR 0005's re-entry trigger; nothing here pre-decides it,
and no Unix shape (paths, flock, signals) appears on the wire.

## Rejected alternatives

- **XDG_RUNTIME_DIR-based default resolution** — environment-dependent
  default rendezvous can silently partition same-node clients into
  multiple activation domains, violating the one-primary-daemon invariant.
- **`$HOME`-derived canonical paths** — the same defect via another
  variable; canonical identity follows the OS account, not the shell.
- **`CORRAL_ENDPOINT` as an instance namespace; spawn at the override** —
  an override redirects a client; it never creates a second primary.
- **`CORRAL_IDLE_EXIT_SECS` / public lifecycle env vars;
  `CORRAL_CORRALD_BIN`; PATH fallback** — shell-local environment deciding
  user-wide daemon lifecycle or binary.
- **launchd/systemd socket activation** — a login-service footprint;
  zero-background-by-default forbids it; platform-divergent.
- **localhost TCP; Linux abstract-namespace sockets** — no filesystem
  ACL; platform divergence; drift toward a default listener.
- **PID file + `kill(pid, 0)`** — PID-reuse races; flock release-on-death
  is strictly stronger.
- **Client-side stale-socket unlink; unconditional unlink of the socket
  pathname** — only the lock winner may clean, and only confirmed socket
  artifacts.
- **Endpoint registry/portfile** — a second source of truth that rots.
- **Server-first hello banner** — unsolicited bytes; a more complex
  bootstrap state machine.
- **"Absent compatibility field ⇒ protocol 0"** for required bootstrap
  fields — conflates malformed input with a genuine old peer.
- **Cancel-on-accept shutdown; abortable ShuttingDown** — raw connections
  must not influence daemon lifetime; one irreversible transition.
- **Socket-deletion polling/rebind; client kill authority** — machinery
  for self-inflicted corruption; process control smuggled into activation.
- **Self-daemonization (fork/double-fork); public `--foreground`** — one
  process model, one lifecycle.
- **Daemon instance/boot ID in PR1** — no PR1 semantic operation depends
  on distinguishing daemon processes across reconnects; can be added
  compatibly later if a concrete semantic consumer requires it.

## Not decided here

Application-level authorization (endpoint possession is not authority —
AGENTS.md §Security; M3 pairing owns authentication); upgrade live-handoff
(`ARCHITECTURE.md` §7); Remote Node Mode lifecycle; native Windows
(ADR 5); the mechanism for recovering a lost rendezvous once managed work
exists; future idle-policy configuration surfaces; CLI exit-code taxonomy.

## Consequences

- Every surface obtains activation through `corral-client`; none
  reimplements it; client and daemon share singleton identity through
  `corral-rendezvous`.
- The canonical endpoint derivation is compatibility-sensitive for the
  future hook shim (ADR 4); changing it is a compatibility event.
- corrald may exit at any idle moment: clients treat connect-refused as
  "check the lock", never as an error by itself; a daemon lost mid-request
  is an honest typed failure, never a transparent replay.
- The flock and 0700 directories are a transport fence, deliberately not a
  security boundary.

Acceptance evidence: `docs/decisions/2026-08-22-pr1-activation-grill.md`
(S1–S6), and `docs/decisions/2026-08-22-pr1-dependency-and-test-seam.md`
for the two late plan-resolution decisions (D-EX1, D-EX2) that surfaced
while the accepted plan was being resolved into an implementation — a new
third-party dependency and a supplement to the approved test-injection
surface, neither an ordinary coding detail, so both were ruled before the
implementation could rely on them.
