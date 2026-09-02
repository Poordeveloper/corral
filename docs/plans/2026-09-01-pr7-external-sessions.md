---
status: active   # ADR 0013 / ADR 0014 accepted 2026-09-02 (grill rounds 1–4); merge owes the real-world fixture corpus (grill Q7′)
class: C
writes: [corrald, corral, corral-protocol, corral-state, corral-tui]
reads: [docs/adr/0004-hook-delivery.md, docs/adr/0006-provider-hook-integration-policy.md, docs/adr/0009-codex-notify-delivery.md, docs/adr/0011-conversation-attachment-is-corrals-to-authorize.md, docs/adr/0012-managed-launch-argument-grammar.md, docs/adr/0013-global-hook-integration.md, docs/adr/0014-external-session-evidence.md, docs/references/2026-08-22-s2-session-identity-verification.md, docs/references/2026-08-27-pr5-claude-code-hook-matrix.md, docs/references/2026-08-31-pr6-codex-notify-matrix.md, docs/references/architecture-benchmarks.md, ARCHITECTURE.md, PRODUCT.md, ROADMAP.md]
---

# PR7 — External sessions: discovery, and the global integration that earns it

**Class C, and why.** Two proposed ADRs cross canonical decision
boundaries: mutation of the user's provider configuration (ADR 0013),
binding-assurance/Run-minting rules for evidence Corral did not construct,
and a new `RunEnd` durable discriminant for succession (ADR 0014, grill
Q4). A node-scoped durable table joins the registry store. Structural
rulings are founder-accepted
(`docs/decisions/2026-09-01-pr7-integration-grill.md`); the ADRs move to
accepted on the post-spike round before implementation crosses either
boundary; merge is human-gated regardless. ROADMAP names PR7 the
schedule's highest-risk point — discovery coverage and safe coexistence
are both release gates here.

## Goal

Make externally launched Claude/Codex sessions appear in Corral: install
Corral's hooks into the user's global provider configuration through a
merge that is provably additive and fails safe (ADR 0013); accept
token-less relay deliveries, corroborate them against observed processes,
and bind external sessions at the assurance the evidence earns (ADR 0014);
sweep for provider processes so idle pre-existing sessions surface
honestly; render external sessions read-only beside managed ones under one
Session identity — the A-thesis made demonstrable ("Corral also sees
sessions it did not launch").

## Non-goals

No attention engine, no five-state main status, no notifications, no
recent-resumable list (PR8): every external fact renders as secondary
evidence and main status stays Unknown. No Continue-in-Corral surface for
discovered sessions (PR8; the resume refusal grounds land now). No history
parsing or transcript reading (M2). No capability-ladder rungs 1–2 (S3 →
PR8). No packaged installer or first-run dialog (M1 completion work; the
CLI runs the same named operations). No notify chaining, no writes to
occupied Codex `notify` (ADR 0013 D7). No remote nodes. No new attention
or eligibility vocabulary. `STORAGE_EPOCH` stays `dev`.

## Existing owner / architecture involved

`corrald`'s `provider/` owns provider knowledge — file shapes, entry
construction, recognition rules join it. `hook_endpoint`/`hook_evidence`
own ingress; the binding-uniqueness resolution, `BindingAdded`/
`BindingConfirmed`, and the eligibility ladder are provider-parameterized
and are consumed. `corral`'s `relay` owns the shim; `corral-protocol::hook`
owns the wire. `corral-state` owns the registry store; the session-event
vocabulary is reused, not extended. `platform.rs` is the platform boundary
the process observation code lands behind. Benchmarks ledger §7 fixes the
settled shapes: CC Switch write patterns, comment-preserving structured
merge, hook identity + runtime corroboration = Attested, history/cwd =
Heuristic read-only.

## Design

**1. The spike, first — done.** Measured 2026-09-02:
`docs/references/2026-09-02-pr7-global-integration-spike.md`. The grill
ruled over it (rounds 3–4 in
`docs/decisions/2026-09-01-pr7-integration-grill.md`) and accepted
ADR 0013/0014; every design below implements accepted architecture as
amended by rulings Q1′–Q7′ (guarded Claude invocation, per-provider
representation policy, pre-replacement validation gate, sticky repair
circuit breaker, split-sealed recognition grammar, runtime-row /
identity-candidate separation). Open evidence and its gates: real-world
fixture corpus — **this PR's merge gate** (see Definition of done); macOS
upper ancestry and the Homebrew channel — post-merge matrix expansion
(Homebrew escalates to a dogfood entry gate where that channel is used).

**2. Integration engine.** `corrald::integration`, a focused module owning
the named operations — install / repair / uninstall / status — install
triggered only by explicit `corral integration enable` during dogfood
(grill Q2; the packaged installer owns default install later) — plus
trigger evaluation, backups, and atomic writes (ADR 0013 D1–D5).
Provider file shape lives with the provider: `provider::claude` owns the
settings.json entry set and recognition; `provider::codex` owns the
`notify` value and `toml_edit`-based editing. The engine never interprets
provider semantics; the adapters never write files themselves.

**3. Integration intent.** New registry-store node-scoped table:
per-provider `Enabled | Disabled` with changed-at. Corral-owned durable
fact (ADR 0013 D6); schema diff carries the human-approval marker. Intent
gates token-less ingress (design 5) and drift repair.

**4. Relay global mode.** Token-less invocation: `--token` absent means
external scope; the relay adds its own pid and parent pid to the delivery
(ADR 0014 D1). `corral-protocol::hook` makes `launch_token` optional and
adds the self-observation fields — additive inside version 1, skew
documented and tested both ways. Poverty contract, silence, exit 0, and
the 50 ms budget unchanged.

**5. Ingress and corroboration.** `hook_evidence` accepts token-less
deliveries: intent-gated, payload parsed as untrusted input by the same
provider adapters, then the daemon-side ancestry walk from the relay's
reported parent pid (ADR 0014 D2) behind the platform boundary. Outcomes
per the D3 claim ladder: corroborated → Attested `ProviderSessionBinding`
(provenance Discovered) + external `RuntimeBinding` + durable `RunStarted`
with OS start time; uncorroborated → Heuristic, live-only. A walk ending
on a Corral-owned process confirms and mints nothing (D4 dedupe).

**6. The sweep.** Daemon-start plus bounded periodic process enumeration
recognizing provider executables; weak candidates stay internal evidence,
and only the spike-sealed high-precision recognizer mints a user-visible
provisional Session (grill Q5's display gate), under a Heuristic runtime
binding, linked or superseded when identity arrives (provider-id-keyed
record wins). Succession commits as one atomic transaction: prior Run
ends `RunEnd::SessionChanged` (`"session-changed"`, no successor
reference), successor Run starts, A-end seq < B-start seq (grill Q7/Q8). Loss of an observed `(pid, start_time)`
ends the external Run `Exited(Unknown)`; reconciliation after daemon loss
re-verifies every formerly live external Run and reports exited or
`Unverifiable`. Enumeration goes through `platform.rs`; check existing
deps/std before any new crate (a new dependency needs its one-line
justification naming alternatives).

**7. Surfacing.** Additive session facts on the client protocol: origin
(managed / discovered) and a runtime-location/cwd hint, absent meaning
unknown. `corral list` and the TUI render external rows with origin facts
per PRODUCT §8 ("Running outside Corral"), recency, provider secondary
line, and Limited awareness strings per PRODUCT §6; no main state beyond
the PR4-frozen projection. Open/attach on an external session answers the
honest terminal-access refusal. The TUI insta snapshot mandate activates
this PR (workflow §6): external rows, degraded rows, and the integration
status view land with snapshot coverage.

**8. CLI.** `corral integration status|enable|disable --provider
claude|codex` over new RPC methods (daemon executes; ADR 0013 D1). Enable
runs install and reports triggers honestly; disable runs uninstall and
sets intent. Disclosure copy follows PRODUCT §9's one-line framing.

**9. The matrix, at merge — partial.** Recorded 2026-09-02:
`docs/references/2026-09-02-pr7-integration-matrix.md`. Verified first-party
on Linux: the whole suite including the real `/proc` observation and a sweep
over the real process table; install/status/disable against a real Claude
2.1.252 configuration with the written entry inspected; a real provider
session running under Corral's guarded entries with no hook error in its UI;
idle exit behaving as designed. **Not** verified end to end: discovery
promoting a delivery to a binding, and the sweep producing a provisional row
— both need a faithful `/proc/<pid>/exe`, which neither udocker engine
provides (PRoot rewrites the link; Fakechroot breaks path interception for
the daemon's raw syscalls). Closing it needs real namespaces (rootless
Podman or Docker, needing an administrator on that host) or a machine where
the provider may run outside a container. Codex's live half waits on the
same environment. `PRODUCT.md` §10 gains the PR7 rows from that record.

**10. Docs.** Glossary adds **Integration intent**, **Runtime
observation**, and **Succession** (nouns ADR 0013/0014 introduce) in the
same change. Drift
fixes (workflow §11.2): ADR 0006's two stale "PR6" phase labels read PR7,
citing ROADMAP's renumbering; on ADR 0014's acceptance, ADR 0004 D5 gains
its superseded-in-part inline annotation. ARCHITECTURE §6's PR7 paragraph
is already correct and gains no duplicate.

## Interfaces or persistence changed

Hook wire: `launch_token` optional plus relay self-observation fields,
inside `hook_protocol_version 1`; old daemon drops token-less deliveries
with diagnostics (degraded awareness, never interference). Client
protocol: additive session facts (origin, location hint) and the
integration RPC methods — absent fields mean unknown; PR6-vintage clients
render nothing new, asserted by future-input tests. Persistence: one new
node-scoped table (integration intent), the `RunEnd::SessionChanged`
discriminant (grill Q4/Q7), and the repair circuit-breaker state —
fingerprint, repair timestamps/window, breaker-open flag, per grill Q4′:
sticky across daemon restarts, cleared only by explicit reconciliation,
never process-local — all human-gated durable diffs carrying
`DURABLE-APPROVED-BY:` (Q4′ accepted the policy; the schema still gets
its own review). The client wire carries no `RunEnd` (measured, Q9
check): a succession-ended Session projects `execution_state: "exited"`,
so no old client loses "ended" and no version bump is needed; any richer
succession fact is a later additive optional field. Otherwise external
Runs are new producers of existing events.
Provider-owned files: written only by the integration engine under
ADR 0013's rules; managed launch paths untouched.

## Failure / unknown states

Any D4 trigger → no write, Limited awareness, one surfaced resolution ask.
`disableAllHooks: true` globally → trigger, never overridden in the user's
file. Occupied Codex `notify` → preserved, degraded, disclosed. Provider
rewrite strips entries → bounded repair at boundaries, counted as
delivery-health evidence. Daemon down → events lost by design; sweep and
next delivery rediscover; no spool. Ancestry race lost → Heuristic, honest
degraded row. Same event, both channels → one Session, managed channel
authoritative. External process gone → `Exited(Unknown)`; cannot re-verify
after restart → `Unverifiable`; never `disconnected == exited`. Intent
Disabled → token-less deliveries dropped with diagnostics; sweep does not
mint for that provider. Unsupported provider version → outside the
guarantee: best-effort display, no semantic claims, direction-aware call
to action (PRODUCT §6). Org-managed policy silencing hooks → undetectable
in M1, recorded limitation. Conversation hop in an external terminal →
succession re-binding, never contested: prior Run ends with the succession
`RunEnd`, successor Run starts (ADR 0014 D7). Corral binary removed with
entries left behind → installer obligation plus the fail-open invariant
(ADR 0013 D8); provider behavior measured, and visible per-event
disruption is a stop on the default-install shape.

## Tests

- Real-format fixtures: the settings corpus drives the merge engine —
  every D4 trigger, comment/format preservation byte-compared, backup and
  atomic-write behavior, version-discriminant refusal (newer than binary).
- Integration (MUST): install → external Claude session discovered end to
  end via mock-provider harness firing token-less deliveries; corroborated
  Attested bind; uncorroborated Heuristic; sweep-then-identity
  link/supersede idempotence across restart; double-fire dedupe against a
  live managed session; disable → ingress dropped and entries removed;
  drift (missing Corral-owned entry) → repair once; non-Corral content in
  Corral's slot → conflict, no overwrite, Limited awareness (grill Q10);
  reconciliation reports exited/unverifiable.
- Binding/scenario: uniqueness under token-less resolution; provisional
  supersession; succession without contest; no external Run for owned
  processes; durable `RunStarted` only at Attested corroboration.
- Protocol: future-input for token-less envelope against old decoder
  expectations, unknown new fields ignored, absent origin fact means
  unknown; compatibility for the new RPC methods.
- Relay: token-less mode silence/exit-0/no-activation unchanged; self-pid
  fields present; budget unaffected (measured evidence, not per-run
  assertion).
- Lifecycle: sweep across daemon restart; pid-reuse guard via start time;
  succession is one transaction — A ends `SessionChanged`, B starts,
  A-end seq < B-start seq, never two concurrently open Runs on one
  process, no durable intermediate state, asserted across restart and
  crash-point injection; A projects `execution_state: "exited"` and a
  PR6-shape decode reads it unchanged.
- Display gate: a weak candidate never produces a row; the sealed
  recognizer does; identity collapses it — snapshot-covered in the TUI.
- TUI: insta snapshots for external, degraded, and provisional rows and
  integration status.
- CLI: enable/disable/status round-trip including a refused install
  naming its trigger.

## Definition of done

- Grill rounds 1–4 recorded and closed (done, 2026-09-02); spike
  reference recorded (done); ADR 0013/0014 accepted (done); plan
  unblocked before any boundary was crossed.
- Designs 2–10 implemented; `./scripts/verify` green on the final tree;
  matrix recorded with the design-9 fields; snapshot coverage present.
- **Merge gate (grill Q7′): real-world configuration-shape fixtures.**
  The merge engine's fixture corpus contains documented, sanitized
  real-world samples exercising at least: unrelated existing Claude
  hooks; multiple hook entries/events; unrelated nested settings; Codex
  TOML comments; unrelated Codex tables/keys; realistic
  whitespace/order/layout; absent Corral slot; refused ownership
  conflict. Provenance recorded per fixture; the corpus never widens
  Corral ownership — a weird file is preserved/refused honestly, never
  normalized until editable.
- Schema gate: intent-table and circuit-breaker-state diffs carry
  `DURABLE-APPROVED-BY:`.
- Human-merged (Class C). PRODUCT §8 law holds in every rendered string
  (no "binding", "assurance", "sweep", "intent" reaches a user).
- Glossary entries landed; ADR 0006 drift fixed; plan moves to `done/`.

## Follow-ups

- The full discovery coverage-audit harness (ROADMAP §9.2) beyond the
  matrix's host scenarios — release-gate machinery, own task.
- Launch-time handshake ("not managed until a hook arrived") from the PR5
  matrix's org-policy limitation — attention-phase shape, unbuilt here.
- S3 live-join census consumes PR7's external bindings; unblocked, not
  started.
- Delivery-health counters (disable rate, merge-failure rate, drift rate)
  need a dogfood-visible readout before the A-window opens (ROADMAP §6).

## Plan size justification

Over target for the PR5/PR6 reason: the two release gates ROADMAP pins to
PR7 are one coherent scope. The integration engine without token-less
ingress mutates user configuration to feed deliveries the daemon drops;
ingress without the engine never receives a delivery to judge; the sweep
without either shows rows that can never gain identity; and the A-thesis
claim this phase exists to demonstrate is only testable with all of it
present. Review seams stay separable: engine, intent store, relay/wire,
ingress/corroboration, sweep, surfacing, matrix.
