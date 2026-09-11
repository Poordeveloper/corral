---
status: active   # rulings in docs/decisions/2026-09-06-m1-completion-grill.md Q7–Q11, Q16–Q18, Q21; D6 ruled 2026-09-11 (§Decision)
class: B         # implements accepted architecture; PR 1 trips the risk-surface detector (protocol schema) and is human-reviewed
writes: [crates/corral-protocol, crates/corrald, crates/corral-client, crates/corral-rendezvous, crates/corral, packaging, scripts/package, scripts/package-smoke, scripts/verify-release, install.sh, .github/workflows/release.yml, .gitignore, Cargo.toml, README.md, ARCHITECTURE.md]
reads: [crates/corral-desktop, docs/decisions/2026-09-06-m1-completion-grill.md, docs/adr/0001-corrald-activation.md, docs/adr/0006-provider-hook-integration-policy.md, docs/adr/0013-global-hook-integration.md, ROADMAP.md]
---

# M1 packaging — the bundle, one packaging entry point, install and uninstall, and the release workflow

## Status

**Accepted 2026-09-06** by the completion grill (Q7–Q11, Q16–Q18, Q21);
this plan materializes those rulings and decides nothing new. The one
place a ruling met an accepted invariant — the package smoke against ADR
0001 D1 — was ruled by the founder on 2026-09-11 (§Decision): no
`CORRAL_HOME`, ADR 0001 D1 unchanged, local smoke on the install surface
only, runtime smoke on a disposable release-runner account. The founder
also confirmed: no metadata file, install is not upgrade, `strip = true`
only, no icon. Four PRs, in order: 1 `hello.pid`;
2 `corral uninstall`; 3 `scripts/package`, `install.sh`, the package smoke
and its `verify-release` step; 4 the release workflow, README, glossary.
It ends where a tag produces a draft release a human can publish, and where
the notification plan has a real `.app` to probe inside.

## Goal

A tagged commit becomes `Corral.app` for Apple Silicon and a Linux
x86_64 tarball through one repository-owned script; a user installs with
one command that places the bundle, links the CLI, and enables the
integrations it discloses; `corral uninstall` refuses while Corral still
owns running work and otherwise removes exactly what install placed; the
release gate can prove all of that on the exact tagged commit.

## Non-goals

OS notifications and the Design-0 probe (`m1-notifications`, which this
plan unblocks). `docs/references/supported-matrix.md`,
`check-provider-matrix`, the dogfood evidence gates (`m1-release-gate`).
Obtaining Developer ID credentials (external, Q9); universal binaries and
Intel (Q9, Q16); aarch64 Linux, a `.desktop` file, a Linux tray (Q10).
Upgrading: install is not upgrade. The installer refuses an existing
installation (D4); in-place upgrade, with the daemon and state migration,
rollback and interruption questions it carries, is its own later design. A frozen installer asset (Q17). The
`0.0.0 → 0.1.0` bump, which is the release-time PR (Q18). Auto-update.
Any change to how integrations are written: the daemon stays the only
mutator (ADR 0013 D1).

## Existing owner / architecture involved

- `corral-client::spawn::sibling_daemon` resolves `corrald` as the sibling
  of the canonicalized `current_exe()`; the Desktop's `Bridge` uses the
  same path. `corrald::provider::launch::sibling_relay` resolves `corral`
  the same way and bakes `<dir of corrald>/corral hook-relay …` into the
  user's provider config. So the three executables are siblings at a path
  that must stay stable across upgrades: Q7's `Corral.app/Contents/MacOS`
  and Q10's `~/.local/share/corral/bin` satisfy both.
- `corral-rendezvous::paths`: the corral root is `<account home>/.corral`,
  from the account database, never `$HOME` (ADR 0001 D1); `provider_home()`
  is the same home. The `test-support` namespace (`CORRAL_TEST_ROOT`)
  redirects both and does not exist in a production binary
  (`check-test-support-boundary`).
- `corrald::server::watch_signals` already turns SIGTERM/SIGINT into
  `Lifecycle::commit_shutdown`; on exit live PTY children are hung up by
  the kernel, not reaped — a daemon stopped under running sessions ends
  the user's work. Live managed runs also hold off idle exit.
- `ServerHello` carries no `pid`; there is no lifecycle RPC (Q21 keeps it
  that way). `integration.{status,enable,disable}` exist as RPCs and CLI
  subcommands; `repair_at_startup` re-installs drifted entries.
- No bundle metadata, no `[profile.release]`, no deployment target, no
  packaging or release scripts; CI is `ci.yml` (ubuntu-latest verify +
  PR checks) and `scheduled.yml` (macos-latest verify weekly). README says
  nothing is packaged.

## Design

### D1 — `hello` names the daemon (Q21)

`ServerHello.pid: Option<u32>` (`serde(default, skip_serializing_if)`);
`corrald` fills `std::process::id()`. `corral-client` exposes it on the
established connection as `daemon_pid() -> Option<u32>`; `None` means an
older daemon and is never read as a pid. No capability string: a field's
absence already says everything.

### D2 — Installation (Q7, Q10)

`crates/corral/src/installation.rs` names the two shapes and recognizes
them from the canonicalized `current_exe()`, nothing else:

```text
macOS  ~/Applications/Corral.app/Contents/MacOS/{corral-desktop,corrald,corral}
Linux  ~/.local/share/corral/bin/{corral,corrald,corral-desktop}
both   ~/.local/bin/corral -> the installed corral
```

An executable anywhere else (a `target/` build, a copied binary) is not an
installation and `uninstall` refuses by name. The home these paths hang
off is `corral-rendezvous::provider_home()`, whose body already resolves
the user's home as Corral sees it (the account database, or the test
namespace's `provider-home`) and is named only for its first consumer;
it is renamed `user_home()` with the same body and a doc comment naming
both tenants — provider dotfiles and the CLI symlink. Corral's home is
Corral's, not a provider's; the corral root keeps its own resolution. `Info.plist`: `CFBundleIdentifier com.poordeveloper.corral`,
`CFBundleExecutable corral-desktop`, `CFBundleName Corral`,
`CFBundleVersion` and `CFBundleShortVersionString` = the workspace version,
`CFBundlePackageType APPL`, `NSHighResolutionCapable`,
`LSMinimumSystemVersion` from one constant in `scripts/package` (Q16: the
lowest macOS major the release workflow verifies; PR 4 sets the number when
it pins the runner). No `LSUIElement`; no icon asset in this plan.

### D3 — `scripts/package` (Q9, Q11, Q16)

One entry point: builds `corral`, `corrald`, `corral-desktop` with the
release profile (`[profile.release] strip = true`, nothing else) and
`MACOSX_DEPLOYMENT_TARGET` = the same constant; assembles `dist/Corral.app`
from `packaging/macos/Info.plist.in` with the version substituted; signs
each executable and then the bundle ad hoc (`codesign -s -`); with
`CORRAL_SIGNING_IDENTITY` set signs with hardened runtime and a secure
timestamp, and with `CORRAL_NOTARY_PROFILE` set submits through
`notarytool --wait` and staples; archives with `ditto -c -k --keepParent`
to `Corral-<version>-macos-arm64.zip`. On Linux:
`corral-<version>-linux-x86_64.tar.gz` with the three executables under
one top-level directory.
Writes `dist/SHA256SUMS` over the archives; `dist/` is git-ignored. Version
travels in the asset names, the plist, and `corral --version`; there is no
separate metadata file. Credentials absent → ad-hoc build, exit 0, and the
log says which class it produced (Q9: the PR merges without credentials).

### D4 — `install.sh` (Q8, Q17)

POSIX `sh`, served from `main`, `curl -fsSL … | sh`. In order: OS/arch
(refuse anything but macOS arm64 and Linux x86_64 by name); resolve the
release — latest non-draft non-prerelease via the GitHub API, or
`CORRAL_VERSION=vX.Y.Z`; download the archive and `SHA256SUMS`; verify the
sum before extraction; refuse when an installation already exists at the
target ("Corral is already installed at …; run `corral uninstall` first —
upgrading in place is not part of M1"); extract into place; link
`~/.local/bin/corral`;
say the PATH line to add when `~/.local/bin` is not on PATH; detect
providers by `~/.claude` / `~/.codex` or `claude` / `codex` on PATH; print
which integrations it will enable; `corral integration enable <provider>`
each; print per-provider status. A provider failure is reported with the
resolution path and never rolls the installation back.
`CORRAL_INSTALL_FROM=<dist dir>` installs from local artifacts instead of
downloading (the smoke uses it; still verifies `SHA256SUMS`). `$HOME` is the installer's home; the
installed binaries resolve the account home, which is the same directory in
any supported setup, and the smoke keeps them equal.

### D5 — `corral uninstall [--purge]` (Q8, Q21)

Runs only from an installation (D2). Sequence, each step fatal on failure
with nothing later attempted:

1. activate the sibling daemon (it is the only reader of intent and the
   only mutator of provider files);
2. preflight: `session.list`; any managed run Running → "Corral is still
   managing N running sessions"; any Unknown → "could not verify whether U
   managed sessions have ended"; both listed, exit non-zero, nothing
   touched. No `--force`.
3. `integration.disable` for every provider whose intent is Enabled or whose
   standing is Installed/Drifted, then `integration.status` must read
   NotInstalled for each; otherwise stop here, before any binary is removed;
4. `hello` → `pid` (an older daemon without one → stop here: "this daemon
   predates uninstall; upgrade first"); SIGTERM through
   `rustix::process::kill_process`; poll liveness for 10 s; still alive →
   "corrald did not exit; the installation is kept", never SIGKILL; a
   permission error is reported as such; no such process (it exited on
   its own between `hello` and the signal) → continue;
5. remove `~/.local/bin/corral` only if it resolves into this installation;
6. remove the installation directory;
7. `--purge` only now removes the corral root, and its help text says it is
   destructive once the epoch is `dogfood`; default keeps `~/.corral`.

Q21's "daemon not running → continue" is moot here: step 1 makes it live,
and an activation failure is an install-integrity error, not a reason to
guess about integrations.

### D6 — Package smoke and the release-gate step (Q11; ruled §Decision)

Local packaging smoke validates the artifact installation surface. Runtime
lifecycle smoke validates the production binary on an isolated release-
runner account. Two environments, one script: `scripts/package-smoke`
takes a `dist/`.

Default level (local, `verify-release`, PR CI): the archive unpacks to the
D2 shape; the plist carries the keys above with the workspace version;
`codesign --verify --deep --strict` passes (macOS); `install.sh` with
`CORRAL_INSTALL_FROM` into `HOME=<temp>` places the bundle and the
symlink; `corral --version` through the symlink prints the version;
`readlink -f` lands inside the installation. It proves the package installs
correctly. No daemon starts and no provider file is touched: the production
binaries resolve the real account home and must (ADR 0001 D1).

`--disposable-account` (the release runner only, a throwaway user with its
real `~/.corral`): install from the built artifact → activation through
the symlink → `hello` → a CLI interaction (`corral list`) →
`corral uninstall` → SIGTERM observed → binaries and symlink gone, corral
root kept. This is closer to a real user than any namespace would be. The
same lifecycle is proven locally by the e2e tests in §Tests against
test-support builds in the D2 layout. `verify-release` replaces "packaging
/ install / uninstall" with `scripts/package && scripts/package-smoke` and
keeps exiting 1 for the gates still missing.

### D7 — Release workflow (Q13, Q16, Q18)

`.github/workflows/release.yml` on tags `v*`: `version` job fails unless the
tag equals `workspace.package.version`; `verify` on `ubuntu-24.04` and on a
pinned macOS image (not `macos-latest`) on the tagged commit, both calling
`./scripts/verify`; `package` per OS after its verify, then
`package-smoke`, then `package-smoke --disposable-account`; `release` job
collects `dist/`, re-checks `SHA256SUMS`, and creates the draft with
`gh release create --draft --verify-tag`, archives and manifest attached,
under `permissions: contents: write`. Publishing stays human (Q18). The macOS image's major fixes the
`LSMinimumSystemVersion` constant (D2); `scheduled.yml` keeps its weekly
run as regression evidence only. `verify-release`'s "multi-platform release
checks" line becomes a pointer to this workflow — the gate cannot run two
OSes locally.

### D8 — Records

`ARCHITECTURE.md` §11 gains *Installation* (the canonical location of the
three executables on one machine plus the CLI symlink; what install places
and uninstall removes). README replaces "nothing is packaged yet" with the
install command, the support claim (macOS arm64, Ubuntu 24.04 x86_64), and
`corral uninstall [--purge]`.

## Interfaces or persistence changed

Wire, additive: `ServerHello.pid`. Durable: nothing. Provider files: the
existing `integration.enable/disable` paths only. New user-facing paths:
D2. New environment: `CORRAL_VERSION`, `CORRAL_INSTALL_FROM` (installer);
`CORRAL_SIGNING_IDENTITY`, `CORRAL_NOTARY_PROFILE` (packaging). No new
crate dependency: `rustix` with `process` is already in the workspace.

## Failure / unknown states

Installer: unsupported platform, no matching asset, checksum mismatch, live
daemon → refuse before placing anything; provider enable failure → report,
keep the installation. Uninstall: not an installation, Running or Unknown
runs, integration cleanup incomplete, no pid from hello, daemon survives
SIGTERM, no permission → refuse or stop with the installation intact and
the reason named; a partially removed installation cannot occur before
step 5, and steps 5–6 are idempotent. Package: missing credentials is not
a failure; a failed notarization is. Smoke: any step failing fails
`verify-release`; nothing is skipped.

## Tests

- Protocol: `ServerHello` fixtures with `pid` absent (→ `None`), present,
  and with unknown fields beside it.
- Daemon e2e: the pid in `hello` is the live daemon's (signal 0 succeeds
  and the process's executable is the sibling `corrald`).
- Uninstall e2e (harness, test-support binaries copied into a D2-shaped
  directory under the isolated account; the symlink under the test
  namespace's home): refused with a holding mock-provider run (Running);
  refused with an Unknown run produced the way the lifecycle tests produce
  one; refused from a non-installation path; succeeds with an enabled
  Claude integration — `settings.json` back to NotInstalled, socket gone,
  pid dead, symlink and directory gone, corral root kept; `--purge` removes
  the root; an older daemon (a hello without `pid`) stops before step 5 —
  a unit test of that decision, since no released daemon exists to dial.
- `stop_daemon` unit: against `sh -c 'trap "" TERM; sleep 30'` returns
  did-not-exit and the child is still alive (the test kills it); against a
  child that exits on TERM returns Ok.
- Package smoke: the default level in `verify-release` on both OSes; the
  `--disposable-account` level in the release workflow only.
- Release workflow: a tag whose version disagrees with the workspace fails
  in `version` (proved by the first mismatched dry-run tag on a fork or by
  `act`, recorded in the PR).

## Definition of done

- PR 1 merged: D1 with protocol fixtures and the daemon e2e.
- PR 2 merged: D2, D5, `user_home()`, uninstall e2e and unit tests.
- PR 3 merged: D3, D4, D6;
  `./scripts/verify-release` reaches and passes the package smoke before its
  designed exit 1, on macOS and on Ubuntu.
- PR 4 merged: D7, D8; a dry-run tag on a fork or branch produces a draft
  release with both archives and `SHA256SUMS`, and the version check has
  been seen failing once.
- Founder walk on the real machine: `curl … | sh` from a draft release's
  assets, Desktop launches from `~/Applications` with the tray, `corral`
  works from PATH, `corral uninstall` refuses with a session running and
  succeeds after it ends, `~/.corral` survives. Recorded in
  `docs/references/2026-09-XX-packaging-walk.md`.
- `./scripts/verify` green on every final tree.

## Decision

Asked 2026-09-11: Q11 requires the package smoke to run install →
activation → handshake → uninstall isolated from the real `~/.corral` and
provider configs, but release-build binaries resolve the account home from
the account database with no override (ADR 0001 D1) and carry no test
namespace (`check-test-support-boundary`). Options: D6 as written, or a
production override such as `CORRAL_HOME`.

Ruled by the founder the same day, verbatim:

```text
D6: ACCEPT

Do not add CORRAL_HOME.
Keep ADR 0001 D1 unchanged.

Local smoke:
artifact/install surface only.

Release smoke:
real production binary on disposable runner account,
using real ~/.corral resolution.
```

Reasoning recorded with it: the account-database resolution is a security
boundary, not a path convenience; an override would give the daemon a
second namespace, make user machines and CI behave differently, and set
the precedent for `CORRAL_PROVIDER_CONFIG`, `CORRAL_DAEMON_STATE`,
`CORRAL_SOCKET_PATH` until the canonical rendezvous is eroded. The daemon
is not untested; artifact validation and runtime validation are split
across two environments. Also ruled in the same message: no metadata
file (the workspace version reaches the binary, the plist, the tag and the
file name — one truth source); install is not upgrade, M1 is fresh install
only; `strip = true` and no other release-profile change (LTO, panic
strategy, codegen units and compression change reproducibility); no icon;
PR 2 must show the SIGTERM permission-failure, daemon-absent and
daemon-does-not-exit behaviours, with no SIGKILL fallback; rename
`provider_home()` only because its body is the user's home, so that
"provider" does not leak from the integration layer into filesystem
identity.

## Plan Size Justification

Over the target because Q7–Q11 and Q21 are one owner boundary: the layout
the bundle fixes is the layout uninstall recognizes, the installer places,
the smoke checks, and the workflow ships; `hello.pid` exists only for
uninstall. Splitting the layout from its consumers would let one PR fix a
path the next must silently match. The four PRs stage the work; the plan is
the single place the layout is written down.
