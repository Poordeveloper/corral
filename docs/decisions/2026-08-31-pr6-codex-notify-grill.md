# PR6 Codex delivery grill — founder rulings

> Acceptance evidence for ADR 0009 (one round, Q1–Q8), 2026-08-31. The
> founder's rulings are recorded here in substance; invariant lines are
> verbatim. Session context: ADR 0009 drafted `proposed`, PR6 plan drafted
> `blocked`; the rulings below reshape both. Acceptance of ADR 0009 was
> made conditional on the Q6 spike passing; its evidence is
> `docs/references/2026-08-31-codex-0.145.0-notify-spike.md`.

## Q1 — managed Codex displaces the user's notify: (a′)

Accepted, as a **managed-launch capability substitution** — and the
justification is corrected: it must not lean on PR8. At PR6 time the
attention surface does not exist, so "the user's notify value is replaced
by Corral's attention surface" would be backdated legitimacy. The honest
statement of the trade-off, made deliberately:

> We choose to sacrifice the managed process's custom-notifier
> compatibility in exchange for reliable Corral integration evidence.

Ruled wording:

> For a Corral-managed Codex process, Corral may temporarily substitute
> Codex's external turn-completion notifier with the Corral integration
> notifier for that process only. This is a managed-launch capability
> substitution, not a persistent configuration mutation.

Constraints: no `config.toml` / profile / project-config mutation; no
effect on non-managed launches; no residual configuration change after
exit; disclosed in managed-mode documentation; never claimed that the
user's notifier was chained or preserved.

(b) chaining rejected: it would force effective-config parsing, arbitrary
argv storage, executing user programs from Corral's shim, and inheriting
failure/timeout/env/cwd/signal semantics — a poverty relay becomes a
general hook host. (c) rejected: managed evidence capability must not
depend on the user's private configuration. (d) deferred: it requires
reliable effective-config discovery as a launch prerequisite,
disproportionate today. Dogfood must record the trade-off; real users
losing custom notify reopens D4.

Core invariant (verbatim):

> Managed launch may substitute an integration side effect for that
> process, but Corral must not mutate the user's persistent Codex
> configuration or pretend that the original notifier was preserved.

## Q2 — evidence thinness accepted; OSC is a research lead only

PR6 delivers only what is verified: if the primitive is `turn_ended`,
only `turn_ended` is produced. Nothing is fabricated to align with the
Claude surface. PR8 remains bound by the existing rule: heuristic or
unverified evidence must not produce a user-visible Needs You — early
Codex may know a turn completed without knowing input is awaited, and
that is a capability limitation, never a license to promote low-quality
evidence. The ADR may name Codex TUI terminal notifications / OSC
signaling as a **candidate evidence source** for the attention phase —
never "future mechanism", "approved evidence", or "expected solution".

## Q3 — zero-turn sessions accepted; one argument deleted

> A Corral-managed runtime may exist without ever acquiring a provider
> session identity.

Managed runtime existence ≠ provider identity known. No rollout parsing,
file watching, timestamp guessing, or manufactured binding. And the draft
argument "a zero-turn session has nothing to resume" is deleted — not an
invariant Corral gets to assert. What is known is only: Corral lacks
sufficient provider identity to resume it. Whether Codex left a resumable
artifact is not PR6's to prove.

## Q4 — enum confirmed; no `dyn Provider` trait

The supported set is closed; a new provider must face launch, evidence,
identity, continuation, capabilities, and failure semantics point by
point rather than inherit trait defaults. Permitted refactor: launch
construction yields provider-specific argv plus an optional
provider-owned injection artifact. No plugin abstraction for a
hypothetical third provider.

> Exhaustive-match friction is intentional integration review pressure.

## Q5 — argv exposure accepted; token wording tightened

No tempfile/secret-handoff machinery. Security wording made strict: the
launch correlation token is NOT a cross-user authentication boundary and
MUST NOT be documented as a secret credential. The boundary is
user-private local IPC and filesystem permissions; the token's job is to
correlate one launch with one arriving provider event, not to protect
against same-user processes. High entropy and single-launch scope stay;
`ps` visibility is not a boundary violation; no authorization claim rests
on token secrecy. Payload-in-argv is Codex's notifier invocation
contract; Corral adds no file lifecycle to reduce same-user observability.

## Q6 — exact-version spike is an ADR acceptance prerequisite

Run on this machine against codex-cli 0.145.0, before acceptance, not at
matrix time: (1) interactive TUI completed turn actually invokes
top-level `notify`; (2) the payload is the final argv item, exact JSON
shape captured; (3) with a user/profile notify configured, runtime
`-c notify=…` wins; (4) free alongside: `config.toml` bytes unchanged
after the managed process exits — turning Q1's "process-local
substitution" into evidence. Failure handling: (1) false → STOP, D1
primitive invalid, reopen the Codex live-evidence design; (2) false →
adjust the relay contract before acceptance; (3) false → STOP, D4
override strategy invalid. Public source (`codex-rs` config layering,
notify argv append) is supporting evidence only — it does not substitute
for the exact-version runtime spike, and known public `-c` precedence
bugs are exactly why.

> Do not accept an ADR whose load-bearing facts have not been measured.

## Q7 — `codex exec` out of M1 managed scope

PR6 managed Codex means the interactive Codex TUI under a Corral-owned
PTY. Not supported: `codex exec`, headless batch, app-server
orchestration, CI-job semantics — not banned forever, but headless has
different lifecycle/interaction/attention/approval/output semantics and
gets its own phase. `KnownProvider::Codex` must not read as "all Codex
surfaces supported"; the support matrix distinguishes: interactive TUI =
supported managed surface; exec = unsupported in M1; app-server = not a
PR6 managed surface.

## Q8 — runtime-managed and identity-known are independent facts

Fed back to the open PR5 handshake question, half a step firmer than a
note: a future handshake/readiness model must not define "Corral
owns/manages this runtime" and "provider session identity established"
as one condition. Codex is the counterexample timeline (launch → usable →
identity unknown → first trustworthy identity evidence → binding), and
exit-before-identity is a legal path. At least two independent facts —
managed runtime established; provider binding established — whether
layered per provider or unified in a facet model. "No provider identity
yet" must never imply "was never managed".

> Runtime management readiness and provider identity readiness are
> independent facts.
