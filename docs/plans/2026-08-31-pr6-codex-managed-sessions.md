---
status: active
class: C
writes: [corrald, corral, corral-protocol]
reads: [docs/adr/0009-codex-notify-delivery.md, docs/decisions/2026-08-31-pr6-codex-notify-grill.md, docs/references/2026-08-31-codex-0.145.0-notify-spike.md, docs/adr/0004-hook-delivery.md, docs/adr/0006-provider-hook-integration-policy.md, docs/adr/0007-managed-session-lifetime.md, docs/adr/0008-managed-runtime-binding-identity.md, docs/references/2026-08-22-s2-session-identity-verification.md, docs/references/2026-08-27-pr5-claude-code-hook-matrix.md, ARCHITECTURE.md, ROADMAP.md]
---

# PR6 — Codex managed sessions, and the seam proved on a second provider

**Class C, and why.** ADR 0009 is the scheduled Codex-delivery decision,
accepted 2026-08-31 on the grill record
(`docs/decisions/2026-08-31-pr6-codex-notify-grill.md`, Q1–Q8) with its
load-bearing facts measured first
(`docs/references/2026-08-31-codex-0.145.0-notify-spike.md`). Merge is
human-gated regardless: a scheduled ADR lands, and the wire admits a new
provider name.

## Goal

Launch a managed Codex session through `corrald` with a launch-scoped
`notify` override; learn its identity from its first completed turn and
bind it Attested; continue an exited session as the same Session with a
new Run via `codex resume`; exercise the existing contested path on a
second provider; render Codex facts through the same projection — and in
doing so prove the provider seam holds two real implementations
(ROADMAP: "the second provider validates the Provider abstraction").

## Non-goals

No external-session discovery and no global notify management (PR7,
ADR 0006). No attention engine input (PR8). No rollout-file or history
reading. No chaining or preservation of the user's own notify program
(ADR 0009 D4 — and no claim that it was preserved). No new durable event
kinds and no epoch movement. No synthesized Codex facts: no start,
awaiting-input, end, or origin evidence exists for Codex in this phase
and none is invented. No new eligibility vocabulary. No Claude behavior
change beyond the seam reshape.

Managed Codex means the interactive Codex TUI under a Corral-owned PTY —
that is the whole supported surface. `codex exec`, headless batch,
app-server orchestration, and CI-job semantics are out of M1 managed
scope: different lifecycle, interaction, attention, approval, and output
semantics, deferred to their own phase, not banned forever.
`KnownProvider::Codex` must never be read as "all Codex surfaces
supported" (grill Q7).

## Existing owner / architecture involved

`corrald`'s `provider/` is the one owner of provider knowledge (ADR 0004
D3 layer 2); `provider::launch` owns tokens and injected artifacts;
`managed_launch` owns the launch/resume ladders; `hook_evidence` owns
ingress. `corral`'s `relay` owns the shim and its poverty contract.
`corral-protocol::hook` owns relay flag constants. Binding uniqueness,
`BindingAdded`/`BindingConfirmed`/`BindingContested`, and
`NativeResumeEligibility` are provider-parameterized already and are
consumed, not changed. ADR 0009 fixes every Codex-specific semantic.

## Design

**1. `provider::codex`.** The second implementation of the four named
boundaries: launch construction, resume construction, ingress
interpretation, validation. `PROVIDER = "codex"`, `PROGRAM = "codex"`,
resolved through `PATH` as Claude is. `interpret` reads
`agent-turn-complete` → `TurnEnded` plus `thread-id` → identity
(`ExternalId` refusal logged, as Claude's); unknown types are
`UnknownEvent`, tolerated and counted; no origin is ever produced.

**2. Launch.** `KnownProvider::Codex` joins the enum; every exhaustive
match extends; `ALL` grows and the unknown-provider error names both.
Launch composition emits `-c notify=[…]` first, ahead of caller args, for
PR5's reasons (nothing Corral needs may sit where caller input can reach
it). The notify value is a TOML array literal carrying the relay
invocation with `--provider codex --token <t>` and the argv-payload flag;
TOML string escaping of the binary path is owned and tested here. No
injected file exists for Codex (ADR 0009 D1). The substitution this
override performs is process-local by measured evidence (spike scenario
4) and is disclosed in the managed-mode documentation (ADR 0009 D4).

**3. The seam reshape the second provider forces.** Launch construction
currently takes a settings path — a Claude fact in a neutral signature.
It becomes a provider-owned launch plan: provider-specific argv plus an
optional provider-owned injection artifact; `managed_launch` stops
assuming a file and the file lifecycle runs only when an artifact exists.
The seam stays an enum with exhaustive dispatch — no `dyn Provider`
trait, and no plugin abstraction for a hypothetical third provider: a
new provider must face launch, evidence, identity, continuation,
capabilities, and failure semantics point by point rather than inherit
trait defaults. "Exhaustive-match friction is intentional integration
review pressure" (grill Q4, answering PR5's Q5 with evidence, recorded
in `provider/mod.rs`'s own doc).

**4. Identity.** First identity-bearing report over a valid token:
`BindingAdded` at Attested. Re-observation each turn: `BindingConfirmed`.
A different id from the same runtime — Codex's in-place new thread — is
the existing contested path, unchanged and now integration-tested on a
second provider. A session that never completes a turn never binds and
`session.resume` answers `IdentityUnknown` (already exists): a
Corral-managed runtime may exist without ever acquiring a provider
session identity (grill Q3), and what Corral knows is only that it lacks
sufficient identity to resume — nothing is claimed about what Codex left
behind.

**5. Resume.** `resume_argv` composes `resume <thread-id>` with the same
override and a fresh token. The ladder in `resume_plan` is untouched.
Argument refusal: every CLI spelling that can displace or disable the
notify override, sealed by matrix evidence, version-sensitive like
Claude's list and documented as such.

**6. Relay argv-payload mode.** One new flag constant in
`corral-protocol::hook`; with it the payload is the final positional
argument and stdin is never read (spike scenario 2: argv-only delivery,
stdin EOF). Unknown-argument tolerance, silence, exit 0, and the single
50 ms deadline are unchanged; the stdin path is byte-for-byte the PR5
relay.

**7. Evidence on the list.** No protocol schema change: `provider.name`
now also carries `"codex"`, and `agent_event` for Codex only ever shows
`turn_ended` — the projection and both surfaces render it with zero code
change, which is itself a claim design 10 tests. Secondary line reads
"Codex reported a turn ended · 2m ago"; no main state, no notification.
Only verified primitives are produced: nothing is fabricated to align
with the Claude surface (grill Q2).

**8. CLI/TUI.** `corral new codex [-- <args>]` and `corral continue`
work through the existing provider-first paths; the TUI new-session
prompt offers codex beside claude. No new surface concepts.

**9. The matrix, first-party, against the merge-time codex-cli.** The
spike measured 0.145.0; 0.151.0 already exists, so the matrix re-runs
the spike scenarios against whatever is installed at merge and adds:
caller `-c notify` spellings that displace ours (seals design 5's list);
identity across `codex resume <id>` and the interactive picker; in-place
new thread reports a new id (contested); zero-turn exit (never binds);
interference characterization (whether codex waits on the notify
process); oversize payload; config residue re-check. Recorded as a dated
reference with PR5's fields (version, channel, OS, scenario, command,
expected, observed, SHA, date, pass/fail). `PRODUCT.md` §10's matrix
gains its Codex row.

**10. Docs.** `provider/mod.rs` doc comment updated for the settled seam
shape; drift fix: `ARCHITECTURE.md` §6's "(PR6)" label on externally
launched integration reads "(PR7)" per `ROADMAP.md` (workflow §11.2). No
new glossary nouns: notify rides Hook relay, Hook endpoint, Launch token.

## Interfaces or persistence changed

Client protocol: no schema change. `session.new` accepts
`provider: "codex"`; `provider.name` may carry it. Semantic-content
review applies: PR5 clients treat the name as opaque data — asserted by a
test, not assumed. Hook wire: envelope unchanged; `provider: "codex"` is
a value, not a field. Relay invocation gains one flag — written into
launch argv by the same daemon build, but skew law still holds: an older
relay meeting the flag ignores it and reads empty stdin; the daemon drops
the empty delivery with diagnostics; fail-open is never conditional on
being understood. Persistence: nothing new; binding events are
provider-parameterized already. Provider-owned files: never read, never
written.

## Failure / unknown states

Daemon down at notify time: lost by design, relay silent. Zero-turn
session: never binds, honest `IdentityUnknown` on resume. The user's own
notify program: substituted for the managed process only — a
capability substitution, process-local, disclosed, never chained, never
claimed preserved (ADR 0009 D4). Malformed/oversize payload:
diagnostics, session unaffected. Unknown notify type: tolerated,
counted, asserts nothing. In-place new thread: contested once, durable,
resume refused, Open/attach untouched. Codex binary missing: existing
spawn error. First-run trust prompt in an untrusted directory: Codex's
own question, surfaced unchanged through the PTY; answering it is the
user's act and Codex's write (spike scenario 4). Version outside the
matrix: launch not gated, evidence best-effort. Daemon restart: tokens
forgotten, live evidence gone, contested survives — all inherited
behavior, re-asserted for Codex where the tests are cheap.

## Tests

- Real-format fixtures: the spike's captured notify payload (TUI) plus
  S2's (`exec`) drive `codex::interpret` as contract tests; the
  mock-provider harness gains a codex stand-in that delivers via argv.
- Future-input: unknown notify type; malformed JSON; missing `thread-id`
  (fact without identity); id `ExternalId` refuses; oversize marker.
- Binding scenarios: first turn-complete binds Attested; second confirms;
  differing id contests exactly once; invalid token binds nothing;
  zero-turn launch leaves no binding and resume answers `IdentityUnknown`.
- Relay: argv-payload mode delivers verbatim bytes without reading stdin;
  flag absent → stdin path unchanged; silence and exit 0 on every failure;
  no activation; deadline honored (budget itself stays measured evidence,
  not a per-PR timing assertion).
- Launch composition: override first; TOML escaping round-trips a path
  with spaces and quotes; refusal list refuses each matrix-sealed
  spelling and passes everything else; no injected file is created and
  the artifact lifecycle is a no-op for Codex.
- Resume: argv composition; contested → no external id in any argv;
  fingerprint and idempotent replay through the existing ladder.
- Projection: a Codex session shows only `turn_ended`; no input produces
  Working / Needs You / Ready; `corral list` and TUI render identically;
  a PR5-vintage decode of `provider.name: "codex"` renders it opaquely.
- CLI: `corral new codex` launches; `corral new codexx` errors naming
  both providers and the raw-command hint.

## Definition of done

- Designs 1–10 implemented; `./scripts/verify` green on the final tree.
- Matrix evidence recorded with design-9 fields; fixtures committed.
- Human-merged: Class C — a scheduled ADR and a compatibility-facing
  provider-name addition, carrying the grill acceptance.
- `PRODUCT.md` §8 terminology law holds: Session is the only exposed
  noun; notify, token, and binding never appear in rendered strings.
- Plan moves to `done/`; `STORAGE_EPOCH` untouched.

## Follow-ups

- Feed grill Q8's invariant into the open PR5 handshake question: any
  future handshake/readiness model must treat "managed runtime
  established" and "provider binding established" as independent facts —
  Codex is the standing counterexample (identity late by design;
  exit-before-identity legal). Never "no provider identity yet →
  therefore never managed".
- Dogfood records the D4 trade-off: track whether users of managed Codex
  miss their custom notify program; real complaints reopen ADR 0009 D4
  (chaining would then be its own decision with its own budget).
- Codex TUI terminal notifications / OSC signaling: a candidate evidence
  source for the attention phase to investigate — no more than that
  (grill Q2).

## Plan size justification

Slightly over target for the PR5 reason: a second provider is one
coherent scope. The relay mode without the codex adapter is a flag
nobody sets; the adapter without the seam reshape wedges a fileless
launch into a file-shaped signature; identity without resume repeats
PR5's half-session; and the abstraction claim ROADMAP assigns to PR6 is
only testable with all of it present. Review seams stay separable: relay
mode, adapter, seam reshape, matrix.
