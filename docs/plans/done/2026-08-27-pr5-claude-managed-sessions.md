---
status: done
class: C
writes: [corrald, corral, corral-client, corral-protocol, corral-core, corral-state, corral-rendezvous, corral-tui]
reads: [docs/adr/0004-hook-delivery.md, docs/decisions/2026-08-27-pr5-hook-delivery-grill.md, docs/adr/0002-resume-lineage.md, docs/adr/0006-provider-hook-integration-policy.md, docs/adr/0007-managed-session-lifetime.md, docs/adr/0008-managed-runtime-binding-identity.md, docs/references/2026-08-22-s2-session-identity-verification.md, ARCHITECTURE.md, PRODUCT.md, ROADMAP.md]
---

# PR5 — Claude managed sessions, and the first attested evidence

**Class C, and why.** ADR 0004 is a scheduled architectural decision, the
client protocol gains a method and fields, and the durable event vocabulary
gains one kind. Every design item below was ruled in
`docs/decisions/2026-08-27-pr5-hook-delivery-grill.md` (two rounds, Q1–Q10
and R2 Q1–Q3); ADR 0004 carries the rulings and is accepted. Implementation
may proceed; the merge stays human-gated on all three grounds.

## Goal

Launch a managed Claude Code session through `corrald` with launch-scoped
hook injection; learn its provider identity from its own hooks and bind it
Attested; continue an exited session as the same Session with a new Run;
record an identity conflict durably and fail closed on it; and render the
first honest provider facts on the list surfaces — as secondary
information, with the main state untouched.

## Non-goals

No global or project provider-configuration mutation, merge, versioning, or
uninstall (PR7, ADR 0006's machinery). No external-session discovery (PR7).
No attention engine and no five-state model: no input produces Working,
Needs You, or Ready (PR8). No Codex (PR6). No fork verb; `SessionForkedFrom`
is not exercised. No `PreToolUse`/`PostToolUse` injection. No blocking hook
reply, decision-hold, or first-response lease. No needs-input actionable
surface — Respond waits for its evidence. No persistence of raw hook events.
No automatic conflict resolution and no correction / re-identification
mechanism — contested is monotonic (ADR 0004 D8); modeling in-runtime
conversation switching stays out of scope, only its conflict is recorded.
No generalized assurance-reassessment vocabulary and no Provider trait. No
resume override of any kind — no `--force`, no "I know it is dead", no pid
heuristics. No epoch advance.

## Existing owner / architecture involved

`corrald`'s `runtime/` owns spawn, launch, and attach under ADR 0007's three
lifetimes; `ManagedSessions::describe` owns the outside view of a session.
`corral-core` owns the Binding / Assurance / Evidence vocabulary —
`EvidenceSource::ProviderHook` already exists and already may not mint a
Run; `ControlEligibility` is generic binding-control eligibility and stays
so. `corral-state` owns the registry and the accepted event vocabulary,
which this PR extends by exactly `binding-contested`. `corral-rendezvous`
owns filesystem paths. `corral-protocol` owns methods and the
additive-evolution law. The PR4 projection owns what a list row may claim.
ADR 0008 D2 fixes managed RuntimeBinding reuse across resume. ADR 0004 —
this PR's decision — fixes the hook channel and the contested semantics.

## Design

**1. The provider seam — a module, not a trait.** Concrete
`provider::claude` in `corrald`, with named internal boundaries: launch
construction, resume construction, hook ingress interpretation,
provider-specific validation. It is Layer 2 of ADR 0004 D3 — the one owner
of Claude knowledge: provider-specific ingress in, normalized Evidence out.
Module boundary now; a trait only after PR6's second implementation
provides the evidence to shape one.

**2. Launch.** `SessionNewParams` gains optional `provider` and `args`,
`provider` mutually exclusive with `argv` — both or neither is a request
error. With `provider` present, `corrald` composes the final argv
(`claude --settings <injected file>` plus `args`), mints the launch token,
writes the Corral-owned settings file (ADR 0004 D6), and spawns through the
existing runtime path. The command fingerprint covers provider, args, and
cwd, exactly as it covers argv and cwd today; geometry stays excluded.

CLI is provider-first: `corral new claude [-- <provider args>]` is the
normal path, and the raw runtime harness requires the separator —
`corral new -- <cmd> [args...]`. An unknown first argument is an explicit
unknown-provider error naming the fix ("For a raw command, use:
corral new -- bash"); Corral never guesses whether it was an executable, so
the provider namespace and the raw-command namespace stay distinct. This is
a public CLI surface change, compatibility-facing and human-reviewed —
legal before an external release, but never described as an internal
refactor. TUI: the new-session prompt gains the same choice — claude, or a
raw command — nothing more; the full New Session dialog of `PRODUCT.md` §9
is a Desktop concern.

**3. Identity.** The first `SessionStart` over a valid token establishes
the `ProviderSessionBinding` — provider `claude`, `external_id` the
provider session id — at Attested, recorded via `BindingAdded`.
Re-observation of the same id resolves through binding uniqueness on
`(node, provider, external_id, kind)` and records `BindingConfirmed`. The
projection carries `identity_status: Confirmed | Contested` as its own
fact, orthogonal to assurance (ADR 0004 D8). `transcript_path` is not
persisted in PR5: no consumer exists before the history phases, and it is
re-learned from any live hook.

**4. Contested.** A valid-token identity report contradicting the confirmed
binding emits `binding-contested` — once; the event records the conflicting
reported id and the evidence, and every subsequent identity report on that
Session is diagnostics only, whichever id it names. Contested is monotonic:
nothing in PR5 clears it. It revokes exactly the authority derived from the
identity claim — `session.resume` — and nothing that rides the
Deterministic runtime binding: Open, attach, observation, and runtime-local
control stay available where otherwise valid.

**5. Continue (NativeResume).** New method
`session.resume { command_id, session_id }`: same Session, new Run, reusing
the managed RuntimeBinding's external id (ADR 0008 D2, lookup-first), a
fresh token and settings file, argv
`claude --resume <provider id> --settings <file>`. Eligibility is
operation-specific (ADR 0004 D8):

```text
NativeResumeEligibility = Eligible | AssuranceTooWeak | IdentityContested
```

Preconditions, all fail closed: sufficient assurance; `identity_status ==
Confirmed`; no live Run; and the previous Run's exit is established — an
Unverifiable end refuses with the fact stated ("Corral cannot verify that
the previous run has exited, so it will not resume this provider session
automatically"), with no override of any kind in M1. When contested, no
provider external id reaches a resume argv. The fingerprint covers
`session_id`. CLI: `corral continue <session>`; TUI: `c` on an Exited row,
then straight into Open, as `new` already does. The prior Run's final
screen is superseded by the new Run's live screen (ADR 0007 L1).

**6. Evidence on the list.** Valid events update per-session live evidence
(ADR 0004 D7). `SessionListItem` gains additive optional fields — Layer 3
of ADR 0004 D3, provider-neutral, normalized by `provider::claude`; no
provider event name reaches a client:

```text
provider:     { name, external_id? }    the identity Corral currently
                                        stands behind; external_id is
                                        withdrawn while contested
agent_event:  { kind, at }              latest still-relevant normalized
                                        fact; absent = none
    kind ∈ session_started | turn_started | turn_ended
           | awaiting_input | session_ended
```

An absent field or an unrecognized `kind` is unknown, never a negative: the
client renders no claim it cannot name. `external_id` is a current claim,
not history — after a contest it is omitted (not currently assertable, not
"never existed"), while durable history keeps both ids and their evidence
as provenance; one field never means both. Assurance is not carried — every
provider fact in PR5 rides the Attested launch channel; PR7 adds what
observed sessions need when they need it.

Presentation (copy is presentation; the semantics are frozen): the PR4
projection is untouched. The secondary line shows the latest
still-relevant provider-reported fact, past tense, with provenance and age
— "Claude reported waiting for input · 5m ago" — and a newer fact retires
the older one: after a later turn start, stop, or session end,
`awaiting_input` is no longer shown. No freshness threshold is set in PR5
(`ROADMAP.md` §9.9 waits for dogfood data); supersession is not a
threshold. It states the past, never asserts the present: no badge, no
notification, no main state, no attention item. `corral list` and the TUI
render through the one shared projection, as PR4 established.

**7. The channel.** `corral hook-relay` (ADR 0004 D1); a hook-endpoint
listener in `corrald` beside the main accept loop (D2); `RendezvousPaths`
gains the hook socket path; the framing primitive is reused. The relay
enforces the single 50 ms interference deadline with definite errors
failing open immediately (D4), and never calls the client-activation path —
asserted by test.

**8. Injected-file lifecycle.** Per-launch files in the daemon's state
directory: unique per launch, 0600, named with provenance sufficient that
cleanup can never confuse them with anything else. A Run whose exit is
established: best-effort delete. Startup cleanup may remove only files
whose owning Run is durably confirmed Exited, malformed Corral-owned files
never successfully published, and creation remnants no launch committed.
An Unverifiable owner retains the file — optionally logged as a stale
Corral-owned artifact, never destructively cleaned in PR5:

> Cleanup requires ownership evidence strong enough for the artifact being
> destroyed. An Unverifiable Run does not provide that evidence.

A post-restart token is stale secret-like material, not a deletion
justification: 0600 plus eventual confirmed cleanup suffice. The matrix
(design 9) verifies the read-once-at-startup assumption before it is ever
relied on.

**9. The matrix, re-verified first-party.** Before merge, against the
currently installed Claude Code (2.1.247 at planning time; S2's evidence is
2.1.239): `--settings` hook injection fires all five events in interactive
mode; identity holds across `--resume`, `--continue`, and the interactive
`/resume` paths S2 left as residual risk; `--settings` composes with
`--resume`; whether `--settings` is read once at startup (design 8's GC
assumption); concurrent resume of a still-running session (design 5's
refusal rationale — observed behavior, not a license); in-session
conversation switching captured for the contested path. Each scenario
records: exact Claude Code version, install/update channel where relevant,
OS, exact scenario, command/config, expected behavior, observed behavior,
Corral commit SHA, date, pass/fail/limitations. Recorded as a dated
reference in `docs/references/`. `PRODUCT.md` §10's version matrix begins
here.

**10. Glossary.** `ARCHITECTURE.md` §11 gains Hook relay, Hook endpoint,
Launch token, and Identity status (Confirmed / Contested) in the
implementation change (AGENTS §Existing concepts).

## Interfaces or persistence changed

Client protocol, all additive, no version bump — version governs breaking
change, and absence of the new method or fields is a complete answer:
`session.new` gains `provider`/`args`; `session.resume` is new;
`SessionListItem` gains `provider` and `agent_event`. Future-input coverage
extends accordingly. Human-gated regardless: the protocol surface is
touched.

Hook channel: established at v1 by ADR 0004 — a new protocol, not a change
to one.

Persistence: **one new durable event kind, `binding-contested`**, with its
schema, encoding, and projection (`identity_status`) in `corral-state` —
accepted in the grill record; `STORAGE_EPOCH` is `dev`, so no migration
obligation attaches. Nothing else: no other kinds, no reinterpretation of
existing events, `transcript_path` not persisted. The schema gate will
flag this diff; the PR cites the acceptance.

Provider-owned files: never written in this PR. Injection is a flag plus a
Corral-owned file.

## Failure / unknown states

`corrald` down when a hook fires: the event is lost by design; the relay
exits 0, silent, inside its budget. Relay over budget or on a definite
error: fail open now, exit 0. Malformed or oversize payload: diagnostics;
the session stays functional; the evidence is simply absent. Unknown or
stale token — including every token after a daemon restart: dropped with
diagnostics, never correlated heuristically. Daemon restart: live evidence
is gone, secondary facts vanish, rows return to bare runtime truth;
**contested survives restart by construction** — it is durable, and resume
stays refused. Claude binary missing or unlaunchable: the existing spawn
error path answers. Resume on an unverifiable end: refused with the fact
stated; no override. Identity conflict: `binding-contested` emitted once,
resume fails closed with `IdentityContested`, `external_id` withdrawn from
the wire, Open and attach unaffected. Claude version outside the matrix:
launch is not gated; evidence is best-effort; unknown event names are
tolerated and counted.

## Tests

- Real-format fixtures: S2's captured payloads plus current-version
  captures drive the payload parser as contract tests. No test calls a real
  provider: integration runs against the sanctioned mock-provider harness —
  a scripted stand-in binary emitting recorded hook calls.
- Hook protocol future-input: unknown envelope fields; unsupported version;
  unknown event kind; malformed JSON; the oversize marker — each asserting
  its defined behavior.
- Binding scenario/invariant: launch → `SessionStart` → one Attested
  binding; duplicate `SessionStart` idempotent; resume → `BindingConfirmed`,
  same Session, new Run, same managed external id; invalid token → nothing
  bound.
- Contested: conflict emits `binding-contested` exactly once; later reports
  of either id (or a third) emit nothing and clear nothing;
  `session.resume` refuses with `IdentityContested` and composes no argv;
  Open/attach still work; `external_id` absent from `session.list` while
  `provider.name` and `agent_event` continue. **The regression that names
  the ruling: daemon restart, and resume is still refused** — the fail-
  closed behavior survives because the fact is durable.
- Durable store: the new kind round-trips; the existing unknown-kind
  fail-closed and unknown-field-ignored behaviors cover it as future input
  for older builds' semantics.
- `NativeResumeEligibility`: exhaustive-match coverage; unverifiable end →
  refusal with the stated error; established exit → eligible.
- Injected-file lifecycle: established exit → deleted; unverifiable owner →
  retained; startup cleanup removes only the three permitted classes.
- Relay behavior: no socket / connection refused / slow ack ⇒ exit 0, empty
  stdout and stderr, no activation, bounded by a generous hard limit in the
  test; definite errors return well before the deadline. The 50 ms budget
  itself is measured evidence recorded in the PR, not a per-PR timing
  assertion — the flake law owns that trade.
- Projection regression: no hook input produces Working, Needs You, or
  Ready; supersession retires `awaiting_input` after a newer fact;
  `corral list` and the TUI render identical text for the same session.
- Wire decoding: absent new fields are unknown; an unrecognized
  `agent_event.kind` decodes and renders no claim.
- CLI: `corral new claude` launches the provider path; `corral new bash`
  errors with the raw-command hint; `corral new -- bash` unchanged.
- Idempotency: `session.new` (provider form) and `session.resume` retried
  with the same `command_id` replay their receipts; changed inputs under
  the same id are a fingerprint conflict.

## Definition of done

- Designs 1–10 implemented; `./scripts/verify` green on the final tree.
- Matrix evidence recorded in `docs/references/` with the design-9 fields;
  fixtures committed.
- Human-merged: Class C — a scheduled ADR, a touched protocol surface, and
  a durable vocabulary addition carrying its grill acceptance.
- `PRODUCT.md` §8's terminology law holds in every rendered string: Session
  is the only exposed domain noun; Binding, Assurance, Evidence, token, and
  contested-as-jargon never appear — conflict wording is neutral
  presentation.
- Glossary rows landed; the plan moves to `done/`; `STORAGE_EPOCH` is
  untouched.

## Known limitation, found in implementation

**Continuing a Session outlives the process, not the daemon.** The
provider resolves which of its own sessions an id names by the directory
it was started in, and where a Run ran is live state on its handle: no
durable event records it. So `session.resume` refuses with `NotThisDaemon`
once the daemon that launched the Session is gone — and a daemon with no
established client and no live Run exits after its idle grace, which for a
plain command-line user is about a minute after the agent stops. Inside one
daemon lifetime — the shape a person running the TUI has — continuing works
as designed.

The refusal is honest and fail-closed; substituting a directory would ask
the provider for a conversation that is not there. Repairing it needs a
durable record of where a Run ran, and `session.list` is live-only too, so
the Session would still not be nameable after a restart — which is
discovery, and discovery is PR7's. Recorded here rather than repaired
inside this PR because the fix crosses the durable-state decision boundary
and the surface it needs belongs to a later phase (`AGENTS.md` §Scope
discipline).

## Follow-ups

- A concurrency bound on accepted connections, for **both** local listeners.
  The hook endpoint spawns one task per connection with no cap, and so does
  the canonical socket it was modelled on; fixing one and not the other is
  the drift worth avoiding. Not urgent: both sit behind a `0700` directory
  and a `0600` socket, so the reach is the account's own processes, and a
  provider firing five hooks a turn is nowhere near the pressure. Measured
  when it is done, not before.
- One query for the startup sweep instead of one per file. `owner_exited`
  opens a read transaction per directory entry; measured on this machine at
  48 ms to serving with an empty launch directory and ~68 µs per retained
  file (500 → 79 ms, 2000 → 180 ms, 5000 → 386 ms). Retained files
  accumulate only on unverifiable endings and are deliberately never swept,
  so the set grows without a bound — the cost is a real slope on a small
  constant rather than a problem today.
- Where a Run ran, recorded durably, plus a way to name a Session the
  running daemon did not launch — together these are what make
  `corral continue` work across a daemon lifetime. Requires an explicitly
  accepted durable-state decision; see the limitation above.
- Supported provider/version matrix automation as a `verify-release`-owned
  task — must land before the M1 release; a one-time evidence document is
  not a permanent release gate (grill Q9).
- A correction / re-identification mechanism able to clear a contested
  binding — a future explicitly accepted decision (grill R2 Q1).
- Provider trait extraction from two real implementations — PR6 (grill Q5).

## Plan size justification

One provider integration is one coherent scope. The channel without a
producer ships a protocol nobody speaks; a launch without identity ships a
terminal, not a provider; identity without continue ships a Session that
dies with its process; and continue without the contested fact silently
resumes an identity Corral already knows is disputed. Each part exists to
keep the others honest, and the review seams stay separable: relay,
endpoint ingestion, launch and resume composition, contested semantics,
list projection, file lifecycle.
