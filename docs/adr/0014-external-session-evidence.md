---
status: accepted
read_when:
  - deciding what a hook delivery without a launch token may claim
  - designing external-session discovery, corroboration, or the process sweep
  - deciding when an unowned runtime becomes a Run, or when it ends
  - deciding what control an externally launched session may be offered
---

> Accepted 2026-09-02, after PR7 integration grill rounds 1–4
> (`docs/decisions/2026-09-01-pr7-integration-grill.md`) and the provider
> behavior spike supporting its load-bearing claims
> (`docs/references/2026-09-02-pr7-global-integration-spike.md`).
> Remaining evidence work does not alter the accepted architecture:
> macOS upper ancestry — post-merge matrix expansion (unsealed ancestry
> is already barred from user-visible claims: missing rows under-claim,
> never over-claim); Homebrew provider channel — post-merge matrix
> expansion, promoted to a dogfood entry gate wherever that channel is
> used. A future measurement reopens an accepted decision only if it
> contradicts a load-bearing accepted assumption; ordinary matrix
> expansion does not.

# External sessions: what a token-less delivery may claim, and how an unowned runtime becomes a Run

**Supersedes in part:** ADR 0004 D5's rule that a delivery without a token is
dropped — token-less deliveries are now the external scope this ADR governs,
and D5's rule keeps governing the managed scope: an *unknown* or
*wrong-launch* token still drops. Everything else in ADR 0004 stands — the
budget, the relay's poverty, D8. On acceptance, ADR 0004 D5 gains the inline
annotation, as ADR 0009 D5 did for ADR 0010.

ADR 0004 D5 ruled that an event with no token is dropped — a law written
for a phase in which every legitimate delivery was a managed launch's. PR7
ends that premise: ADR 0013's global hook entries invoke the relay with no
launch token by construction, because there is no launch. The architecture
already anticipates the rest — discovery is idempotent, process-only
discoveries are provisional, and a Run is minted from "the node's accepted
runtime-observation mechanism" (`ARCHITECTURE.md` §1) without ever naming
that mechanism. This ADR names it for M1 and fixes what each kind of
external evidence is entitled to assert.

Structural rulings for this ADR were founder-accepted 2026-09-01
(`docs/decisions/2026-09-01-pr7-integration-grill.md`, Q4–Q5); the
post-spike rulings (Q5′–Q6′, 2026-09-02) sealed the fact-sensitive
remainder over the measured evidence, and round 4 accepted the ADR.

**The invariant.** External evidence changes what Corral can see, never
what it may do. No claim arriving over the token-less path grants control,
and no absence of evidence is read as an absent session.

## D1 — The token-less delivery, and what the relay adds to it

A global-scope relay invocation omits `--token`. `HookDelivery`'s
`launch_token` becomes optional inside `hook_protocol_version 1` —
additive evolution: token present is the managed channel unchanged; token
absent is external scope. The delivery gains optional self-observation
fields: the relay's own pid and its parent pid, read from the process
itself, costing no parsing and nothing against the D4 budget. The relay
still never reads the payload, never writes, always exits 0.

Skew, both directions: an older daemon meeting a token-less delivery fails
to decode it and drops it with diagnostics — degraded awareness on a mixed
pair, never interference, and fail-open is never conditional on being
understood. A newer daemon meeting a tokened delivery from an older relay
without the new fields treats them as absent, which means unknown.

## D2 — The runtime-observation mechanism, named

For M1 on the host OS, a runtime observation is a process identity:

```text
(pid, process start time, recognized provider executable identity)
```

observed by one of two paths:

- **The ancestry walk.** From a delivering relay's reported parent pid,
  `corrald` walks the live process tree to the nearest process recognized
  as the named provider's. The walk is daemon-side — the relay's poverty
  and budget forbid it the work — and best-effort by nature: hooks are
  short-lived children and the chain can be gone before it is read.
  Failure degrades (D3); it never blocks ingestion.
- **The sweep.** On daemon start and on a bounded periodic cadence,
  `corrald` enumerates processes and recognizes provider executables. The
  sweep is how sessions that produce no events — idle since before Corral
  started — appear at all.

Recognition rules are per-provider and version-sensitive — a CLI may
present as `node` or a wrapper, and tmux re-parents everything under its
server — so the rules are sealed by the matrix as measured fact, never
assumed from the binary's name. Start time disambiguates pid reuse
(measured: microsecond resolution on macOS).

**Where this mechanism exists.** Ruled 2026-09-02 (grill Q8′): Linux supplies
every field through `/proc`, and macOS observes no processes at all — none of
the three ways to reach the facts there was judged worth its price. The
consequence is not hidden behind the mechanism: on macOS the sweep has no
table and a delivery has nothing to corroborate it, so no external session is
discovered, and promoting an uncorroborated one anyway is exactly what Q6′
forbids. `Unobservable` is a first-class state and never collapses into
`Gone`, so what macOS loses is awareness, never truthfulness.

The post-spike ruling (grill Q5′) seals the grammar in two halves. Sealed
now, on the 2026-09-02 measurements:

- resolved executable identity/path is evidence; **raw argv[0] is never
  sufficient identity evidence** (measured: argv carries symlinks on both
  platforms);
- the provider executable may sit one runtime hop below a
  launcher/wrapper (measured: Codex's node wrapper spawns the native
  binary on both platforms); recognition follows only measured
  provider-specific shapes;
- truncated `comm` names are not primary identity evidence (measured:
  16-character truncation);
- Claude hook ancestry has the measured lower-chain shape — hook process
  → `/bin/sh -c` → provider process, two hops;
- Codex notify's measured parent relationship (the notify process's
  parent is the provider binary itself) may be used only for the exact
  claim it supports;
- arbitrary descendant-of-provider is **not** sufficient recognition
  evidence — providers spawn unrelated children such as `git`.

Not sealed: tmux/screen/nohup/general terminal-host ancestry, macOS
host-chain shapes not yet measured, and Homebrew installation shapes.
Unsealed upper-chain facts may be collected diagnostically, but they MUST
NOT contribute the evidence required for a user-visible provisional
session claim: until a matrix row is sealed, the implementation cannot say
"this ancestor chain proves a supported external provider session" — it
may still reach sufficient evidence through an independent sealed path,
such as provider integration delivery. The recognizer may know more
candidates than the UI is allowed to claim (the Q5 display-gate
invariant). Matrix expansion is additive evidence work and does not reopen
the recognizer model.

## D3 — The claim ladder for external evidence

All resolution goes through binding uniqueness on
`(node, provider, external_id, kind)` — re-observation confirms, an
unknown identity creates an **identity candidate** (see below), and
nothing ever duplicates a Session. The post-spike ruling (grill Q6′)
splits what a row is from what a row is bound to:

> **Runtime/provisional row.** A user-visible provisional row is
> justified by approved runtime-recognition evidence alone. It claims "a
> supported provider runtime appears to be running here", status Unknown.
> It does not require a provider thread/session id to exist yet.
>
> **Provider identity candidates.** Provider-emitted identities without
> promotion-grade evidence — an unknown Codex notify `thread-id` above
> all — are recorded as live/internal candidate binding evidence
> associated with the observed runtime. A new candidate identity MUST
> NOT mint an additional user-visible row merely because a delivery
> arrived.

The ruling stands on a measured fact: one Codex user turn emitted two
`agent-turn-complete` notifies, the second for an internal
title-generation turn carrying a **different `thread-id`**, structurally
indistinguishable from the real one. A notify proves *a provider thread
identity emitted an event*, not *a user-facing live session exists*; and
repetition of the same identity raises confidence in persistence without
changing the semantic type of what is observed, so "second occurrence
promotes" is explicitly rejected as a frozen rule (an internal thread may
emit twice in a future version). The measured sequence yields one runtime
row plus candidates A and B — never two Sessions flashing in the UI.

Promotion from candidate to Session binding requires evidence that the
identity represents the user-facing provider session: future
matrix-proven provider behavior, or another strong identity primitive.
Never prompt-content sniffing, never "looks like title generation", never
lexical heuristics. If PR7 ends with no strong discriminator for external
Codex notify identities, the honest M1 result is a visible runtime with
identity unresolved and identity-requiring continuation/control features
unavailable — not ghost Sessions. Managed Codex identity paths, already
proven by launch/binding evidence, are not weakened by this ruling.

> Provider-emitted identity evidence may create identity candidates; a
> user-visible Session requires evidence that supports the literal claim
> that this identity is the user's session.

The ladder, restated with the split:

```text
payload identity + corroboration     ProviderSessionBinding at Attested,
  (ancestry walk reaches a             provenance Discovered, plus a
   recognized provider process         RuntimeBinding to the observed
   Corral does not already own)        process
payload identity, no corroboration   identity candidate against the
                                       observed runtime (or held
                                       internally if none observed):
                                       read-only, never notifies, never
                                       a row by itself
approved runtime recognition only    provisional runtime row, Heuristic
  (sweep or walk, no identity)         RuntimeBinding, semantic status
                                       Unknown, no identity claim
```

The third row is gated twice, and the gates are different in kind (grill
Q5). Recognition has three tiers: **weak candidate evidence** — a loose
name match, an ambiguous wrapper, an IDE child whose role cannot be
established — is internal discovery evidence only and never a user-visible
row; **approved provisional runtime recognition** — a per-provider
recognizer sealed by the spike on argv shape, executable identity, process
relationships, and mode exclusions, precise enough to claim "there is a
supported provider runtime here" — is immediately visible as a provisional
row, with no semantic state, no identity claim beyond the evidence, no
heuristic merge, and no durable binding fabricated for display; **identity
evidence** binds per the ladder above. PRODUCT §9 reads accordingly:
supported pre-existing live sessions become visible as soon as Corral has
sufficient evidence to make that claim — not "every heuristic hit must be
visible" — because a false row damages trust more than a delayed one, and
the strict sweep and the global hooks' first event back each other up. If
the spike proves no sufficiently precise process-only recognizer exists,
the evidence threshold wins and nothing forces the display. The ruled
display gate:

> Discovery can collect weak evidence freely; user-visible rows require
> enough evidence to support the row's literal claim.

Attested here is the glossary definition verbatim — live provider-native
evidence corroborated by an observed process — and nothing weaker is
promoted to it: a payload alone proves someone fired a hook, not that the
process it names is the one observed. Provisional discoveries are linked or
superseded when identity is learned; the provider-id-keyed record wins
(`ARCHITECTURE.md` §1, now operative). Cwd and time correlation never bind
(law); the payload's cwd is a display hint, not an identity input.

A token-less delivery for a provider whose integration intent is Disabled
(ADR 0013 D6) is dropped with diagnostics: stale copies of Corral's entry
in files Corral does not manage must not keep feeding evidence the user
switched off.

## D4 — One event, two channels, one fact

A managed Claude session loads both the injected settings file and the
user's global settings, so one provider event can fire two relays: one
tokened, one token-less. The tokened managed channel stays authoritative
for launch-scoped facts. The token-less duplicate resolves through D3 to
the same binding and is re-observation — never a second Session — and its
corroboration must not mint an external Run for a process Corral already
owns: the ancestry walk ending on a managed Run's process contributes
confirmation and nothing else.

## D5 — External Runs are durable exactly when the law already says so

Durability follows fact assurance, not object existence (ADR 0002 D6), and
that rule composes here without amendment:

- A **corroborated** observation asserts an association at Attested: it
  mints a durable `RunStarted` with the OS-reported start time (an
  authoritative occurrence time) against the external `RuntimeBinding`. The
  existing event vocabulary carries it; no new event kind and no changed
  meaning — a Run has always been "one concrete runtime occurrence", and
  the binding's provenance already says Discovered.
- A **heuristic-only** runtime — sweep-discovered, never corroborated — is
  a Run that exists as live state under a Heuristic binding, and its
  association stays out of the durable log as a withheld fact. Weak
  identity never erases the runtime's existence; it only keeps the claim
  out of durable history. The provisional Session it hangs from is live
  state for the same reason: it enters the durable log when its first
  durable-grade fact does — an Attested identity, or the user's explicit
  Manual link — and a daemon restart before that honestly re-discovers
  rather than replays it.

An external Run ends only on the process table's positive answer: the
observed `(pid, start time)` no longer exists — `RunEnd::Exited` with cause
`Unknown`, because the OS says gone, not why. After daemon loss, every
formerly live external Run is re-verified on the next start and reported
exited or `Unverifiable` — the no-lying reconciliation law, now with
external sessions in its scope. Never `disconnected == exited`: a failed
walk, a missed sweep, or a daemon restart is not an exit.

## D6 — Read-only is structural, not a mode

No external runtime exposes a channel Corral owns: `corrald` did not spawn
it, holds no PTY for it, and `PRODUCT.md` §3's higher rungs are S3/PR8
material. So Open/attach answers with the honest capability fact (terminal
access absent — unknown-shaped, never evidence of death), input is never
injected into a terminal Corral does not control (law), and even an
Attested external session is See/Know only in this phase. Assurance
answers "is this really that session"; it never conjures a control channel.
The roadmap's "unsafe binding degrades to read-only" is therefore not a
special mode but existing law composed with the absence of an owned
channel — heuristic never controls, and nothing external is controllable
yet.

`session.resume` for an external session walks the existing eligibility
ladder and refuses honestly — assurance too weak, identity not Confirmed,
or exit not established — with no new vocabulary. The surface that offers
Continue in Corral for discovered sessions is PR8's; the refusal grounds
exist now.

## D7 — Succession is re-binding, never a contest

In an external terminal, one process legitimately hops conversations —
`/resume`, `claude -c`, a picker. A corroborated observation whose payload
names a different provider identity than the one last seen on that process
is succession: the runtime now evidences a different Session, the previous
Session's binding is not contradicted (its identity claim was never "and
this process is mine forever"), and nothing is contested. `BindingContested`
(ADR 0004 D8) remains a managed-launch-channel fact, where Corral
constructed the correlation a conflicting report betrays. External
RuntimeBindings are not control-capable (D6), so the at-most-one-control-
capable invariant is untouched by a runtime that evidences several Sessions
over its life.

What succession does to the Runs is ruled in concept (grill Q4), and it is
a Class C durable semantic expansion, not a projection trick:

> A Run ends when that runtime stops carrying its Session, even when the
> underlying OS process continues.

On strong succession evidence, Session A's Run ends with a **new**
`RunEnd` semantic and Session B's Run starts on the same continuing
process context. Neither existing end may be reused: `Exited` is false —
the process did not exit — and `Unverifiable` is false the other way,
because Corral holds affirmative evidence explaining exactly why A stopped
being carried; PR3's `Unverifiable` was honest because the outcome was
genuinely unknown, and abusing it here to preserve a zero-durable-diff PR7
would be an assurance word covering a known lifecycle transition. A and B
never remain concurrently open merely because the OS process is the same —
that projection would show a live Run for a Session no runtime carries —
and B's Run is durable for the same reason A's end is: observed runtime
truth may not silently leave the durable model at a succession.

The discriminant is ruled (grill Q7): `RunEnd::SessionChanged`, storage
encoding `"session-changed"`, meaning exactly this —

> The Run ended because the continuing runtime stopped carrying this
> Session identity and began carrying another Session identity.

It explicitly does not mean the OS process exited, the runtime
disappeared, or execution became unverifiable, and the doc comment locks
the second sentence: *`SessionChanged` means that the runtime continued,
but ceased to carry this Session. It never claims that the underlying OS
process exited.* It carries **no successor reference**: A's end event
states only why A's Run no longer holds; B's appearance is already
expressed by the same transaction's `SessionCreated` (when new),
`BindingAdded`/`BindingConfirmed`, and `RunStarted`. Embedding
`successor_session_id` would buy one saved join at the price of
cross-session durable coupling, a creation-ordering dependency, a
two-sided replay-consistency obligation, and binding future
succession/fork semantics to today's model; a projection that wants
"this runtime went from A to B" derives it from the atomic transition,
never from a navigation link inside A's durable event.

The transaction is ruled (grill Q8): one accepted succession observation
proves both sides of the transition and commits as **one atomic store
operation** — one SQLite transaction, projections updated in it, all or
nothing. Canonical order when B is new: A `RunEnded(SessionChanged)`;
B `SessionCreated`; B `BindingAdded`/`BindingConfirmed` as applicable;
B `RunStarted`. When B exists, the same without `SessionCreated`. The
invariant: **A-end seq < B-start seq**, and no durable-visible
intermediate state — A ended, succession known, B not yet started — may
exist merely because an implementation split one fact into two commits.
This atomicity is claimed for the succession observation only, not for
provider events in general, and retry/idempotency rides the existing
command/event machinery: no second transaction-identity scheme.

Old peers are ruled on (grill Q9), against the measured wire: the client
protocol today carries **no `RunEnd` at all** — `session.list`'s
`execution_state` is an open string (`running` / `exited` / `unknown`)
whose decoder already treats an unrecognized value as `unknown`, and no
durable-event stream has a wire surface yet (PR1's no-ghost-wire). Three
layers, kept apart: durable truth stores `SessionChanged` precisely;
the projection degrades intentionally — a succession-ended Session
projects `execution_state: "exited"`, the wire's existing "this
session's execution ended" value, because projecting a new string would
downgrade old clients to `unknown` and violate the ruled invariant —

> A newer daemon must never make an older compatible client lose the
> fact that a Run ended merely because the daemon knows a newer end
> reason.

— and any richer "moved to another conversation" fact for newer clients
arrives as an additive optional field whose absence means unknown. No
minimum-version bump is required. When a later phase gives the durable
stream a wire representation, its `RunEnd` encoding is born open —
unknown reasons decode to "ended, reason unavailable", never a decode
failure — which protects future clients and is understood to protect
nobody already shipped.

## Rejected

- **Promoting uncorroborated payloads to Attested.** Reads the glossary's
  "corroborated by an observed process" out of existence, and turns anyone
  who can invoke the relay into a session-fabricator at the assurance level
  that will later gate control.
- **Relay-side ancestry walking.** The relay's poverty is the contract
  (ADR 0004 D1); a walk is daemon work and budget risk for no gain.
- **Watching provider config/transcript files to detect live sessions.**
  Semantic evidence proves identity, never live runtime truth
  (`ARCHITECTURE.md` §1); history is a later phase's surface, and a file
  watcher asserts nothing about a process.
- **Dropping uncorroborated identity entirely.** It is honest discovery
  evidence; discarding it fails the phase's See claim for the race D2
  admits. It is kept as an identity candidate (grill Q6′) — retained,
  read-only, never a row by itself.

## Load-bearing facts to measure before acceptance

Measured 2026-09-02
(`docs/references/2026-09-02-pr7-global-integration-spike.md`) except the
per-terminal-host upper ancestry (macOS run still open — grill Q5′ seals
the lower chain only and bars unsealed upper-chain facts from
user-visible claims) and the Homebrew recognition shape. The grill
(rounds 1–4 record) ruled on the results and placed both open items as
post-merge matrix expansion (Homebrew additionally a dogfood entry gate
where that channel is used).

- Hook/notify spawn ancestry per provider, per terminal host: direct
  terminal, tmux, screen, nohup/setsid, shell-wrapper layers — does the
  walk reach a recognizable provider process before the chain dies?
- Executable recognition shapes for supported Claude/Codex versions on
  macOS and Linux (`node` wrappers, versioned paths, renamed binaries out
  of scope vs in).
- Double-fire (with ADR 0013): global + injected entries on one event, and
  arrival order.
- Sweep cost and a defensible cadence on a loaded machine.
- Platform APIs for `(pid, start time, executable)` on macOS and Linux, and
  their failure modes, behind the platform boundary.

## What this does not decide

Attention derivation and the five-state model (PR8 — everything here feeds
it as evidence with source and freshness, asserting no main state). The
recent-resumable history list (PR8) and history parsing (M2). Capability
ladder rungs 1–2 for external sessions (S3 → PR8). Remote nodes'
observation mechanisms (M3). Any correction or re-identification mechanism.
Whether the sweep should ever mint provisional Sessions for providers whose
integration is Disabled — this phase does not (Disabled means the user
asked Corral to stop watching), and revisiting that is a product decision,
not a drift.
