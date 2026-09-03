# portable-pty — vendored with two Corral patches

Upstream:
: `portable-pty` 0.9.0 (wezterm), unmodified sources plus `LICENSE.md`.
  Vendored from the crates.io release; the packaging metadata
  (`Cargo.lock`, `.cargo_vcs_info.json`, `Cargo.toml.orig`) is not
  carried.

Reason:
: `close_random_fds()` runs inside `pre_exec` and closes every descriptor
  above 2 — including the `FD_CLOEXEC` pipe libstd uses to report exec
  failure to the parent. With that pipe gone, a failed `execve` becomes a
  child-side abort and the parent observes an ordinary exit, so Corral
  cannot distinguish *the command failed to exec* from *the command
  exec'd and later exited*. PR3 requires that distinction: a Run that
  never started and a Run that started and exited are different durable
  facts.

Upstream issue:
: wezterm/wezterm#7893 (open at the time of vendoring). 0.9.0 is the
  current published release, so there is no fixed version to pin. The
  fix is offered upstream, but upstream's merge and release schedule is
  not a correctness dependency of Corral.

Patch:
: **PATCH 1 — preserve `FD_CLOEXEC` descriptors during
  `close_random_fds()`** (`src/unix.rs`). No other behavioural change. A
  close-on-exec descriptor is closed by the kernel on a successful exec
  by definition, so skipping it leaks nothing.

Reason:
: `openpty()` fails intermittently when threads of one process allocate
  at once. Measured on macOS 25.5.0 by performing the call's steps
  separately: the failing step is always `posix_openpt` — opening
  `/dev/ptmx` — never `grantpt`, `unlockpt`, `ptsname`, or opening the
  slave. It is not exhaustion (peak 42 allocated against a
  `kern.tty.ptmx_max` of 511), and it is not `ptsname`'s static buffer,
  the obvious suspect, which never failed. The errno reaching userspace
  does not describe it: `ENXIO`, and a plain `-6`, which is not an errno,
  so nothing may branch on it.

  | shape                                    | calls | failures |
  | ---------------------------------------- | ----- | -------- |
  | 16 threads, one process                  | 6400  | 6        |
  | 16 threads, one process, one at a time    | 6400  | 0        |
  | 16 single-threaded processes, concurrent  | 6400  | 0        |

  For Corral this is `corral new` telling a person the daemon could not
  allocate a pty, at a rate that grows with how many sessions they start
  at once — and it took `./scripts/verify` red twice, in the daemon and
  in `corrald`'s own suite.

Patch:
: **PATCH 2 — allocate one PTY at a time per process** (`src/unix.rs`).
  A process-wide gate around the `libc::openpty` call. The critical
  section is one non-blocking call, and PTY allocation is a per-session
  event rather than a hot path. Unconditional rather than
  `cfg(target_os = "macos")`: the guarantee is a property of the
  allocation boundary, not of a platform, and Linux is unmeasured rather
  than known-good. A genuine allocation failure still fails, unchanged.

Corral-added files:
: `CORRAL_PATCHES.md` (this file) and `rustfmt.toml`, which disables
  formatting inside this directory so the delta against upstream stays
  exactly the patch declared above. No upstream file carries either
  change. The workspace also `exclude`s this directory, so the vendored
  crate is built without inheriting Corral's lints — its `unsafe` is
  upstream's, and Corral-owned crates keep `#![forbid(unsafe_code)]`.

Deliberately not patched:
: the `cwd` fallback in `cmdbuilder.rs`, which silently substitutes
  `$HOME` when the requested directory is not a directory. Corral
  validates the working directory itself and fails with a typed error
  before calling into this crate, keeping the vendored delta to one
  hunk.

Corral regression:
: `crates/corrald/src/runtime/spawn_tests.rs` — the whole suite, and in
  particular the pair that may never collapse into one observation
  again: `spawn_bad_shebang_is_error` (spawn fails) against
  `spawn_exit_1_is_distinguishable_from_exec_failure` (spawn succeeds,
  the child exits 1). PATCH 2's is
  `ptys_allocated_at_once_are_all_allocated`, sized from the measurement
  above: 32 threads allocating 100 PTYs each failed 10 runs out of 10
  before the patch and passed 10 out of 10 after.

Removal condition:
: an upstream release containing equivalent fixes passes the complete
  Corral PTY spawn compatibility suite on macOS and Linux. At that point
  this directory is deleted and the workspace depends on the published
  crate again.

Decision:
: `docs/decisions/2026-08-24-pr3-spawn-gate.md`.
