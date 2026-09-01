---
status: accepted
read_when:
  - writing code that mutates a user's global provider configuration
  - designing or changing hook install, merge, repair, versioning, or uninstall
  - deciding what Corral may do when a merge or write cannot be proven safe
  - deciding where integration enabled/disabled intent lives
---

> Accepted 2026-09-02, after PR7 integration grill rounds 1–4
> (`docs/decisions/2026-09-01-pr7-integration-grill.md`) and the provider
> behavior spike supporting its load-bearing claims
> (`docs/references/2026-09-02-pr7-global-integration-spike.md`).
> Remaining evidence work does not alter the accepted architecture:
> public dotfiles corpus — PR7 merge gate; Homebrew provider channel —
> post-merge matrix expansion, promoted to a dogfood entry gate wherever
> that channel is used. A future measurement reopens an accepted decision
> only if it contradicts a load-bearing accepted assumption; ordinary
> matrix expansion does not.

# Global hook integration: how Corral writes itself into a user's provider configuration, and what it does when it cannot

ADR 0006 fixed the policy: integration is default-installed, disclosed, and
fail-safe, and the permanent ban is undisclosed or destructive mutation.
`ARCHITECTURE.md` §6 fixed the shape: install / version / merge / uninstall
with lock and owner identity, degrading to read-only discovery when safe
coexistence cannot be proven. This ADR fixes the mechanics under those for
PR7: who writes, what ownership is, what the merge may touch, the closed
list of fail-safe triggers, drift repair, where intent lives, and the Codex
single-value ruling ADR 0009 left open. What a token-less delivery from
these hooks may then claim is ADR 0014's, not this file's.

Structural rulings for this ADR were founder-accepted 2026-09-01
(`docs/decisions/2026-09-01-pr7-integration-grill.md`, Q1–Q3, Q6); the
post-spike rulings (Q1′–Q4′, 2026-09-02) sealed the fact-sensitive
remainder over the measured evidence, and round 4 accepted the ADR.

The S2 scope items this decision depends on — the real-world settings
corpus, the merge-ambiguity taxonomy, the fail-safe trigger set
(`docs/references/2026-08-22-s2-session-identity-verification.md`) — are
measured by the PR7 spike before acceptance; the load-bearing facts are
listed at the end.

## D1 — One mutator, and every mutation is a named operation

`corrald` is the only process that writes a user's provider configuration.
The singleton claim (ADR 0001) is the write lock: there is no second lock
file, because there is no second legal writer. Surfaces — CLI, TUI, later
Desktop — request `install`, `repair`, `uninstall`, and read `status` over
the client protocol; none of them touches a provider file.

Every mutation is one of those named operations, disclosed and recorded
with what it read, what it wrote, and the backup it took (D3). No provider
file is ever written as a side effect of another action.

Who pulls the install trigger is ruled (grill Q2): **first activation is
not the installation trigger during PR7 dogfood.** Install runs only on the
explicit user action `corral integration enable`; the packaged installer
(M1 completion) owns default installation, which is how ADR 0006's
"installed with the normal installation" is honored — product default
Enabled and PR7's explicit development trigger are different things and do
not conflict. Daemon activation may inspect integration state and detect
and report drift; it never performs first-time installation, so a logically
read-only `corral list` can never be the event that rewrites
`~/.claude/settings.json`. Whether and how an explicitly enabled integration is
repaired on drift is D5's, ruled in the same grill's round 2. There is no
file watcher and no rewrite loop.

Rejected: letting the `corral` CLI write directly when the daemon is not
running. Two writers need a real lock protocol, and the CLI starts the
daemon lazily anyway; the failure mode it would protect against — mutating
config with no daemon to receive events — is a state nobody should be in.

## D2 — Ownership is structural, and versioned

Corral's Claude entries are hook entries whose command invokes
`corral hook-relay --provider claude` with no launch token (token-less
scope is ADR 0014 D1) and a config-version discriminant flag. Corral's
Codex integration is a `notify` value that invokes the same relay. That
invocation **is** the owner identity: Corral may modify or remove exactly
the entries and values that invoke its relay, recognized structurally by
parsing the command line — never by similarity, position, comments, or
guesswork. Everything else in the file is the user's or a third party's and
is never touched.

The config-version discriminant is how the written artifact evolves — read
by the merge engine only; the relay ignores unknown flags by its PR6
tolerance contract: an
entry carrying a version this binary understands may be upgraded in place
by `repair`; an entry carrying a newer version than the binary understands
is left untouched and reported (a D4 trigger) — an older Corral never
rewrites what a newer Corral wrote.

The events installed globally are ADR 0004 D6's five — `SessionStart`,
`UserPromptSubmit`, `Stop`, `SessionEnd`, `Notification` — for the same
reasons, now at global scope.

## D3 — The merge preserves semantics universally, bytes per provider

Ruled post-spike (grill Q3′), replacing the earlier "additive structured
editing over a preserved original" — a phrase the spike showed reads as a
byte-preservation promise no measured provider honors symmetrically. The
two-level model:

> Semantic preservation is universal. Byte/format preservation is
> provider-specific and required where the measured provider
> format/workflow makes it meaningful.

Universally: Corral preserves user-owned configuration semantics outside
the exact Corral-owned integration surface — it appends or removes only
Corral-owned entries (D2) and never deletes or alters the meaning of
anything else.

The representation policy is per provider, bound to the 2026-09-02
measurements:

- **Claude `settings.json`** — strict JSON only; whole-document parse;
  structured merge; unknown keys and values preserved semantically;
  serialization is complete valid JSON in the accepted canonical
  formatting, with no attempt to preserve byte layout or key ordering
  (the provider itself reserializes the whole document on its own
  writes). **Corral MUST NOT introduce comments**: measured Claude
  treats comments/JSONC as invalid, and an invalid file silently drops
  the user's entire settings, not just hooks.
- **Codex `config.toml`** — format-preserving TOML editing; comments,
  unrelated key order, and spacing retained as far as the chosen
  format-preserving editor contract allows; only the owned `notify`
  surface is mutated (measured Codex itself patches surgically and
  preserves what it did not write — and a malformed file is fatal to the
  Codex CLI, which raises the write-safety bar).

Neither provider's mechanics leak to the other: Claude does not imitate
Codex's editing, Codex does not pay for Claude's whole-file rewrite.

**Pre-replacement validation gate** (grill Q2′, mandatory): after
constructing the complete candidate content, the engine re-parses it in
full with **Corral's provider-specific strict validation parser** — a
Corral-owned parser whose accepted grammar is tied to measured provider
behavior, never a claim that the provider's own parser ran. Only a
successful full re-parse is eligible for atomic replacement; a failed
re-parse is a D4 refusal that leaves the original bytes untouched. The
gate is necessary but not sufficient to prove provider acceptance; the
supported-version matrix remains the empirical authority.

Writes are atomic same-directory tempfile plus rename with mode
preservation (law, ADR 0006; restated here only because this is the
module that implements it).

Backfill-before-overwrite: before every mutation, the current file content
is copied into Corral's own state directory, timestamped, with bounded
retention. The backups are disclosed recovery artifacts, not a
byte-for-byte restore promise — ADR 0006 already refused that promise and
this ADR does not quietly reinstate it.

Between read and rename the file may change under us — providers rewrite
their own configuration. The write re-verifies the file identity it read
(mtime/size/inode class evidence) immediately before rename; a mismatch
aborts the write as a D4 trigger rather than clobbering a concurrent
writer. Corral loses that race on purpose.

## D4 — Fail-safe triggers are a closed, version-sealed list

Merge ambiguity fails safe (ADR 0006). "Ambiguity" is not a mood; it is
this list, sealed by the post-spike ruling (grill Q2′) for the measured
versions — Claude Code 2.1.252 and codex-cli 0.152.0, per
`docs/references/2026-09-02-pr7-global-integration-spike.md` — re-sealed
per supported provider version by the matrix, and a condition outside the
list fails closed to the same behavior:

```text
common to both providers
  the provider config cannot be parsed
  a path the merge must traverse has an incompatible structural type
  a Corral-owned entry claims a representation/version newer than this
    Corral understands
  the file or its directory is not safely writable
  the source changed between the read basis and the replacement attempt (D3)
  the pre-replacement re-parse of the candidate content fails (D3)

Claude-specific
  effective `disableAllHooks: true` at any measured effective layer —
    user, project, project-local, or enterprise managed settings —
    including layers Corral never mutates but must inspect (measured:
    any one of the four silences every hook, silently); integration
    cannot claim delivery

Codex-specific
  the `notify` value is occupied by something not Corral's (D7)
  the `notify` value has an incompatible type (measured: fatal to the
    Codex CLI, not merely ignored)
```

On any trigger: **no write**, the provider enters Limited awareness
(`PRODUCT.md` §6), the cause is surfaced once with a resolution ask, and
the operation is recorded as refused. Never overwrite, never retry-loop;
the trigger is re-evaluated on the next named operation or daemon start,
not by watching the file.

And one rule that outranks convenience: at global scope Corral never writes
a key that changes the meaning of the user's other configuration.
Scenario 13's `"disableAllHooks": false` is legal only inside Corral's own
injected launch file, where it scopes a document Corral wrote entirely; in
the user's global settings the same key would be Corral overriding the
user's stated intent, which is the mutation ADR 0006 permanently bans. A
user who globally disabled hooks gets honest Limited awareness and a
disclosed cause, not a silently re-enabled hooks system.

## D5 — Drift is detected at boundaries and repaired as an operation

Providers rewrite their own files, and whether Corral-owned entries survive
that is an open evidence question (`ROADMAP.md` §9.6). So Corral verifies
rather than assumes: on daemon start, and on any `status`/`install`/`repair`
request, the effective file is re-read and compared against what intent
(D6) says should be installed.

Enabled intent authorizes maintenance, and the scope of that authority is
ruled (grill Q10): automatic repair touches only what Corral can **prove**
it owns, and "missing" and "modified" do not share a policy:

```text
Corral-owned entry missing                    → may auto-repair
Corral-owned entry holding an old Corral
  path/representation ownership rules prove
  is ours                                     → may auto-repair
unrelated user-owned configuration changed    → never touch
Corral's expected slot holding non-Corral /
  user-authored content                       → conflict: never overwrite;
                                                Limited awareness +
                                                explicit resolution
```

Enabled is not permission to continuously normalize the user's provider
configuration. Repair runs only at daemon startup and inside explicit
named operations — never a periodic polling writer, a mid-run rewrite, a
per-hook rewrite, or a background normalization loop — through the same
engine, triggers, and backup, recorded observably and counted as
delivery-health evidence (`ROADMAP.md` §6). A repair that would trip D4
degrades instead. And the structural principle is frozen:

> Repeated evidence that another authority keeps undoing Corral's
> integration must eventually stop automatic repair rather than create a
> configuration tug-of-war.

Its parameters are now ruled (grill Q4′), informed by the measured fact
that a provider's own whole-file rewrite can silently drop Corral's entry
in an ordinary race — so a repaired "missing" is the expected path, not
evidence of a competing authority, and the budget is sized for the
authority case:

- **Fingerprint**: `(provider, config target, drift class)`. Initial
  drift classes: Corral-owned entry missing; Corral-owned entry present
  in an older Corral-owned representation; ownership conflict — but
  ownership conflict never consumes the repair budget, because it is
  already non-auto-repairable and goes directly to Limited awareness and
  explicit resolution.
- **Breaker**: 3 automatic repairs within a rolling 24-hour window per
  fingerprint; the next matching auto-repair opportunity does not
  rewrite — it opens the circuit breaker for that fingerprint, enters
  Limited awareness, and surfaces the explicit resolution path.
- **The breaker does not close by itself.** The rolling window decides
  only when it opens. Once open it stays open — a window that later
  holds fewer than three historical repairs does not re-arm automatic
  repair, or a dotfiles authority gets a repair-a-day loop forever. Only
  an explicit user-controlled reconciliation action
  (`corral integration repair`, or an equivalent explicit
  enable/re-enable flow) that re-checks current ownership and succeeds
  clears the breaker and the repair history for that fingerprint. Daemon
  restart alone never clears it.
- **The breaker and its history are Corral-owned durable operational
  state.** If the registry schema cannot yet represent the fingerprint,
  the repair timestamps/count window, and the breaker-open state, that is
  a durable-state expansion under the storage law with its own
  schema/migration review — the grill ruling covers the policy, not the
  schema; the count may not be quietly made process-local to keep PR7's
  durable diff small.
- 3 / rolling 24h is a **dogfood-tunable policy default, never a wire
  constant**; tuning may go stricter or looser on dogfood evidence, but
  no implementation may silently exceed the currently accepted repair
  authority.

> Enabled authorizes bounded self-repair, not an endless configuration
> tug-of-war.

The prior form of the ruled invariant still holds beneath it:

> Enabled authorizes Corral to maintain what Corral owns. It does not
> authorize Corral to overwrite configuration whose ownership has become
> ambiguous.

Drift in the other direction — the user deleting Corral's entries by hand —
is indistinguishable in the file from a provider rewrite, which is exactly
why intent does not live in the file (D6). The file is evidence of what is
installed; it is never the record of what the user chose.

## D6 — Integration intent is a Corral-owned durable fact

Per provider, node-scoped: `Enabled` (the ADR 0006 default) or
`Disabled` (the user chose). It is the first node-scoped Corral-owned
durable fact, stored in the registry store beside the session log — not in
the session-event log, whose streams are per-session (ADR 0002), and not in
the provider's file, which D5 shows cannot carry it. As a Corral-owned fact
it is subject to the durable-state law from the dogfood epoch onward; today
the epoch is `dev`.

`Disable Integration` = the `uninstall` operation (remove Corral-owned
entries per D2) plus intent set Disabled plus ADR 0014's ingress gate: a
disabled provider's token-less deliveries are dropped with diagnostics, so
stale copies of Corral's entry in files Corral does not manage cannot keep
feeding evidence the user switched off. Enable is the reverse. Neither
touches managed sessions: launch-scoped injection (ADR 0004 D6) needs no
global entry and asks no permission of this intent.

## D7 — Codex: set only what is absent, preserve what is not

Codex has one `notify` value where Claude has a merging hooks layer
(ADR 0009). The ruling:

> Corral sets the global Codex `notify` only when it is absent or already
> Corral's own. An occupied `notify` is the user's: preserved verbatim,
> never chained, never overwritten.

The resolution path is human, and it is exactly this (grill Q3): detect
that `notify` is owned by another configuration; preserve it; report
Limited awareness; explain exactly what blocks Corral integration; tell
the user how to remove or change the conflicting value; and accept a fresh
`integration enable` after they have. Nothing vaguer is promised, because
no Corral-owned resolution operation exists in this phase: no `--force`,
no take-over, no backup-and-replace, no chained notifier. Explicit
takeover UX is deferred, not rejected forever — it arrives only if real
dogfood conflict data shows users with a custom `notify` are an actual M1
problem, and it is not a small flag: it would owe a durable backup
location, conflict semantics when the user edits after backup,
compare-and-swap restore conditions, stale/corrupt backup handling, and
cross-version behavior. The ruled invariant:

> Corral never overwrites a non-Corral Codex notifier merely to obtain
> awareness.

Setting an absent value is a merge with no ambiguity: nothing of the user's
is displaced, ownership is structural (D2), and uninstall clears exactly
that value. A user who later edits a Corral-set `notify` has taken the
value back — it no longer invokes Corral's relay, so it is no longer
Corral's (D2), and D5's repair refuses it as occupied rather than
reclaiming it.

Chaining the user's program stays rejected on ADR 0009 D4's grounds, which
bind harder at global scope: executing user-configured code from Corral's
relay, for every session on the machine, forever, to avoid an honest
degradation message. Managed Codex launches are untouched by this ruling —
the `-c notify` override (ADR 0009 D1) neither reads nor needs the global
value.

## D8 — Absence must fail open: stale integration may not disrupt

Ruled (grill Q6), in two layers. First, the packaging obligation: the
normal installer/uninstaller runs the `integration uninstall` operation
before removing Corral-owned executables or artifacts. Second — because
manual binary deletion, broken package removal, downgrades, and partial
installs will all happen — the obligation the packaging cannot carry:

> A default-installed Corral integration must fail open when Corral is
> absent or unavailable; stale integration must not repeatedly disrupt
> the user's provider sessions.

A default-installed integration may not make "the Corral binary happens to
exist forever" a precondition of provider usability. The missing-command
fact is now measured
(`docs/references/2026-09-02-pr7-global-integration-spike.md`, scenario
11): a naked Claude hook command whose path does not exist produces a
two-line visible error on session start, on **every** prompt, and at
**every** turn end — the stop condition fired — while a missing Codex
`notify` program is completely silent. The residual-failure shape is
therefore ruled per provider (grill Q1′):

- **Claude entries MUST use a fail-open guarded invocation**: execute the
  Corral relay, and regardless of relay exit or failure return provider
  success at the hook boundary — conceptually
  `<corral relay invocation> || true`. What D8 freezes is the semantic
  shape — *the provider-visible hook result is fail-open* — not a
  particular quoting string; the measured fact that Claude judges this
  boundary by the guarded command's resulting exit status alone is
  load-bearing and is retested by the matrix on every supported-version
  change.
- **The guard is not a new provider-data parser.** The guarded shell
  command may contain only Corral-owned static invocation structure plus
  safely represented Corral-owned path/arguments. It MUST NOT interpolate
  hook payload, prompt text, provider event content, session identifiers
  originating as arbitrary text, or user shell fragments into shell
  syntax; provider event data continues through the already-defined data
  channel (stdin / fixed argv contract / other measured mechanism), never
  by string concatenation into `sh -c`.
- **Ownership recognition stays exact**: the checked-in Claude
  integration grammar recognizes exactly the Corral-owned guarded form it
  writes; an arbitrary `… || true` in a hook entry is never proof of
  Corral ownership.
- **Codex keeps its native argv invocation** — measured missing-command
  behavior is already silent and fail-open, and no shell exists there to
  host a guard.

The accepted trade-off is explicit: a relay crash may be invisible in
Claude's own UI. That is intentional — integration delivery failure
belongs to Corral's delivery-health and Limited-awareness reporting, not
to repeated interference in the user's agent session. The ruled invariant:

> Removing or losing Corral must not turn a previously installed
> integration into persistent interference with the user's agent.

## Load-bearing facts to measure before acceptance

First-party, dated reference, PR7 spike — this ADR is accepted only over
these measurements. Measured 2026-09-02
(`docs/references/2026-09-02-pr7-global-integration-spike.md`); the grill
(rounds 1–4 record) ruled on the results and placed the open items'
gates: public-dotfiles corpus — PR7 merge gate; Homebrew channel —
post-merge matrix expansion (dogfood entry gate where used); macOS
upper ancestry — an ADR 0014 fact, post-merge matrix expansion.

- The settings corpus: real-world `~/.claude/settings.json` and Codex
  `config.toml` shapes — strict JSON or JSONC in practice, comments in the
  wild, third-party hook layouts — sealing D3's parser choice and D4's
  trigger list.
- Provider rewrite behavior: whether Claude/Codex rewriting their own
  files preserves foreign entries, formatting, and comments (D5's premise,
  `ROADMAP.md` §9.6).
- Whether a global Corral entry and a managed launch's injected entry both
  fire for one provider event (feeds ADR 0014 D4).
- Whether a config-layer `notify` fires for interactive Codex TUI sessions
  — the spike measured only the `-c` override path (ADR 0009's spike,
  scenario coverage).
- Claude's documented settings precedence for hooks across user / project /
  local layers, insofar as it decides where the global entry must be
  written to observe sessions in any project.
- **Missing-command behavior (D8's stop check):** for each provider, a
  configured hook/notify command whose path does not exist — user-visible
  warning or error, per-event/per-turn repetition, whether the agent
  continues, latency or blocking, exit-status interpretation,
  stdout/stderr behavior, and whether the provider disables or retries the
  integration.

## What this does not decide

What a token-less delivery may claim, corroboration, external Runs, and
the process sweep (ADR 0014). Attention semantics and the five-state model
(PR8). The packaged first-run install and disclosure surface (M1 completion
work runs these same operations; nothing here changes). Detection of
organization-managed policy that silences hooks outside the file Corral can
read — recorded as a known limitation, not solved here.
