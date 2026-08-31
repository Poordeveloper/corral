---
status: accepted
read_when:
  - writing or changing the codex provider adapter, its launch or resume composition
  - deciding how codex evidence reaches corrald, or what a notify report may claim
  - changing how the relay receives a payload, or adding a payload delivery mode
  - deciding what happens to a user's own notify configuration in a managed launch
---

# Codex delivery: the notify channel, launch-scoped, over the same hook wire

ADR 0004 fixed hook delivery and deliberately did not decide Codex: "PR6
examines `notify` — not a hooks system — and extends or revisits per
provider." This ADR is that examination. Scheduled by `ROADMAP.md` §3 for
PR6. Acceptance evidence:
`docs/decisions/2026-08-31-pr6-codex-notify-grill.md` (one round, Q1–Q8,
the founder's rulings), whose Q6 made acceptance conditional on measuring
the load-bearing facts first — done in
`docs/references/2026-08-31-codex-0.145.0-notify-spike.md`, on top of
S2's earlier `codex exec` evidence
(`docs/references/2026-08-22-s2-session-identity-verification.md`).
Public `codex-rs` source is supporting evidence only. Accepted
2026-08-31; PR6's merge-time matrix re-verifies against the then-current
release.

Everything ADR 0004 fixed about the channel is provider-neutral and
carries unchanged: the endpoint and its trust floor (D2), the envelope and
the three-layer placement law (D3), the 50 ms interference budget (D4),
the launch token and its lifetime (D5), what the daemon may do with an
event (D7), and the contested semantics (D8). This ADR decides only what
Codex requires that Claude did not.

**The shape of the difference.** Claude has a hooks system: many events, a
config file layer that merges additively, payload on stdin. Codex has one
`notify` setting: a single program, one event family observed today,
payload delivered as a process argument, and a value that replaces rather
than merges. Every decision below is one of those four differences ruled
on.

## D1 — The channel is `notify`, injected per launch as a config override

A managed Codex launch runs the interactive `codex` the user installed,
under a Corral-owned PTY, with `-c notify=[…]` composing the relay
invocation — token included — for that launch alone. Measured: the
interactive TUI fires top-level `notify` on turn completion (spike
scenario 1; S2 had proven only `codex exec`). No provider-owned file is
read for injection and none is written: the override lives and dies with
the argv. Unlike Claude there is no Corral-owned settings file, so the
injected-file lifecycle machinery has nothing to own for this provider —
one less artifact, not a gap.

The interactive TUI is the whole managed surface. `codex exec`, headless
batch, app-server orchestration, and CI-job semantics are out of M1
managed scope — different lifecycle, interaction, attention, approval,
and output semantics, deferred to their own phase, not banned forever.
`KnownProvider::Codex` must never be read as "all Codex surfaces
supported" (grill Q7).

Rejected: watching the rollout file
(`~/.codex/sessions/…/rollout-*.jsonl`). It is a provider-owned history
artifact: reading it as live evidence would poll a file for facts the
provider is willing to push, would make freshness a polling interval
rather than an arrival, and would put history interpretation — a later
phase's owner — inside the live evidence path. History stays
provider-owned; live state stays runtime-owned (`AGENTS.md` §Durable
state).

Rejected: `codex exec --json` or the app-server protocol as the managed
mode. Both change how the user's agent runs — a managed session is the
user's own interactive agent in a PTY, not a Corral-shaped harness around
a headless one. Do not silently substitute a different execution mode for
the one the user would have run (`AGENTS.md` §Runtime truth).

Rejected: screen heuristics. Never authoritative, never a binding
(`AGENTS.md` §Core model); PR8 weighs screen evidence under the attention
engine's authority order, not here.

## D2 — The payload rides argv, and the relay accepts that without parsing it

Codex invokes the notify program with the notification JSON appended as
exactly one final argument, and delivers nothing on stdin — measured
(spike scenario 2), not assumed from source. The relay gains one flag:
with it, the payload is the final positional argument of the invocation
and stdin is never read. Everything else in ADR 0004 D1/D3/D4 is
unchanged and deliberately uniform: verbatim bytes, the 256 KiB cap with
the oversize marker, one monotonic deadline, silence on every path, exit
0 always. Codex may prove indifferent to notify's output and exit code
where Claude is not; the poverty contract stays uniform anyway, because
two relay contracts is a drift trap and the strictest consumer sets the
bar.

Exposure, stated so nobody restates it wrongly later. A payload in a
process argument list is visible to `ps`; that exposure is created by
Codex's delivery design the moment any notify program is configured, and
Corral neither widens nor re-exposes it — the payload is never logged
verbatim and never re-execed into a child. The launch token also appears
in argv (for Claude it sits in a 0600 file), and the ruling makes the
wording strict: the launch correlation token is **not** a cross-user
authentication boundary and **must not** be documented as a secret
credential. The boundary is user-private local IPC and filesystem
permissions — the hook endpoint's 0600 mode. The token's job is to
correlate one launch with one arriving provider event, not to protect
against another process of the same OS user; it keeps high entropy and
single-launch scope, `ps` visibility is not a boundary violation, and no
authorization claim rests on its secrecy (grill Q5). No tempfile or
secret-handoff machinery is added to reduce same-user observability.

## D3 — What one event honestly supports

The event family observed today is `agent-turn-complete`, carrying
`thread-id` (measured in TUI mode; same shape as S2's exec capture, with
`client` distinguishing them). Normalized: `turn_ended`, plus identity.
That is thinner evidence than Claude's five events, and the thinness is
stated rather than padded:

- **Identity arrives at the first completed turn, not at startup.** The
  first identity-bearing report over a valid token establishes the
  `ProviderSessionBinding` at Attested — same attribution logic as
  ADR 0004 D5. A managed Codex session that exits before any turn
  completes never binds; `session.resume` answers `IdentityUnknown`.
  What Corral knows then is exactly that it lacks sufficient provider
  identity to resume — nothing is asserted about what Codex itself left
  behind. The ruled invariant: **a Corral-managed runtime may exist
  without ever acquiring a provider session identity.** Managed-runtime
  existence and provider-identity knowledge are independent facts, and a
  future handshake or readiness model must keep them independent — "no
  provider identity yet" never implies "was never managed" (grill Q3,
  Q8).
- **Only verified primitives are produced.** No start, no
  awaiting-input, no end facts exist for Codex in this phase, and
  nothing synthesizes them to align with the Claude surface. A spawned
  process is runtime truth the runtime already owns; turning it into a
  provider-reported `session_started` would fabricate evidence from a
  source not entitled to assert it. Downstream, the existing law holds
  with extra force: heuristic or unverified evidence must not produce a
  user-visible Needs You, so early Codex may know a turn completed
  without knowing input is awaited — a capability limitation, never a
  license to promote weaker evidence (grill Q2).
- **No origin discrimination.** `SessionOrigin` is read off a start event
  and Codex reports none; the field stays `None`, never `Unrecognized` —
  unreported and unrecognizable are different facts.
- **Unknown notify types are tolerated and counted.** A later Codex that
  adds types is additive under ADR 0004 D3; each new type is mapped
  deliberately or asserts nothing.

Re-observation of the same `thread-id` on later turns records
`BindingConfirmed` through the existing uniqueness path. An in-place new
thread — Codex's way of starting over inside one runtime — that reports a
different id is ADR 0004 D8 verbatim: `binding-contested`, once, durable,
monotonic. Nothing Codex-specific is added to contested semantics.

## D4 — A managed-launch capability substitution, named as such

`notify` is a single TOML value; the runtime `-c` override wins over the
user/profile/project layers — measured, not read off documentation
(spike scenario 3; public `-c` precedence bugs are exactly why). So for
the managed process, the user's own notify program does not run. The
ruling, verbatim:

> For a Corral-managed Codex process, Corral may temporarily substitute
> Codex's external turn-completion notifier with the Corral integration
> notifier for that process only. This is a managed-launch capability
> substitution, not a persistent configuration mutation.

The trade-off is owned, not explained away: Corral chooses to sacrifice
the managed process's custom-notifier compatibility in exchange for
reliable integration evidence. It is acceptable because it is
process-local (measured: no configuration residue, spike scenario 4 —
the one observed write was Codex persisting the user's own answer to its
own trust prompt, present with or without Corral), touches no
`config.toml`, profile, or project config, leaves non-managed launches
untouched, is disclosed in the managed-mode documentation, and is never
dressed up as "the user lost nothing." Corral must not pretend the
original notifier was preserved.

Rejected: chaining the user's program from the relay. It would force
effective-config parsing, storing arbitrary argv, executing
user-configured code from Corral's shim, and defining
failure/timeout/environment/cwd/signal inheritance semantics — a poverty
evidence relay becomes a general hook host. Rejected: skipping injection
when the user has a notify configured — managed evidence capability must
not depend on the user's private configuration. Deferred, not rejected:
blocking the launch for an explicit user choice; it requires reliable,
cheap, version-stable effective-config discovery as a launch
prerequisite, disproportionate today. Rejected: writing `config.toml` —
provider-owned files are read-only outside ADR 0006's machinery, which
is PR7's.

Dogfood records this trade-off. If real users demonstrably miss their
custom notify, D4 reopens; chaining would then arrive as its own
decision with its own budget, never as a quiet relay feature. The ruled
invariant:

> Managed launch may substitute an integration side effect for that
> process, but Corral must not mutate the user's persistent Codex
> configuration or pretend that the original notifier was preserved.

## D5 — Resume composes like a launch, under the same ladder

`session.resume` for a Codex session composes the interactive resume —
`codex resume <thread-id>`, the verb the provider itself prints on exit
(spike scenario 2) — with the same `-c notify` override and a fresh
token. S2 verified identity stability across `codex exec resume`; the
interactive path shares the session store and the matrix drives it
first-party before merge. Eligibility is the existing ladder unchanged —
sufficient assurance, Confirmed identity, no live Run, established exit —
plus `IdentityUnknown` for the never-bound session of D3. No provider
external id reaches an argv while contested (ADR 0004 D8).

Caller arguments follow Claude's criterion, not Claude's list: refuse
exactly what defeats the injection. The refusal is load-bearing, not
ornament — the last `-c notify` on an invocation wins (spike scenario
5), so a caller's later flag silently displaces Corral's. Refuse
`-c notify=…` in every spelling the CLI accepts, and whatever else the
matrix proves can displace or disable the override. The list is
version-sensitive by nature; a managed launch also has to survive
learning nothing, and does, as an identity that never binds rather than
a false one.

## What this does not decide

External Codex discovery and global integration (PR7, ADR 0006 —
including whether `notify` can be globally managed at all given D4's
single-value problem). Attention semantics and what thin Codex evidence
means for the five-state model (PR8's authority order weighs it; Codex
TUI terminal notifications / OSC signaling are a **candidate evidence
source** for that phase to investigate — no commitment is made here).
Reading rollout files for history (the history phases). A headless or
exec-mode managed Codex. The provider seam's internal shape — ruled an
implementation concern: enum with exhaustive dispatch, no `dyn` trait
(grill Q4), recorded in the PR6 plan and the seam's own docs.
