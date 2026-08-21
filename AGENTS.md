# AGENTS.md

Repository-wide hard rules for coding agents working on Corral. This file is canonical engineering law; `docs/GOVERNANCE.md` explains the document hierarchy.

Keep this file concise and bounded (~260 lines). Product detail belongs in `PRODUCT.md`, architecture detail in `ARCHITECTURE.md`, roadmap scope in `ROADMAP.md`, process mechanics in `docs/ENGINEERING_WORKFLOW.md`, implementation plans in `docs/plans/`, irreversible decisions in `docs/adr/`, and reference evidence in `docs/references/`.

Before non-trivial work, read `docs/ENGINEERING_WORKFLOW.md` and the nearest scoped `AGENTS.md` for the touched subtree, if one exists.

## Product invariant

Corral is a session-first control center for coding agents.

The core loop is:

    See every session.
    Know what needs you.
    Take control.

Do not add adjacent product features merely because the architecture can support them. The current `ROADMAP.md` phase is a scope boundary.

## Core model

`Session` is the primary unit of AI work.

A Corral Session is not a terminal, pane, process, transcript, provider session, workspace, or machine. Those are facets or bindings.

Provider-native identity and execution location are bindings, not Corral Session identity.

Every logical Session has a Corral-owned globally unique identity.

Never use `(node, provider_session_id)`, pane id, terminal id, cwd, path, or provider id as the Corral primary key.

Heuristic bindings are never authoritative and must never enable control operations. Heuristic-assurance evidence may render, marked unverified, but never fires a notification.

## Runtime truth

The execution node/runtime is authoritative for execution state.

Never infer:

    disconnected == exited
    unreachable == stopped
    unknown == false

Unknown / unverifiable is a first-class state.

Do not silently substitute local execution when requested remote execution is unavailable.

Do not duplicate agent-status detection when an authoritative runtime source already provides it.

Agent status is evidence with source and freshness. Source authority applies only to evidence still fresh enough to support its claim: provider-native signals outrank in-band/screen heuristics, but a stale high-authority signal is invalidated by fresher contradicting evidence and the state is recomputed — possibly to Unknown. Fresher low-authority evidence does not thereby inherit the right to assert a state its source may not assert.

Attention/status derivation runs only in `corrald`. Clients render the daemon's attention state; no client derives its own.

After control-plane loss, formerly-live sessions must be re-verified and reported exited or unverifiable — never silently dropped or shown stale-running.

Corral integration must never degrade the user's agent: hook shims fail open within milliseconds when `corrald` is unavailable, never start `corrald`, and must never indefinitely or accidentally block agent progress. For an interaction that is already blocked awaiting user input, Corral may hold a bounded first-response lease of at most 15 seconds when necessary to provide centralized interaction; timeout, daemon loss, or delivery failure must immediately fail open to the provider's native surface.

## Client / daemon boundary

`corrald` owns shared session/runtime truth. Desktop, TUI, Tray, CLI, and future Mobile/Web are clients of the same semantic model.

Do not create surface-specific Session identities.

Shared runtime/session facts belong behind the daemon/protocol boundary. Presentation-only state belongs in the client surface.

Do not make semantic RPC depend on Desktop-specific concepts such as rows, cards, sidebars, widgets, or windows.

PTYs are owned by `corrald` alone. No surface — Desktop, TUI, Tray, CLI — ever hosts a session PTY; surfaces render streams they do not own.

Closing a UI surface must not unnecessarily terminate managed work.

Restarting/upgrading the control plane must not unnecessarily terminate managed sessions.

## Local-first lifecycle

Corral is zero-background-by-default after installation.

Installing `corrald` does not imply enabling a login service, network listener, discovery broadcast, or Remote Node Mode.

Remote availability is explicit opt-in.

## Protocol

Mixed client/daemon versions are normal.

Wire changes must preserve compatibility or use explicit version/capability negotiation.

Never reinterpret an absent field as a known negative value when absence may mean unknown.

Wire protocols must tolerate additive evolution: unknown methods, notifications, fields, and discriminants each have an explicit compatibility behavior.

Wire permanence begins at the first external tagged release exposing the contract: from then on a shipped discriminant/opcode number is permanent and is never reused. Before that release, renumbering is legal when tests and fixtures move with it. Persisted event semantics harden earlier — at the first write into non-rebuildable storage after the dogfood epoch.

Changing semantic content can break an old client even when the schema does not change; review both syntax and meaning.

## Durable state

Every durable schema or event diff requires human merge; agents never merge one autonomously. Additive change that implements accepted design — no altered meaning for existing fields or events, no changed identity/key semantics, no changed migration guarantee, no changed ownership, no reinterpretation of recorded facts — needs no new decision ceremony. Anything else is an architectural decision requiring explicit human acceptance. Uncertainty resolves to the stricter path.

Durable state has two kinds with opposite guarantees: derived state rebuildable from an authoritative source, and Corral-owned facts with no external source of truth (acknowledgements, the durable semantic event log, manual corrections).

`STORAGE_EPOCH` at the repository root records `dev`, `dogfood`, or `released`; only a human maintainer advances it. Before `dogfood`, development databases are disposable. From `dogfood` onward, Corral-owned facts may not be silently reinterpreted or discarded: migrate them, introduce a new representation, or obtain explicit approval for a destructive reset — which restarts any evidence window that depended on the discarded data.

The durable semantic event log records Corral-owned facts only. Provider history stays provider-owned; live runtime state stays runtime-owned and is never persisted as fact.

## Security

Transport identity, application identity, and authorization are separate concerns.

Being the same OS user is not sufficient proof that a process is an authorized Corral actor.

Possession of a local RPC endpoint is not sufficient authority for privileged control.

Plugin security architecture is intentionally undecided. During M0/M1:

- do not implement a public plugin runtime or marketplace;
- do not commit to a stable plugin ABI/API;
- do not prematurely implement a capability broker, WASM sandbox, permission UI, or signing system;
- preserve clean semantic extension seams without pretending they are a security boundary.

Native plugins, if later allowed without sandboxing, must be treated as trusted user code.

## Scope discipline

One task has one explicit goal and one coherent semantic scope.

A change may cross files/modules when required to repair the same invariant or owner boundary. Do not absorb unrelated cleanup or adjacent product work.

Repair the owner, not the symptom. Do not add speculative retries, wider timeouts, weakened assertions, test-only behavior, duplicate fallbacks, or consumer-side normalization when the producer owns the invalid state. A consumer may fail closed temporarily when invalid upstream state would otherwise cause unsafe behavior — refusing control, degrading to Unknown — but it must never silently repair or reinterpret that state, and the containment names the root-cause follow-up.

Record unrelated problems as follow-ups.

Implementation must be preceded by a written plan for Class B/C work. A new or changed architectural decision requires explicit human acceptance before implementation crosses that decision boundary. Work that implements already-accepted architecture may proceed from an unblocked implementation plan without repeated founder approval.

Before proposing an architecture that differs from settled decisions, read the matching row in `docs/references/architecture-benchmarks.md` and bring new evidence.

If implementation shows that the approved architecture is wrong, surface the conflict and update the design/ADR before silently redesigning it in code.

Do not solve future roadmap phases inside the current task.

## Existing concepts before new concepts

Before creating a new module, trait, state enum, protocol message, persistent field, helper, or abstraction, search for the existing owner/concept first.

New domain nouns must match the `ARCHITECTURE.md` glossary or add to it in the same change.

Prefer extending a coherent existing abstraction over creating a parallel one.

Do not extract a single-use helper merely to make generated code look structured. Extract when it names a real concept, isolates a meaningful boundary, is reused, or materially improves the caller.

Never create vague dumping-ground modules named `utils`, `helpers`, `common`, `misc`, or similar. Name the concrete domain responsibility.

Avoid god modules. Prefer focused modules; do not keep adding substantial behavior to an already very large central orchestration file when a coherent module boundary exists.

## Comments

Comments explain non-obvious WHY: invariants, ownership, lifecycle constraints, protocol/platform behavior, or surprising trade-offs.

Do not:

- narrate obvious assignments or control flow;
- restate names/types;
- describe what changed in the current commit;
- add decorative section comments;
- add comments merely because code is new.

Keep comments brief; one line when one line is enough. Prefer clearer code over explanatory prose.

## Rust

Prefer typed domain models over loosely structured strings/maps for core semantics.

Avoid ambiguous boolean/`Option` positional APIs when a named method, enum, or newtype makes the call site self-documenting.

Prefer exhaustive matches for domain state when practical. Do not hide new states behind wildcard arms.

`#![forbid(unsafe_code)]` in every crate by default. Unsafe is permitted only in named platform/runtime boundary crates; every unsafe block carries a `// SAFETY:` comment stating the invariant, enforced by lint.

Do not add dependencies before checking whether existing dependencies or the standard library cover the requirement. A new dependency requires a one-line justification in the PR naming the alternatives considered.

Platform-specific OS behavior stays behind explicit platform modules/boundaries rather than leaking `cfg` branches through core domain code.

## Tests

Tests prove observable contracts and invariants, not private implementation shape.

Prefer real implementations over mocks. A mock only proves behavior behind that mock.

Provider/history parsers use real-format fixtures/contract tests.

Session identity/binding changes require invariant/scenario tests.

Session lifecycle, provider integration, runtime behavior, and attention/status behavior changes MUST add or extend an integration test.

Protocol changes require compatibility coverage. Wire types require future-input coverage: decoding fixtures with unknown fields and unknown discriminants asserts the defined behavior.

Runtime changes test relevant lifecycle failures: detach, disconnect, restart, crash, handoff, and unverifiable state.

A flaky test is a P1 bug. Quarantine is a human-approved, time-bounded lease with a tracked owner — never a silent `#[ignore]`; quarantined tests keep running and reporting, and a quarantine over release-critical coverage blocks release. Never retry-loop CI to green. Widening a test timeout requires measured evidence that the test is correct and its budget unrealistic; a timeout is never widened to conceal uncertainty about whether the system makes progress. Mechanics: `docs/ENGINEERING_WORKFLOW.md`.

A regression test should fail on the pre-fix behavior for the intended reason whenever practical.

Do not duplicate production logic inside tests.

## Verification

Use repository verification entry points rather than inventing a new definition of done:

    ./scripts/verify-fast      iteration feedback; never merge evidence
    ./scripts/verify           the one definition of merge-ready
    ./scripts/verify-release   release gate; a strict superset of verify

There is exactly one definition of merge-ready verification: `./scripts/verify`. Repository scripts own verification semantics; CI calls them and adds only the PR-metadata checks declared in the Engineering Workflow's verification map. CI configuration never re-implements test selection, quarantine, compatibility, or release logic.

Scheduled jobs may amplify evidence — stress, fuzz, soak, repeated flake probes, breadth too expensive per PR. No merge-critical invariant may be covered only there.

A failed canonical verification stays failed until the cause is repaired or formally quarantined. Diagnostic reruns are permitted as recorded experiments; retrying until green is not.

Run focused tests during iteration. Run the final relevant verification on the final tree before claiming completion.

Passing tests are necessary, not sufficient: inspect the final diff against the stated goal and scope.

## Git / worktree safety

Read-only investigation may use a shared checkout. Use an isolated task worktree for substantive work when concurrent edits or unrelated local changes make isolation valuable.

Stage and commit only files you changed in this task. A conflict in a file you did not modify means stop and ask.

Never stage unrelated files.

Never use `git add .`, `git add -A`, or `git add --all` in a shared/multi-agent checkout.

Never use `git reset --hard`, `git clean -fd`, or blanket `git stash` to discard or hide work you do not own.

Never use `--no-verify`. Never force-push a shared branch.

Nobody pushes directly to `main` — humans included. Every change lands through a pull request under the merge-authority rules in `docs/ENGINEERING_WORKFLOW.md`.

Before commit/PR, inspect `git status` and the complete diff.

## Change size

Large non-mechanical diffs are a review risk; staging thresholds live in `docs/ENGINEERING_WORKFLOW.md`. Generated files, lockfiles, snapshots, vendored content, and pure mechanical moves are evaluated separately.

Do not split a coherent invariant into unsafe partial behavior merely to satisfy a number.

## Review

Reviews are findings-first and concise.

Review the stated goal, owner boundary, architecture invariants, failure states, compatibility, tests, and whether the fix is the best coherent fix rather than merely a plausible patch.

Do not praise the implementation, restate the task, narrate correct code, or invent nits to appear thorough.

Each material finding includes severity, concrete location, failure mode/violated invariant, and the smallest useful explanation.

`No material findings.` is a valid complete review.

Put missing test/verification evidence under `Verification gaps`; do not turn missing evidence into speculative code findings.

Do not propose architectural rewrites during review unless the current change creates a concrete correctness, reliability, security, compatibility, ownership, or maintainability problem.

## Architectural changes

Before changing any of these, update/add an ADR and get explicit design alignment:

- Session identity;
- binding assurance/control rules;
- agent-status evidence authority order;
- execution-state authority;
- runtime or PTY ownership;
- `corrald` / client boundary;
- protocol compatibility semantics;
- mutation of the user's provider/agent configuration (hook install/merge/uninstall);
- history ownership/distribution;
- remote trust/authentication;
- plugin execution/security model;
- durable storage semantics requiring migration guarantees.

This list is canonical; other documents reference it and must not carry copies.

Do not hide an architectural decision inside an unrelated feature or bug fix.

## Rule growth

Automate first: a rule that a lint, script, or CI check can own belongs in the verification map, with at most a pointer here. Prose law is reserved for what automation cannot own.

Add a rule here only after an observed failure with durable future cost. Root `AGENTS.md` stays bounded; dangerous subsystem knowledge moves into scoped `AGENTS.md` files when the need is real.
