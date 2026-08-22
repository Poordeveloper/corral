# PR1 Founder Decision Record — Late plan-resolution decisions (D-EX1, D-EX2)

> Status: founder-accepted, 2026-08-22.

## What these are, and why they are not D1–D7

The activation grill closed its decision frontier on the questions it could
see (S1–S6 → ADR 0001 D1–D7). Two more surfaced afterwards, while resolving
the accepted plan into an implementation. Neither is an ordinary coding
detail the grill's closing boundary left to the implementer:

- **D-EX1** adds a third-party dependency, which is a standing human gate.
- **D-EX2** widens the test-injection surface the grill had already
  approved. Test-only, but still a supplement to a frozen decision.

So they are **late plan-resolution decisions required before ADR
acceptance**: recorded as accepted first, implemented after. Implementation
may never be the first place a decision boundary is crossed, and a PR may
never explain after the fact why it was allowed to do what it already did.

Companion record: `2026-08-22-pr1-activation-grill.md`, whose S1–S6 rulings
stand unchanged. Materialized in `docs/adr/0001-corrald-activation.md` (D1
dependency note, "Test injection").

> Provenance note: the acceptance of ADR 0001 and this record were meant to
> merge together. A merge race split them, so ADR 0001 reached `accepted`
> one change earlier than the evidence for these two rulings. Recorded here
> because a later reader would otherwise wonder why the evidence is dated
> after the acceptance it belongs to.

## D-EX1 — `uzers` approved

Approved as the narrow safe wrapper for resolving the effective OS user's
account home without consulting `$HOME`. A third-party dependency, so the
human gate applied and is satisfied here.

Frozen usage:

    effective UID
    → uzers::get_user_by_uid(...)
    → UserExt::home_dir()
    → canonical Corral user root

The **effective** uid, matching the filesystem and process authority Corral
actually acts with.

Rejected alternatives:

- **direct `libc::getpwuid_r`** — would expand Corral's unsafe boundary; the
  grill already rejected direct libc (S6f).
- **`home` / `dirs`-style crates** — they prefer `$HOME` on Unix, which is
  exactly the defect S1 and S6d closed: environment-local `$HOME` cannot
  define a user-wide daemon rendezvous.
- **`nix`** — valid, but would stand up a second Unix abstraction beside
  `rustix`.

## D-EX2 — `CORRAL_TEST_ROOT` approved, and reclassified

Approved, but **not** as a third knob beside the S6c daemon timing settings.
It is a **test-only rendezvous namespace seam**, not a daemon runtime-policy
override, and the two must not be described as the same thing.

The distinction earns its keep: process-level tests must exercise the real
path — real `corral` binary → canonical resolution → lock → socket →
auto-spawn → sibling `corrald` — and if the canonical root can only ever be
the real account home, the only place those tests can run is the developer's
own `~/.corral`.

Frozen shape — the variable names the Corral root itself, not a home
directory under which one is derived:

    production:    corral_root = <OS account home>/.corral
    test-support:  corral_root = CORRAL_TEST_ROOT

so a test namespace contains `run/`, `log/` directly.

Hard limits:

- only compiled and recognized with `test-support`;
- `test-support` is not a default feature;
- normal production binaries do not recognize the variable at all — only
  explicit test-support builds do, so the boundary between a test artifact
  and a shipped binary is mechanically checkable;
- must be an absolute path;
- the same lock, socket, path-length and filesystem validation still runs;
- client and daemon resolve through the same `corral-rendezvous` function;
- never a normal auto-activation configuration surface;
- no fallback from the production canonical root to `CORRAL_TEST_ROOT`.

This does not weaken S1, because the variable has no meaning in production;
in a test build it creates a virtual OS-user Corral root so a multi-process
integration test can run inside a temporary directory. An environment
variable is the right carrier here — unlike the rejected
`--internal-config` backdoor — because an auto-spawned child `corrald`
inherits the same test namespace naturally, without adding a parameter to
the production activation protocol.

## Wording

Do not describe the gate as "compiled out of release builds": that blurs
which artifacts are affected. The accurate statement, and the one that can
be checked by machine, is *normal production binaries do not recognize the
variable; only explicit test-support builds do.*
