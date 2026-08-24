# Founder Decision Record — the PR3 spawn gate and the portable-pty vendor

> Status: founder-accepted, 2026-08-24. Materialized by
> `third_party/portable-pty/` and the Design 1 runtime landing with this
> record. Ruled on measured evidence from a throwaway probe against
> `portable-pty` 0.9.0 on macOS, run under the gate the PR3 plan grill
> (Q1) made a prerequisite.

## The invariant under test

    PR3 requires Corral to distinguish:

    A. command failed to exec
    from
    B. command successfully exec'd and later exited.

## What the probe measured

Unpatched `portable-pty` 0.9.0, macOS:

| case | behaviour | verdict |
|---|---|---|
| normal spawn; exit codes 0 / 1 / 42 | reported faithfully | pass |
| initial size 31×113; setsid + process-group/session leadership; valid cwd | correct | pass |
| nonexistent executable; missing exec bit | typed spawn error (pre-fork checks) | pass |
| **post-fork execve failure** (dangling shebang interpreter) | `spawn OK`, child aborts inside the Rust runtime, parent observes **exit 1** | **violates the invariant** |
| invalid cwd | silently replaced with `$HOME`, command runs | silent argument rewrite |
| ENOEXEC garbage binary | exit 126 with a shell diagnostic | not a defect: libstd uses `execvp`, and POSIX prescribes the `/bin/sh` fallback — a real process did run |

Mechanism, confirmed in the source: `close_random_fds()` inside
`pre_exec` closes every descriptor above 2, including the CLOEXEC pipe
libstd uses to report exec failure to the parent. With the pipe gone, a
failed `execve` becomes a child-side abort that the parent reads as an
ordinary exit. Upstream issue: wezterm/wezterm#7893 (open); 0.9.0 is the
current published release, so there is no fixed version to pin.

## The decision — Option 3

    Vendor portable-pty 0.9.0 with one narrowly-scoped upstreamable
    patch, plus Corral-owned argument validation.

Option 2 (Corral-side prevalidation alone, unpatched dependency) is
**insufficient**: shebang-interpreter failure, TOCTOU, and any other
failure occurring after prevalidation remain indistinguishable from a
successful exec that later exited. Option 1 (pin a fixed upstream) does
not exist. Option 3b (fork the WezTerm monorepo and pin a git revision)
is heavier than vendoring one small crate for a single reviewable hunk:
it adds a git source, an external fork's lifecycle, revision
availability, and a cargo-deny git-source exception.

## Patch boundary — locked

    PATCH 1:
    preserve FD_CLOEXEC descriptors during close_random_fds()

    NO OTHER behavioral changes.

A CLOEXEC descriptor is closed by the kernel on a successful exec by
definition; closing it early only destroys Rust's error reporting when
the exec fails.

**The cwd fallback is deliberately not patched.** Layering:

    Corral layer:
    invalid/missing cwd
    → typed validation failure
    → never calls portable-pty

    portable-pty vendor patch:
    only repairs exec-failure reporting

This keeps the vendored delta to one hunk, so removing the vendor later
is easy.

## Provenance and exit

`third_party/portable-pty/` carries the upstream sources, the upstream
LICENSE, and `CORRAL_PATCHES.md` naming: upstream crate and version, the
reason, the upstream issue, the Corral regression tests, and the removal
condition — *an upstream release containing an equivalent fix passes the
complete Corral PTY spawn compatibility suite on macOS and Linux*. Cargo
uses a local path patch/pin; no long-lived private fork.

Submitting the fix upstream is the right thing to do and is **not a
correctness dependency of PR3**: when upstream merges or releases must
not block Corral.

## Evidence becomes permanent tests

The probe was a throwaway; the evidence is not. The suite is permanent:

    spawn_missing_executable_is_error
    spawn_non_executable_is_error
    spawn_bad_shebang_is_error
    spawn_invalid_cwd_is_rejected_before_pty_spawn
    spawn_exit_1_is_distinguishable_from_exec_failure
    spawn_exit_42_is_preserved
    pty_child_is_session_and_process_group_leader
    pty_resize_round_trips

The load-bearing pair:

    bad shebang        → spawn Err(ENOENT)
    real program exit(1) → spawn Ok(child), wait → exit 1

These two observations may never collapse into one again. The macOS
probe is evidence for the vendoring decision, **not** a substitute for
Linux correctness: the suite runs on Linux in `./scripts/verify` through
platform CI.

## Governance classification

This does not raise PR3 to Class C:

    High-consequence Class B
    + new third-party dependency
    + vendored modification
    → HUMAN_REVIEW_REQUIRED → human merge

The dependency strategy is founder-approved here. No ADR for a
Corral-authored unsafe boundary crate is needed, because that path was
not taken. Existing `unsafe` inside the vendored crate is **third-party
vendored code and does not expand Corral's unsafe boundary**;
Corral-owned crates keep `#![forbid(unsafe_code)]`, and vendored
third-party `unsafe` is not counted as a Corral policy violation.
