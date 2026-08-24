# Corral — Architecture

> Boundaries, invariants, and the domain glossary. Hard rules that agents
> must obey: `AGENTS.md` (this file never restates them). What Corral is:
> `PRODUCT.md`. Current-phase scope: `ROADMAP.md`. Irreversible decisions:
> `docs/adr/`. Settled subsystem decisions with evidence:
> `docs/references/architecture-benchmarks.md`.
> Derived at PR0 from `docs/history/Corral_Development_Plan_v2.0_EN.md`
> §4–§14, §18 and the founder decision records in `docs/decisions/`. Where
> this file and the retired plan disagree, this file wins.

## 1. Session identity

A Corral Session is a logical unit of AI work with independently available
facets:

```text
Session
├── Identity          Corral-owned, globally unique
├── History           provider transcript, provider-owned
├── Context
├── Runtime           PTY / process
├── Control           provider API where offered
├── Artifacts         file changes
└── Attention         derived state
```

The primary key is a Corral-minted `CorralSessionId` (UUID). Provider
session ids, pane ids, terminal ids, cwd, and `(node_id, provider_session_id)`
are never the logical identity. `node_id` scopes external bindings only.

### Bindings

External identities attach as bindings, each recording at least:

```text
corral_session_id · node_id · kind · provider/runtime · external_id
created_at · provenance · assurance · evidence_source · observed_at
```

Kinds: `ProviderSessionBinding`, `RuntimeBinding`, `TerminalBinding`,
`HistoryBinding`.

### Assurance

Discrete levels, never a floating confidence score:

```text
Deterministic   corrald spawned and owns the runtime; identity holds by
                construction
Attested        live provider-native evidence proves the binding — e.g. a
                hook event carrying the exact provider session identity,
                corroborated by an observed process
Manual          the user explicitly linked it
Heuristic       cwd / time / process / history correlation only
```

Only Deterministic, Attested, or Manual bindings may drive cross-facet
control (AGENTS.md §Core model). Whether a heuristic match was *claimed* by
provider history or *inferred* from correlation is evidence detail, not a
separate level. Assurance is re-evaluated when evidence changes; it is never
a one-time stamp.

### Binding invariants

- **Discovery is idempotent.** Re-scanning, re-watching, or restarting
  resolves a previously seen external identity to its existing Session
  through binding uniqueness on `(node, provider, external_id, kind)` —
  never a duplicate Session. Process-only discoveries are provisional and
  are linked or superseded once provider identity is learned; the
  provider-id-keyed record wins.
- **At most one control-capable runtime binding is active per Session.**
  Acquiring control requires the previous binding to have ended or to be
  explicitly superseded.
- Corral uses **link / unlink**, never merge. Corral does not merge or
  destroy provider data.

### Session outlives process

When an agent process exits and the same provider session is resumed, the
result is the **same Session with a new Run** — never a new Session:

```text
Session A
├── Run #1   process #1, exited
└── Run #2   process #2, resumed
```

`NativeResume`, `ContextHandoff`, and `RuntimeMove` stay distinct operations
and are never collapsed into a generic "resume"; only NativeResume continues
the same Session by definition. Fixed by ADR 2.

Run existence, association assurance, and control eligibility are three
separate facts:

> A Run records a concrete runtime occurrence. Its RuntimeBinding relates
> that runtime to a Session and carries the assurance of that association.
> Run existence alone never grants control eligibility.

A Run therefore carries no assurance of its own and has no grades: a
runtime observed but only heuristically associated with a Session is a Run
that exists, under a Heuristic binding, with control unavailable and
semantic status possibly Unknown. Weak identity never erases the fact that
the runtime exists. Control eligibility resolves through the binding's
assurance and the Control facet policy above — never through the presence
of a `session_id` on a Run.

A Run is minted only from independent authoritative evidence that a
concrete runtime occurrence exists or existed: Corral created the runtime,
or the node's accepted runtime-observation mechanism observed it. Semantic
evidence — a hook event, a transcript line, cwd/time correlation — proves
identity, never live runtime truth, and never mints a Run on its own.

### Observed and Managed

Observed sessions are launched outside Corral and discovered later: history,
live status, runtime attachment when deterministically identified, and
terminal control when a compatible runtime owns it. Managed sessions are
launched through Corral's runtime: deterministic identity, persistent
runtime, terminal control, create/send/interrupt/resume, reliable attention.

Corral must be useful before a user moves any work into managed sessions.
Both terms are internal vocabulary (`PRODUCT.md` §8).

### Lifecycle axes

Execution state and user organization stay separate. A completed turn or an
ended runtime says nothing about whether the user wants the Session out of
the way. `Archived` removes a Session from the active surface while
preserving identity and history. `Deleted` removes Corral-owned metadata
only and never provider-owned history — for observed sessions, deleting
inside Corral must not modify provider source data.

## 2. Evidence and attention

Agent status is evidence with source, freshness, and assurance — not an
oracle. Source ranking:

```text
provider-native hook/event        highest-assurance evidence while fresh
    ↓ explicit runtime/provider signal
    ↓ in-band signal (e.g. OSC status sequences)
    ↓ terminal/screen detection
    ↓ history/process heuristics
```

Authority is qualified by freshness (AGENTS.md §Runtime truth). Additional
architecture rules:

- Hook/provider evidence **splits by kind**. For identity and resume it is
  authoritative. For turn state it is one weighted source, never
  load-bearing: Herdr ran hook-driven state in production and rolled back to
  identity-only hooks; Orca sustains hook state only behind a large
  per-vendor normalization layer.
- PTY output activity is the default authority for *working*; pattern and
  hook evidence refine it.
- The attention engine must remain fully functional on screen plus
  PTY-activity evidence alone. Hook state transitions are additive evidence,
  never a dependency.
- Screen-detection rules are versioned manifest data (`version` /
  `min_engine_version`) loaded at runtime, so agent-UI drift is fixable
  without a binary release. M1 ships the engine and the manifest format; a
  remote manifest-update channel is deferred.
- Status restored from persistence with no live signal since is marked
  unconfirmed and treated as immediately stale until a live event confirms
  it.
- Hooks drop events — no receiver, interrupts, crashes. The model tolerates
  missed transitions rather than assuming a complete stream.
- Attention is derived in `corrald` only (AGENTS.md §Runtime truth).

Attention vocabulary is structured from day one, never a bare boolean:

```text
AttentionItem                     NeedsInputRequest
├── reason                        ├── id
├── source                        ├── session_id
├── freshness                     ├── provider/tool context
└── action?                       └── allowed_actions?
```

M1 answers needs-input by attaching the terminal and using the provider's
own TUI; structured approval UI is M2. The vocabulary is reserved now
because attention booleans cannot be upgraded into answerable requests
compatibly.

## 3. Daemon and client boundary

One node runs one primary daemon:

```text
corrald
├── session registry
├── history providers
├── attention aggregation
├── runtime      PTY/process supervision · terminal state ·
│                agent detection/status · provider-session detection
├── protocol server
└── identity
```

Desktop, Terminal/TUI, Tray, CLI, and future Mobile/Web are clients of the
same semantic model (AGENTS.md §Client / daemon boundary). A narrow internal
`RuntimeBackend` boundary keeps the design testable against another
implementation; no external runtime is a production dependency.

Machine, workspace, and provider are filters and attributes, never the
top-level hierarchy. Terminal and pane objects belong to a Session's runtime
facet and never become Corral's top-level ontology.

### Terminal state and streaming

`corrald` owns the authoritative VT screen state for every managed terminal
— one bounded emulator per session — and answers terminal queries (DA, DSR,
OSC) when no client is attached, so unattached agents never stall.

```text
subscribe → snapshot @ sequence N → sequenced raw deltas N+1 …
```

The snapshot is an ANSI replay serialization of the authoritative buffer,
not a structured cell grid: the wire never encodes any client's rendering
model. Fixed by ADR 3, with these rules:

- **Recovery has exactly one path**: gap, decode failure, or a slow client
  discards incremental state and requests a fresh snapshot.
- **Input encoding is client-side.** The client encodes keystrokes and mouse
  events using its replica emulator's live mode bits; the daemon accepts raw
  input bytes. The wire stays dumb.
- **Resize starts a new snapshot epoch.** Resize reflows the emulator, so
  replaying pre-resize bytes into a resized replica diverges. Sub-cell size
  changes are ignored; pending resizes coalesce.
- Scrollback depth and snapshot extent are wire-contract numbers (reference
  points: 10k lines default, 100k max). M1 keeps bounded in-memory
  scrollback only; no persisted scrollback.
- Daemon-sourced PTY bytes are replayed unmodified — no LF/CRLF munging
  between daemon log and client parser.
- **PTY byte streams travel on a dedicated framed data channel, never on
  the semantic RPC channel.**
- **A finished run's screen is released, not kept served.** The screen
  thread ends with the runtime it exists for; the session answers from the
  final screen that thread published, and no viewer is offered a stream
  that can never produce (ADR 0007).

Deferred until remote/mobile requires them: ACK/credit flow control, remote
backpressure, viewport claiming, paired parking, and any large binary opcode
surface.

## 4. Protocol

Mixed client/daemon versions are normal (AGENTS.md §Protocol). The hello
handshake carries, both ways, from PR1:

```text
PROTOCOL_VERSION · MIN_COMPATIBLE_PEER_VERSION ·
capabilities   # flat string set
```

- Compatibility is one symmetric predicate, evaluated independently by
  both sides: `remote.protocol_version >= local.min_compatible_peer_version`
  and vice versa. Divergent verdicts are an internal protocol bug; the
  connection fails rather than continue ambiguously compatible.
- Absence policy splits by field class
  (`docs/decisions/2026-08-22-pr1-activation-grill.md` S3): a required
  bootstrap identity field (`protocol_version`,
  `min_compatible_peer_version`) missing or type-invalid makes the hello
  malformed — a protocol violation, never an inferred version, because the
  peer's version is simply unknown. An absent optional/additive field
  means **unknown**, never a known negative: each defines its documented
  default (capabilities ⇒ empty set), and unknown future fields are
  ignored.
- Unknown-input policy is defined per kind: unknown method → explicit error;
  unknown notification → ignore and count; unknown binary opcode → drop and
  count, because silent drops otherwise present as hangs.
- The technique for tolerating additive evolution — string method ids,
  extensible envelopes, `Unknown(raw)`, `serde(other)`, capability
  negotiation — is chosen per protocol shape, not mandated globally.
- New stream opcodes, and semantics old peers cannot interpret, sit behind
  capabilities.
- **Recovery splits by stream kind** and neither model generalizes to the
  other: terminal streams recover only by discarding local state and taking
  a fresh snapshot; durable session-event streams resume by per-session
  `after` sequence-cursor replay.
- An incompatible pair fails clearly rather than silently corrupting
  behavior.

Cross-version tests dialing the previous release against the working tree
arrive once independently upgrading clients and nodes exist (M3+); the rules
above are enforced from PR1.

Endpoints — not sockets or file descriptors — are the wire-level concept.
Nothing Unix-shaped leaks into the protocol or the domain model.

## 5. Durable state

Two stores with opposite guarantees, never the same file:

```text
registry store (authoritative)          history index (derived)
├── sessions           projections      └── FTS5 over provider history,
├── bindings                                rebuildable and deletable (M2)
├── runs
├── session_lineage
├── command_receipts   client-supplied command ids / idempotency
└── session_events     Corral-owned durable semantic events,
                       per-session monotonic seq; projections
                       committed in the same transaction
```

The registry store also carries its own metadata: the schema version it was
written at, and the node it belongs to. Neither is a projection — no event
derives them, and rebuilding the projections never touches them.

The event log records only semantic facts Corral must order, replay, and
keep consistent — `SessionCreated`, `BindingAdded`, `BindingConfirmed`,
`RunStarted`, `RunEnded`, `RunAttached`, `RunDetached`,
`SessionForkedFrom`, `CommandAccepted`. It records **none of**: PTY bytes,
raw hook events, provider transcripts, derived status. Provider history
files remain the provider's source of truth; live runtime state remains
`corrald`'s live truth and is never persisted as fact.

Two laws bound what may be written (ADR 2 D6):

> The event log owns durable semantic facts. Projections may summarize
> those facts; they may not silently acquire additional durable truth.

Every persistent projection mutation is justified by an accepted durable
event; a change the accepted vocabulary cannot express waits for the phase
that extends the set. And durability follows fact assurance, not object
existence: a fact asserting an association only heuristically supported
stays out of the log. The log is append-only in seq order — event seq is
when Corral accepted a fact, occurrence time is when it happened, and a
later-accepted fact is never inserted into an earlier seq.

Clients resume durable streams with a per-session `after` cursor. Mutating
commands accept client-supplied ids, unique within the node's durable
command namespace across Sessions, clients, connections, and daemon
restarts. Reuse with the same command fingerprint returns the original
receipt without re-executing; reuse with a different fingerprint is a
conflict that executes nothing. The fingerprint covers the command kind
and its semantic inputs — never serialization, transport, or tracing
detail.

Rationale: retrofitting an event log under a CRUD store later is a full
storage migration; adding it from the start is a small increment. A generic
event framework is not the product.

Schema, event, and migration governance — the decision test, the two durable
kinds, and the `STORAGE_EPOCH` clock — is law in AGENTS.md §Durable state.

### History pipeline

```text
provider history → HistorySource → HistoryParser → normalized records
                                                 → HistoryIndex?
```

The parser does not know whether SQLite exists. Append-only formats use
source cursors (`file_id`, `offset`, `size`, `mtime`); full reparse happens
only on truncation, replacement, file-identity change, or parser anomaly.
Session history stays on the machine where it was created unless the user
explicitly moves it; remote nodes aggregate lightweight metadata and fetch
full transcripts on demand.

**Provider data is untrusted input.** Malformed history or hook payloads
degrade the affected session to unverifiable with diagnostics; they never
panic `corrald`.

## 6. Provider integration

Provider-native hooks are authoritative for provider session identity and
native resume, and are one weighted evidence source for agent state.

```text
Managed sessions (PR4/PR5)      launch-scoped hook injection; per-launch
                                settings/env pointing at corrald;
                                NO mutation of global agent configuration

Externally launched (PR6)       managed global hook configuration:
                                install / version / merge / uninstall with
                                lock and owner identity; if safe coexistence
                                with the user's existing hooks cannot be
                                proven, degrade to read-only heuristic
                                discovery
```

Hook delivery is a second versioned wire protocol — shim → local endpoint →
`corrald` — fixed by ADR 4. Its fail-open budget and the bounded
first-response lease are law (AGENTS.md §Runtime truth). Hook events fired
while `corrald` is down are lost by design; external sessions are
re-discovered on the next start through history and process scan.

**Provider-owned files and directories are read-only.** Mutating them —
installing hooks is the canonical case — happens only as a named,
disclosed, reversible operation, never as a side effect. Writes use atomic
same-directory tempfile plus rename with mode preservation, comment-
preserving structured editing, and backfill-before-overwrite.

The default-install policy, disable path, and uninstall promise are product
decisions recorded in `PRODUCT.md` §10 and ADR 6.

## 7. Runtime continuity and lifecycle

Corral is zero-background-by-default (AGENTS.md §Local-first lifecycle).
Installing the daemon binary does not register a login service, open a
network listener, or advertise discovery; the first launch of a client
lazily starts or attaches to `corrald`. Users never launch `corrald`
manually. When there are no clients, no managed work, and Remote Node Mode
is disabled, `corrald` may exit.

Remote Node Mode is explicit opt-in and reversible: it may register a
per-user service, start `corrald` at login, enable an approved
LAN/Tailscale listener, enable peer discovery, and allow paired devices to
reach the node while clients are closed.

Committed M1 continuity hierarchy:

```text
closing Desktop/TUI/Tray     never terminates managed work
corrald planned upgrade      live handoff (FD/state transfer); if takeover
                             fails the upgrade ABORTS and the old corrald
                             keeps serving — never proceed-and-drop
corrald unexpected crash     M1 does NOT guarantee managed-session survival
```

Riders that keep "no guarantee" honest:

- **No-lying reconciliation**: on the next start after a crash, every
  formerly live session is re-verified against the OS and reported exited
  (with cause when determinable) or unverifiable — never silently dropped,
  never shown as stale running.
- **A crash never kills work `corrald` does not own**: externally launched
  sessions hold their own PTYs and are re-bound on restart.
- Live handoff is a platform capability, not a protocol guarantee; the wire
  never promises it.

A separate runtime-host/PTY-keeper process is rejected for M1 and
reconsidered only if implementation evidence forces it.

## 8. Connectivity and trust

Three concerns stay separate — discovery, authentication, authorization —
and so do discovery, direct transport, and remote bootstrap/tunnel.

Direct transports (local, LAN, Tailscale, other user-owned paths) all carry
the Corral Protocol at the application layer. SSH is a different category:
a bootstrap and tunnel mechanism that parses `~/.ssh/config`, uses existing
keys, supports jump hosts and custom ports, and installs or starts
`corrald`. LAN discovery uses mDNS (`_corral._tcp.local`) for
unauthenticated discovery only and advertises minimal node metadata — never
session, project, or history content before trust. Tailscale is the first
recommended remote option, not an architectural dependency; membership in a
tailnet is not Corral authorization.

Every `corrald` generates a node keypair. First pairing uses an explicit
trust flow with QR/fingerprint support; a pairing payload may carry node
identity, endpoint hints, a short-lived pairing capability, and the protocol
version — never a reusable raw password. After pairing, Corral uses mutually
authenticated cryptography at its own application layer even over an
encrypted transport, preferring a standard Noise-style construction over a
bespoke design. Initial permissions stay coarse (view sessions, send input,
terminal, files, structured approval) — no early RBAC.

Transport identity, application identity, and authorization remain distinct
(AGENTS.md §Security).

## 9. Platform boundary

M1 targets macOS and Linux. Windows is deferred by ADR 5 with an explicit
re-entry trigger; the first Windows step is WSL2-as-a-node reusing the Unix
runtime, with native ConPTY ownership after that, and a continuity model of
job-object child lifecycle and no live handoff — never an FD-style ConPTY
handoff.

The M1 execution domain is the host OS. Containers, VMs, WSL2, and SSH
targets are future nodes: documented out of scope, not blind spots
(`2026-08-21-m1-decision-grill.md` §1).

Platform-specific behavior stays behind platform modules (AGENTS.md §Rust).

## 10. Code structure

```text
crates/
├── corral-core        domain semantics and invariants; no IO
├── corral-protocol    wire vocabulary, protocol schemas, compatibility-
│                      facing representations
├── corral-rendezvous  canonical rendezvous paths, OS-account home
│                      resolution, singleton lock/socket artifact rules
├── corral-client      shared client/core logic
├── corrald            daemon: registry, runtime, attention, protocol server
└── corral             CLI / TUI
```

Later crates (identity, crypto, history, runtime) graduate out of `corrald`
modules when a boundary proves real, not speculatively. Surfaces depend on
`corral-protocol`, never on `corral-core`; a type appearing on the wire does
not move its business semantics into the protocol crate
(`docs/ENGINEERING_WORKFLOW.md` Appendix A).

### Extension seams

Internal seams preserved from the start — `Provider`, `HistorySource`,
`RuntimeProvider`, `DiscoveryProvider`, `TransportProvider`,
`NotificationSink`, `ArtifactRenderer` — kept out-of-process-friendly, with
CLI/RPC semantics usable without Desktop, stable semantic events rather than
UI details, explicit invocation context instead of global-state scraping,
and extension-owned state separated from Corral-managed data.

These are seams, not a plugin boundary. M0/M1 implements no plugin runtime,
manager, marketplace, dynamic ABI, permission system, or sandbox, and
preselects no security model (AGENTS.md §Security).

## 11. Glossary

Domain vocabulary. New domain nouns land here in the same change that
introduces them (AGENTS.md §Existing concepts). User-facing versus internal
terms: `PRODUCT.md` §8.

| Term | Meaning |
|---|---|
| **Session** | the logical unit of AI work; Corral's primary object and the only domain noun exposed to users |
| **CorralSessionId** | Corral-minted UUID; the primary key. Never a provider id, pane id, cwd, or `(node, provider_session_id)` |
| **Run** | one concrete runtime occurrence of a Session, identified by a Corral-minted `RunId`. A Session outlives its runs; a Run carries no assurance and never by itself grants control |
| **Facet** | an independently available aspect of a Session: history, runtime, control, artifacts, attention |
| **Binding** | an edge from a Session to an external identity, carrying provenance, assurance, evidence source, and observation time |
| **Assurance** | discrete binding trust: Deterministic, Attested, Manual, Heuristic. Heuristic never controls and never notifies |
| **Provenance** | how a binding came to exist — Corral created it, discovered it, or the user linked it. Never re-evaluated, unlike the evidence supporting it |
| **Control eligibility** | whether control may be driven through a binding. Resolved from that binding's assurance and nowhere else — never from a Run |
| **Occurrence time** | when a runtime fact happened, as against the event sequence, which is when Corral accepted it. Recorded only when authoritative runtime evidence supports it; a first-observed time is never one |
| **Withheld fact** | a fact deliberately kept out of the durable log because the association it would assert is only heuristically supported. The thing it describes still exists |
| **Evidence** | a status observation with source, `observed_at`, and assurance. Authority is qualified by freshness |
| **AttentionItem** | a structured reason a Session needs the user, with source, freshness, and optional action |
| **NeedsInputRequest** | a reserved answerable entity: a specific blocked interaction with provider/tool context and allowed actions |
| **Acknowledge** | the user has seen an attention item; held by `corrald`, consistent across surfaces |
| **Observed / Managed** | launched outside Corral versus launched through Corral's runtime. Internal vocabulary |
| **Node** | a machine running `corrald`. Scopes bindings; never part of Session identity |
| **Canonical rendezvous** | the one filesystem location where an OS account's primary `corrald` is claimed and reached. Derived from the account home; no environment variable moves it |
| **Singleton claim** | the exclusive lock a `corrald` holds for its lifetime, released by the kernel when the process dies. The claim is the singleton truth; the socket is only the meeting place |
| **Pending handshake / Established client** | a connection before versus after a successful hello. Only established clients count towards daemon lifetime |
| **Endpoint** | the wire-level concept of a reachable address. Never sockets or file descriptors in the domain model |
| **Provider** | an integrated coding-agent product with declared capabilities |
| **Capability** | a declared provider or protocol ability (`history`, `resume`, `terminal`, `structured_approval`, `terminal.stream.v1`, …). Absence means unknown, never a known negative |
| **Registry store / History index** | authoritative Corral-owned state versus derived rebuildable index. Never the same file |
| **Durable semantic event log** | the per-session ordered record of Corral-owned facts. Not event sourcing, and not the system of record for all state |
| **Storage epoch** | `dev`, `dogfood`, or `released`: which durability guarantees currently bind |
| **Command fingerprint** | the semantic identity of a mutating command — kind plus the inputs that affect the mutation. Excludes serialization, transport, and tracing detail. One command id means one immutable semantic command, for the life of the node's durable state |
| **Link / unlink** | attach or detach a binding. Corral never merges or destroys provider data |
| **NativeResume / ContextHandoff / RuntimeMove** | distinct continuation operations, never collapsed into a generic resume |
| **Session lineage** | a Corral-owned edge from a Session to the one it continued from, carrying the assurance of that claim. Recorded only where Corral knows the parent; heuristic similarity records nothing |
| **Snapshot epoch** | the screen shape a sequence is measured against. A resize reflows the emulator, so bytes recorded before it cannot be replayed into a screen shaped after it: the epoch advances and a fresh snapshot replaces the stream |
| **Terminal data channel** | the connection carrying a session's terminal frames. A second connection to the canonical rendezvous, claimed by redeeming a one-time attach token, after which it never carries semantic RPC again |
| **Attach token** | the single-use, short-lived capability that opens one terminal data channel. Bound to a Session *and* its concrete Run, because a Session outlives the process a token was minted for |
| **Final screen** | the snapshot a Run's screen thread publishes as its last act. A finished Run's screen is a value, not an actor: the emulator, its scrollback, and the thread that owned them are released when the runtime ends, and this is what a session answers from afterwards |
| **Snapshot budget / ceiling** | the encoded size a normal snapshot aims at versus the absolute bound no successful snapshot may pass. Trimming sacrifices oldest scrollback first; a viewport alone past the ceiling is a typed failure, never a partial screen |
| **Attachment seam** | the advisory record that a Corral attachment to a Run became active and later ended. It carries no holder, no client identity, and no ownership: it says a surface was watching, never who, and never that anyone had a claim. Detaching is not an end — `RunEnded` is terminal for a Run's attachment state, and a projection reads still-open attachments as inactive after it rather than inventing detaches |
| **Managed runtime binding** | the `RuntimeBinding` Corral holds for a runtime it launched itself: provider `corral`, provenance `CorralCreated`, assurance `Deterministic`, and a Corral-minted opaque external id. It names the binding, never a process — not a pid, not a `RunId`, not a runtime occurrence, and not a provider session (ADR 0008) |
| **Live synchronized control** | joining the same live provider session as a second synchronized surface; the preferred control path |
| **First-response lease** | the bounded window (≤15s) during which Corral may hold an already-blocked interaction before failing open |
| **Surface** | a client rendering the shared model: Desktop, Terminal/TUI, Tray, CLI, Mobile, Web. Holds presentation state only |
| **Local Mode / Remote Node Mode** | zero-background default versus explicitly opted-in availability |
| **Detection manifest** | versioned screen-detection rule data, loaded at runtime |

Banned synonyms: "adopt" (use Continue in Corral), "merge" for bindings (use
link), "finished" as a state, "event sourcing" for the durable log, "take
control" as a verb in code or UI.
