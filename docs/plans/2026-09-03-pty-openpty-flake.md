---
status: active
class:  B
writes:
  - third_party/portable-pty (vendored PTY backend)
  - crates/corrald/src/runtime (PTY spawn)
reads:
  - crates/corral/tests/support (harness terminal)
---

## Goal

Allocating a PTY concurrently inside one process must not fail.

`./scripts/verify` went red twice on the same defect, in two different
places, neither of them the test's own fault:

```text
runtime::session::tests::the_last_output_instant_is_published_once_the_child_draws
  the session starts: Spawn(Pty(Custom { kind: Other,
  error: "failed to openpty: Os { code: 6, kind: Uncategorized,
  message: \"Device not configured\" }" }))

corral_new_records_the_run_it_started
  corral: the daemon refused the request: invalid_params:
  could not allocate a pty: failed to openpty:
  Os { code: -6, kind: Uncategorized, message: "Unknown error: -6" }
```

The second is the product failure this is really about: a person running
`corral new` is told the daemon could not allocate a pty, for no reason
they can act on, at a rate that grows with how many sessions they start at
once. The flaky test is the same defect seen from the suite.

## Non-goals

- Making the tests that hit it serial, retry, or tolerate the failure. The
  producer owns the invalid state (AGENTS.md §Scope discipline).
- Changing PTY ownership, the runtime boundary, or what `spawn` returns.
- Cross-process contention. Measured below and not present.

## Existing owner / architecture involved

`corrald` alone owns session PTYs (AGENTS.md §Client / daemon boundary).
The one allocation call is `crates/corrald/src/runtime/spawn.rs`,
`native_pty_system().openpty(...)`, which reaches the vendored
`third_party/portable-pty` and there calls `libc::openpty` directly.
`crates/corral/tests/support/pty.rs` calls the same vendored entry point
from the test harness's own process, for the terminal that stands in for
the user's, not for a session.

The vendored crate already carries one Corral patch under a documented
mechanism (`third_party/portable-pty/CORRAL_PATCHES.md`, PATCH 1), whose
regression is `crates/corrald/src/runtime/spawn_tests.rs`.

## Design

Measured first, on macOS 25.5.0, with a standalone probe that performs
`openpty`'s steps separately and records which one fails
(`posix_openpt` → `grantpt` → `unlockpt` → `ptsname` → `open` the slave):

| shape                                        | calls | failures      |
| -------------------------------------------- | ----- | ------------- |
| 16 threads, one process                       | 6400  | 6, all `posix_openpt` |
| 16 threads, one process, one at a time         | 6400  | 0             |
| 16 single-threaded processes, concurrent       | 6400  | 0             |

So: the failing step is `posix_openpt` — opening `/dev/ptmx` — the race is
between threads of one process, and serializing within the process removes
it. It is not exhaustion: the peak was 42 allocated ptys against a
`kern.tty.ptmx_max` of 511. `ptsname`'s static buffer, the obvious
suspect, never failed and is not involved. Both observed errnos come from
that one step: `ENXIO`, and `-6`, which is not an errno at all — the value
reaching userspace is not a reliable description of what went wrong, so
nothing may branch on it.

The repair is to allocate one PTY at a time per process, at the vendored
backend's `libc::openpty` call — PATCH 2. That is the single place every
Corral process reaches, so the daemon's `spawn`, the harness terminal, and
any later caller are covered without call-site discipline. The critical
section is one syscall triple that does not block; PTY allocation is a
per-session event, not a hot path.

Unconditional, not `cfg(target_os = "macos")`: the guarantee wanted is
"Corral allocates one PTY at a time", which is a property of the
allocation boundary rather than of a platform, and a platform branch here
would be a second thing to be wrong about. Corral-side code keeps no
`cfg` either way.

## Interfaces or persistence changed

None. No wire type, no durable state, no public signature. `openpty`'s
observable contract is unchanged: same arguments, same successes, strictly
fewer spurious failures.

## Failure / unknown states

- A poisoned gate would be a panic in another thread while inside
  `openpty`; there is no unwinding call in the critical section, so the
  gate is taken through the poison rather than propagating it.
- A genuine allocation failure — real exhaustion, a sandbox with no
  `/dev/ptmx` — still fails, unchanged, and still reaches the caller as
  `SpawnError::Pty`. This removes a spurious failure; it does not invent a
  retry or hide a real one.
- Linux was not measured. The change is safe there by construction and is
  not claimed to fix anything there.

## Tests

`crates/corrald/src/runtime/spawn_tests.rs`, the vendored patch's
regression suite, gains one test: many threads allocate PTYs through
`native_pty_system().openpty(...)` at once and every allocation must
succeed. Sized from the measurement above — the observed rate reached 8
failures in 1600 calls — so it is a high-probability detector pre-fix and
deterministic post-fix, in under a second.

Pre-fix evidence is required and recorded: the test must be seen failing
with the same `posix_openpt` signature before the patch lands.

`CORRAL_PATCHES.md` gains PATCH 2, its measurement, and its regression
pointer, so the removal condition still names everything upstream has to
satisfy.

## Definition of done

- `./scripts/verify` green on the final tree.
- The new regression seen red before the patch and green after.
- `CORRAL_PATCHES.md` describes PATCH 2, the evidence, and the regression.
- No test made serial, retried, or given a wider timeout to pass.
