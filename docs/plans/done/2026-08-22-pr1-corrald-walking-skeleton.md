---
status: done
class: C  # contains ADR 0001 + an ARCHITECTURE §4 amendment; founder acceptance + human merge
writes:
  - adr                    # ADR 0001 (rewritten per the activation grill)
  - decision-records       # docs/decisions/2026-08-22-pr1-activation-grill.md
  - canonical-docs         # ARCHITECTURE §4 hello/absent-field amendment
  - corral-protocol
  - corral-rendezvous      # new crate
  - corrald                # new crate
  - corral-client          # new crate
  - corral-surface         # new `corral` CLI crate
  - verification-tooling   # workspace members; dependency-direction set
reads:
  - docs/decisions/2026-08-22-pr1-activation-grill.md   # the authority for every rule below
  - docs/references/architecture-benchmarks.md          # rows 4, 12
---

# PR1 — corrald walking skeleton

All activation/lifecycle/protocol semantics below are founder-ruled in the
2026-08-22 activation grill record and materialized in ADR 0001; this plan
adds no new decisions.

## Goal

Prove the client → daemon path under the accepted architecture: a corrald
that clients lazily start or attach to at the canonical per-OS-user
rendezvous, with flock singleton semantics, client-first hello
negotiation, a committed Running → ShuttingDown → Exited lifecycle, and
`corral ping` / `corral list` working end to end (ROADMAP §3 PR1).

## Non-goals

No sessions/bindings/persistence (PR2 — `session.list` returns an honest
empty list); no PTY/terminal streams (PR3); no providers/hooks/attention
(PR4+); no TUI; no listener/discovery/Remote Node Mode; no mutating RPC of
any kind; **no ghost wire surface** — no wire representation, fixture, or
discriminant for subscribe/live-event/durable-event behavior PR1 does not
serve (the roadmap's stream vocabulary stays in ARCHITECTURE prose); no
daemon instance/boot ID; no `corral-core` changes; no Windows.

## Existing owner / architecture involved

Crate layout, dependency direction, and the hello/compat posture are
accepted (ledger rows 4, 12; ARCHITECTURE §4 §10); lazy activation and
may-exit-when-idle are accepted (§7). Their mechanics are ADR 0001,
grilled and pending founder acceptance of the revised text. New crates
follow accepted layering; `corral-rendezvous` is the grill-added shared
owner of singleton identity (two independent PR1 consumers).

## Design

1. **Decision docs land first**: grill record (immutable),
   rewritten ADR 0001 (`proposed` → founder confirms → `accepted`),
   ARCHITECTURE §4 amendment (symmetric `min_compatible_peer_version`
   hello; absent-field rule split into required ⇒ MalformedHello vs
   optional ⇒ documented default). Staged as its own small PR so
   acceptance provably precedes implementation.
2. **corral-rendezvous**: canonical path derivation from the effective OS
   user's account home (never `$HOME`); lock/socket artifact rules
   (stable lock inode; confirmed-socket-only stale cleanup; fail-closed
   corruption errors); SH|NB probe and EX bounded-wait claim helpers;
   user-private directory creation; the S6d failure table's
   resolution/configuration variants. Unix mechanics stay behind explicit
   platform modules (`cfg(unix)` does not leak upward).
3. **corral-protocol**: newline-delimited JSON envelopes
   (request/response/notification), string method ids (pre-release shape;
   renumbering legal until first tagged release); ClientHello/ServerHello
   with the single symmetric compat predicate; typed request errors
   (MethodNotFound preserving request id, InvalidParams) and bootstrap
   failures (MalformedHello, ProtocolViolation, incompatible result);
   baseline methods `ping`, `session.list`; empty capability set.
   Future-input fixtures: unknown hello fields ignored; optional field
   missing ⇒ default; required version field missing ⇒ MalformedHello;
   unknown method ⇒ MethodNotFound; unknown notification ⇒ ignored;
   response frame without a matching daemon request ⇒ ProtocolViolation.
4. **corrald**: EX-lock claim → confirmed-stale cleanup → bind → listen →
   serve; pending_handshakes (pre-hello deadline, no idle influence) vs
   established_clients; hello-first enforcement (repeated hello ⇒
   ProtocolViolation); responses only — no server-initiated frames;
   `DaemonPolicy { idle_grace: 60s, pre_hello_deadline: 10s }`; idle
   commit + SIGTERM/SIGINT on one committed shutdown path; tracing to the
   canonical log, best-effort, never a correctness authority.
5. **corral-client**: the activation state machine — connect → probe →
   sibling-only spawn (internal auto-start mode; child does setsid;
   spawner nulls stdio, retains/reaps Child) → retry under one
   `ClientActivationPolicy { activation_deadline }`; handshake
   orchestration; the typed failure set (OwnerPresentButUnreachable,
   SpawnedDaemonDidNotBecomeReady, IncompatibleDaemon,
   DaemonConnectionLost, resolution/config errors). `CORRAL_ENDPOINT` =
   connection override only: unreachable ⇒ explicit error, never spawn.
6. **corral (CLI)**: `corral ping` (RTT + negotiated versions/
   capabilities), `corral list` (honest empty state; every typed failure
   rendered as facts + direction, never auto-decided).
7. **Wiring**: workspace members; `scripts/check-dependency-direction`
   CLIENT_SIDE set += corral-rendezvous; no corrald → corral-client edge.
   New dependencies (all HUMAN_REVIEW_REQUIRED; justifications per the
   grill): serde/serde_json, tokio, rustix, clap (CLI binaries only),
   tracing (+subscriber at executable init only). Test-support-only typed
   settings input for daemon timing knobs; no production backdoor.

## Interfaces or persistence changed

New wire surface (hello, ping, session.list, envelope, typed errors) —
compatibility-sensitive from birth; wire permanence has not begun. New CLI
commands (exit codes explicitly not yet a stable contract). Canonical
on-disk rendezvous and log paths per ADR 0001. No durable storage;
`STORAGE_EPOCH` stays `dev`; PR1 owns no durable semantic state.

## Failure / unknown states

The ADR's failure table is normative: layered
resolution/configuration → activation → reachability → handshake
failures; wedged rendezvous = bounded eventual recovery via idle exit;
shutdown races resolved by the atomic Running → ShuttingDown commit;
crash residue owned by the next lock winner; incompatibility terminal
with no retry/spawn/kill; in-flight requests fail honestly with no
replay.

## Tests

All mandatory in `./scripts/verify`. Integration (real processes; Linux
CI, macOS locally): env-invariance (XDG/HOME variations ⇒ same canonical
endpoint); N concurrent cold-start clients ⇒ one daemon, all served;
override purity (dead `CORRAL_ENDPOINT` + live canonical daemon ⇒ error,
no spawn); lock-held wedge ⇒ no spawn ⇒ OwnerPresentButUnreachable;
confirmed-stale cleanup vs non-socket fail-closed (file survives);
EACCES ⇒ permission error not owner-present; half-start absorbed by
retry; hello-first / repeated-hello violations; pending starvation (
repeat connect-no-hello ⇒ daemon still idle-exits); commit linearization
both orders (establish-then-commit, commit-then-establish) using
test-support timing; incompatible daemon (typed response, terminal, no
kill); malformed-frame close; SIGTERM with established clients
(DaemonConnectionLost, exit 0, lock released); idle exit end-to-end +
fresh reactivation; SIGKILL crash residue recovery; sibling-only (PATH
decoy unused; missing sibling ⇒ InstallIntegrity); unwritable log dir ⇒
daemon still serves; ping/list baseline. Unit: symmetric compat predicate
(+ both-sides-agree property); envelope/hello future-input fixtures;
rendezvous path rules. `./scripts/verify` green on the final tree.

## Definition of done

- Grill record + revised ADR 0001 merged, ADR `accepted` by founder
  confirmation **before** any implementation commit crosses the decision
  boundary (evidence: PR ordering in history).
- Design items 2–7 landed; ping/list work against a lazily started
  corrald on macOS and Linux (evidence: the integration suite above runs
  inside `verify`; verify output in the PR body).
- No ghost wire surface (review checklist: every serializable protocol
  variant maps to a served method or bootstrap frame).
- Class C path honored: HUMAN_REVIEW_REQUIRED satisfied, two independent
  fresh-context reviews, founder merge; dependency justifications in the
  PR body; staging note if the non-mechanical diff nears ~800 lines
  (expected: docs PR + implementation PR).
- Plan moves to done/ on land.
