---
status: accepted
read_when:
  - writing or changing the hook relay, its injection, or its budget
  - adding, versioning, or reinterpreting hook-channel messages or events
  - deciding what corrald may do with a hook event, or with an invalid one
  - deciding what happens when provider identity evidence conflicts
  - considering any blocking reply, hold, or lease on the hook channel
---

# Hook delivery: how provider events reach corrald, and what the channel may never do

`ARCHITECTURE.md` §6 fixes the outcome — hook delivery is a second versioned
wire protocol, shim → local endpoint → `corrald` — and `AGENTS.md` §Runtime
truth makes the fail-open guarantee law. ADR 0006 fixes the installation
policy this channel serves. This ADR fixes the mechanics under those: what
the shim is, where the endpoint lives, what a message carries, how the
contract evolves, the concrete budget, and what an identity conflict on the
channel becomes. Scheduled by `ROADMAP.md` §3 for PR5, grounded on S2's
first-party payload evidence
(`docs/references/2026-08-22-s2-session-identity-verification.md`).
Acceptance evidence: `docs/decisions/2026-08-27-pr5-hook-delivery-grill.md`
— two grill rounds, the founder's rulings verbatim; accepted 2026-08-27.

**The invariant.** The hook channel is a one-way evidence conduit. Nothing
that travels it may slow, gate, or steer the user's agent, and nothing
received over it may claim more than its source is entitled to. Both
directions are bounded: the relay's whole life fits inside a budget, and the
daemon's answer carries no instructions.

## D1 — The relay is `corral hook-relay`, and its poverty is the contract

A hidden subcommand of the installed `corral` binary. It does exactly this:
read stdin (bounded), frame one message, connect to the hook endpoint, write,
await one ack, exit 0.

It never parses the provider payload — payload drift cannot break the relay,
and semantic interpretation is not its job (D3 fixes whose job it is). It
never writes to stdout or stderr and it always exits 0 — Claude Code
interprets hook stdout and nonzero exits as decisions, so a relay that can
fail loudly is a relay that can steer the agent. It never takes the
rendezvous lock, never spawns, and never activates: shims never start
`corrald` (`AGENTS.md`).

Enforced, not aspired: tests assert the relay path performs no daemon
activation and stays silent on every failure, and D4's budget is measured
evidence in the PR that lands it.

Rejected: a separate minimal binary. A second artifact and a second
version-alignment surface, referenced from every injected settings file and —
from PR7 — from the user's global hook configuration. It is chosen only if
the `corral` CLI's startup provably cannot hold D4's budget; that evidence
reopens this decision.

## D2 — A separate hook endpoint, evidence-only by construction

`corrald` listens on a second local socket beside the canonical rendezvous
socket. `RendezvousPaths` gains the path; the daemon alone creates and
removes it, mode 0600. The listener dispatches into evidence ingestion and
nothing else: no session method and no control surface is reachable from this
endpoint. "Possession of a local RPC endpoint is not sufficient authority
for privileged control" (`AGENTS.md` §Security) is enforced structurally
rather than by method ACL.

The trust floor, stated so it cannot be over-read. The M1 Local Mode
hook-ingress authenticity floor is the same-host OS user boundary: the 0600
endpoint protects against other OS users, and it does **not** authenticate
one process belonging to that user against another process belonging to the
same user. A malicious process already executing as the same OS user is
outside the M1 local evidence-authenticity boundary — it could already
rewrite the settings files and binaries this channel is built from, so any
stronger claim here would be a fake boundary. Resisting same-user processes
is a new trust architecture (M3's adversarial work), not a PR5 patch.

The floor bounds who could forge; it never licenses accepting. The daemon
still verifies every delivery: the token is one it minted, it maps to the
launch it claims, and the facts it carries satisfy the binding rules. An
arbitrary same-user `session_id` is not accepted merely because the floor is
the OS user. And ingestion is not privileged control: control eligibility
still resolves through binding assurance and the control rules, so this
floor does not conflict with "same OS user is insufficient for privileged
control".

The relay never creates the socket. An absent socket means `corrald` is not
running, which means fail open now; events fired while `corrald` is down are
lost by design (`ARCHITECTURE.md` §6).

Rejected: multiplexing the main socket. Hook versioning would ride the
client hello, and evidence-only would be an ACL promise instead of a
structural fact.

## D3 — One connection, one message, verbatim payload — the first of three layers

One connection carries one message and one ack, over the existing framing
primitive:

```text
HookDelivery {
  hook_protocol_version    1
  launch_token             opaque, minted per launch (D5)
  provider                 "claude"
  shim_version             the relay binary's build version
  payload                  the provider's hook stdin, verbatim bytes
  payload_omitted?         "oversize" — payload dropped whole at the cap
}

HookAck {}                 receipt only; no fields, no instructions, ever
```

This wire is **provider-specific ingress**, and its place in the semantic
pipeline is fixed here, because the relay must never grow into a second
semantic engine hidden in a hook shim:

```text
provider hook wire         provider-specific facts, verbatim      (this ADR)
        ↓
corrald provider adapter   the one owner of provider knowledge:
                           raw event → normalized Evidence
        ↓
client-facing IPC          provider-neutral Corral semantics only
```

Clients never see a provider event name; the relay never assigns meaning to
one. Interpretation lives in the daemon's provider adapter — the same
placement law that keeps attention derivation in `corrald`.

`corrald` stamps arrival time itself: freshness authority belongs to the
clock of the process that judges freshness. The payload travels verbatim
because the relay is semantics-free; `corrald` parses it as untrusted input
(`ARCHITECTURE.md` §5) — malformed payloads degrade to diagnostics, never
panic, never invent. An oversized payload (cap: 256 KiB) is dropped whole
and marked, because a truncated fact would be a fabricated one, and a
systematic oversize must be visible rather than silently missing.

Evolution follows the protocol law: unknown envelope fields are ignored;
unknown event names inside the payload are tolerated and counted, asserting
nothing; an unsupported `hook_protocol_version` is dropped with diagnostics —
and the relay exits 0 regardless, because fail-open is not conditional on
being understood. Version governs breaking change; additive evolution stays
inside the version (`docs/decisions/2026-08-25-protocol-2-acceptance.md`).
Skew is normal: a settings file written at launch invokes whatever binary is
installed by the time an event fires.

## D4 — The budget: a 50 ms interference ceiling, one deadline, no second chance

50 ms is the **maximum synchronous interference budget of one hook relay
invocation**: a single monotonic deadline spanning stdin read, connect,
delivery, and the ack the protocol requires. Every phase consumes the
remaining budget; no phase resets it — there is no 10 ms connect plus 50 ms
send plus 50 ms ack. Definite errors — a missing socket, a refused
connection, a permission failure — fail open immediately rather than waiting
the budget out. There is no relay-side retry loop, no spool, no queue, no
background wait: a timeout or a transient backlog drops the event and
returns control to the provider, and the budget is never widened to mask
daemon slowness.

The budget is per invocation, deliberately. A provider that synchronously
fires five hooks in one operation can accumulate ~250 ms of interference —
a composition of the provider's calling pattern, not a budget the relay
owns. No multi-event transaction guarantee exists or is promised.

> Hook delivery is best-effort within a hard interference budget. Daemon
> slowness degrades Corral awareness, never provider progress.

At-most-once is the delivery contract; the evidence model already tolerates
missed transitions (`ARCHITECTURE.md` §2). The measured latency distribution
lands with PR5's evidence; a budget that measurement cannot hold is repaired
by repairing the relay, never by quietly widening the number.

And the boundary of what this version may hold: nothing. Protocol v1 has no
blocking reply — the ack carries receipt, never a decision. The bounded
first-response lease (≤ 15 s, `AGENTS.md` §Runtime truth) belongs to a phase
that earns interaction interception (S3 → PR7/PR8); it arrives, if it
arrives, as a protocol evolution with its own admission conditions. This ADR
grants none of it.

## D5 — The launch token is how evidence finds its Session

`corrald` mints an opaque single-launch token for every managed provider
launch and embeds it in the injected hook command line. The token maps to
the launch's (Session, Run). It is correlation evidence and protection
against accidental cross-session confusion — proof that a hook event matches
a Corral-created launch under the non-malicious-same-user threat model of
D2. It is not cryptographic authorization and not a privilege boundary: it
authorizes nothing and controls nothing.

A token resolves for as long as `corrald` remembers the launch. A daemon
restart forgets every token — the launches they named cannot have survived
it (ADR 0007 L6) — so an event bearing a forgotten token is late evidence,
dropped with diagnostics. An event arriving after its Run ended is late
evidence about a dead Run — recordable as diagnostics, never a claim the
runtime is alive (`EvidenceSource::ProviderHook` never establishes a runtime
occurrence).

Attribution is by construction — Corral minted the token into a launch it
owns — but the payload's content stays provider-claimed. So a `SessionStart`
carrying `session_id` and `transcript_path` over a valid token establishes
the `ProviderSessionBinding` at **Attested**: live provider-native evidence
corroborated by an observed process, the glossary's definition exactly. Not
Deterministic — Claude minted the id, Corral did not hold it by
construction.

An event with no token, an unknown token, or another launch's token is
dropped with diagnostics.

> **Superseded in part by ADR 0014 D1 (accepted 2026-09-02).** "No token"
> stopped meaning one thing. A delivery from a *globally installed* entry
> carries none because that entry outlives every launch and belongs to none,
> and it is taken in on its own path under its own rules. The rest of this
> paragraph is unchanged and still governs the managed channel: an unknown
> token, another launch's token, and a token this build cannot read are all
> dropped, because each of them names a launch and names it wrongly. A valid token whose payload names a different
identity than previously confirmed is the D8 conflict: recorded durably,
never merged, never moved by a payload claim alone. Never fall back to cwd
or time correlation: heuristics never bind (`AGENTS.md` §Core model), and a
guessed attribution would poison an Attested edge.

## D6 — Injection is launch-scoped and additive; provider files stay untouched

`claude --settings <file>` — first-party verified present on 2.1.247 — with
a Corral-owned per-launch file under the daemon's state directory. The
user's global and project settings load unchanged, the user's own hooks keep
running, and no strict flag is passed. No provider-owned file is written:
the read-only law (`ARCHITECTURE.md` §6) is honored by never touching them,
not by editing them carefully. Global hook configuration is PR7's problem,
with ADR 0006's machinery.

The injected file is destroyed only on ownership evidence as strong as the
destruction: its Run's established exit. An unverifiable end retains it —
loss of Corral ownership is not proof the provider process is dead (grill
Q10; lifecycle mechanics in the PR5 plan).

Events injected in PR5: `SessionStart` (identity; startup / resume / fork
discrimination), `UserPromptSubmit` (a turn began), `Stop` (a turn ended),
`SessionEnd` (the session is closing), `Notification` (the agent reports it
is blocked on the user). `PreToolUse`/`PostToolUse` are not injected:
high-frequency, and nothing in this phase consumes them; adding an event
later is additive under D3.

## D7 — What corrald does with an event, and what it never does

A valid event becomes live Evidence — source `ProviderHook`, `observed_at`
stamped at arrival — and freshness governs what it may claim from then on.
Durable writes happen only through the accepted vocabulary: `BindingAdded`
when identity is first learned, `BindingConfirmed` on re-observation, and,
on the one path D8 rules, `BindingContested`. Raw hook payloads are never
persisted as fact (`ARCHITECTURE.md` §5); tracing logs are diagnostics, not
the durable log. A daemon restart loses live evidence, and whatever is
restored without a live signal since is unconfirmed and immediately stale
(`ARCHITECTURE.md` §2): the honest answer returns to Unknown.

And the display boundary, stated where it will be looked for: hook evidence
in this phase feeds secondary presentation only — the latest still-relevant
provider-reported fact, in the past tense, with provenance and age, and
superseded by any newer fact. Main states are the attention engine's to
assert, and the engine is PR8's. The projection PR4 froze — no input
manufactures Working, Needs You, or Ready — binds every surface that renders
these facts.

## D8 — Conflicting identity is a durable fact: `binding-contested`

A previously accepted provider binding that receives contradictory
provider-identity evidence through the managed launch channel is
**contested**, and the contest is a Corral-owned durable fact — the one
narrow addition PR5 makes to the accepted event vocabulary:

```text
BindingContested {
  session, binding,
  conflicting_external_id,    what was reported — nothing more
  evidence,
}
```

`conflicting_external_id` records that this identifier was reported. It
creates no binding, merges no sessions, replaces nothing, and does not
assert the conflicting id is the correct one. This is not weaker evidence
about the same claim — it is positive evidence that two incompatible
identity claims have been observed. That is why an assurance downgrade
(Attested → Heuristic) would misdescribe it, and why a projection-only flag
is forbidden: a contest that evaporates on restart lets the next
`session.resume` silently act on an identity Corral already knows is
disputed, and the fail-closed behavior below would be a lie.

Contested is monotonic in this phase. Once emitted: later reports of the
original id do not restore, later reports of the conflicting id do not
replace, a third id creates nothing further, and no repeated transition
events are written — subsequent reports are diagnostics. Clearing contested
requires a future explicitly accepted correction / re-identification
mechanism.

What it revokes follows from where authority comes from:

> Identity contest revokes authority derived from that identity claim; it
> does not revoke unrelated authority derived from a deterministic runtime
> binding.

The projection therefore carries identity status as its own fact —
`Confirmed | Contested` — orthogonal to assurance: Attested-and-contested
is not Heuristic. Consumers derive operation-specific eligibility from it;
PR5's is

```text
NativeResumeEligibility = Eligible | AssuranceTooWeak | IdentityContested
```

and `session.resume` requires sufficient assurance, a Confirmed identity,
and every other resume precondition; contested fails closed, and no
provider external id reaches a resume argv. Open, terminal attach,
observation, and runtime-local control ride the Deterministic runtime
binding and are untouched. `ControlEligibility` in `corral-core` is generic
binding-control eligibility and stays generic — an `IdentityContested` arm
there would invite `!= Eligible → disable everything`, disabling exactly
the operations the runtime binding still honestly supports. No generalized
action-policy framework arrives in this phase.

The claim and the provenance stay separate. A client-facing `external_id`
means "the provider identity Corral currently stands behind"; after a
contest it is withdrawn — absent, meaning not currently assertable, never
meaning no id ever existed — while the durable history keeps the original
id, the conflicting report, and their evidence as provenance. One field
never means both, and a later report of the conflicting id cannot publish
itself as the current identity, clear the contest, or enable NativeResume.

> Withdraw exactly the claim that became unsafe. Do not erase the provider
> and runtime facts that remain known, and do not promote the conflicting
> claim into a replacement identity.

## What this does not decide

Codex delivery (PR6 examines `notify` — not a hooks system — and extends or
revisits per provider). Global hook installation, merge, versioning, and
uninstall (PR7, under ADR 0006). Attention derivation and the five-state
model (PR8). Any blocking reply, decision-hold, or lease semantics. The
correction / re-identification mechanism that could clear a contested
binding, and any generalized assurance-reassessment vocabulary — both wait
for a phase with the evidence to design them. What a runtime hosting an
in-session conversation switch *means*: D8 makes the conflict durable and
inert; modeling it is a later phase's question.
