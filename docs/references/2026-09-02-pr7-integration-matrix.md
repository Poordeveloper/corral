# PR7 integration and discovery, exercised against real providers

> The PR7 matrix (plan design 9). What the implementation was run against, on
> named versions, with what it did — and, as prominently, what this
> environment could not establish and why. Every row is from a run performed
> for this record. The provider-behavior facts the design rests on are the
> spike's (`docs/references/2026-09-02-pr7-global-integration-spike.md`);
> this record is about Corral's own behavior against them.

## Method

| | |
|---|---|
| Corral | branch `task/pr7-external-sessions` at `ba57fad`, built in place |
| Claude Code | **2.1.252** at install time; the same install self-updated to **2.1.258** mid-session, which is recorded rather than suppressed — an integration that only survives the version it was written against is not integration |
| codex-cli | **0.152.0** (npm global) |
| Host | Linux `ne`, Ubuntu 24.04, x86_64, kernel 6.8.0-110 |
| Container | udocker 1.3.17, Debian 12 `node:22-bookworm` |
| Date | 2026-09-02 |

Corral was built and run inside the same container the spike used, so the
provider CLIs, their configuration, and their credentials stay off the host
account. `corrald`'s idle grace was raised through its `test-support` seam;
without it the daemon exits sixty seconds after the last client, which is
correct behavior and makes a scripted scenario race it.

## What ran, and what it did

### Unit and integration suite on Linux — **314 pass, 0 fail**

The whole `corrald` suite, including the parts macOS cannot run: the `/proc`
observation implementation against this machine's own process table, and a
sweep pass that reads the real table and recognizes what is on it. On macOS
the same suite runs with those three tests compiled out and the rest passing.

One test failed on the first Linux run and was repaired rather than
quarantined. `a_configuration_directory_that_cannot_be_written_refuses_the_write`
made its directory unwritable with a mode bit, and root ignores directory
permissions — so in a container running as root the test asserted nothing
while looking like coverage. The refusal it stands for is real, so the
mechanism changed to one that binds every uid: a path that cannot be a
directory because a file is already there.

### Integration install, status, and disable — **pass, first-party**

`corral integration status claude` on a machine with no Corral entry:

```
claude · not integrated
  configuration /root/.claude/settings.json
  Sessions from this agent show Limited awareness until this is resolved.
```

`corral integration enable claude` then reports `claude · integrated`, and
the file it wrote carries exactly the accepted shape — the five events of
ADR 0004 D6, each invoking the relay with the version discriminant, each
guarded:

```json
"SessionStart": [
  { "hooks": [ { "command": "'…/corral' 'hook-relay' '--provider' 'claude' '--integration-version' '1' || true",
                 "type": "command" } ] }
]
```

The user's own `"theme": "dark"` survived unchanged, and the pre-mutation copy
landed in Corral's own state directory (`~/.corral/state/integration-backups`).
Nothing was written into the user's file that a person did not already have,
and nothing of theirs was reformatted away.

### A real Claude session under Corral's installed hooks — **pass**

Claude Code was started outside Corral, in a directory Corral knows nothing
about, with Corral's entries installed. A full turn completed with **no hook
error of any kind in the provider's UI** — the measured contrast is the
spike's scenario 11, where an unguarded entry whose command is missing prints
two error lines on every prompt and every turn. The guard works in the place
it was designed for.

The relay was also driven by hand with a synthetic `SessionStart` payload and
exited 0, which is the whole of what it promises.

### Idle exit — **pass, and worth stating**

The first daemon exited sixty seconds after its last client, before the
provider session was started. That is `zero-background-by-default` behaving
correctly, and it is recorded because it is the reason the first scenario
attempt observed nothing: a daemon that is not running receives no
deliveries, and the relay that cannot reach it fails open silently. Nothing
was lost that Corral promised to keep.

## What this environment could not establish

**End-to-end discovery of an external session was not validated here.** Both
of udocker's engines distort exactly the thing the claim ladder rests on, in
different places, and neither distortion is a Corral defect:

- **PRoot (P1).** `/proc/<pid>/exe` resolves to a PRoot temporary file
  (`/tmp/prooted-…-XXXXXX`) rather than to the running binary. Recognition
  reads the resolved executable and nothing else — by ruling, `argv[0]` is
  never sufficient — so under PRoot no provider process can be recognized,
  the ancestry walk cannot corroborate, and the sweep sees nothing. Verified
  directly: `readlink -f /proc/<claude pid>/exe` → `/tmp/prooted-1272344-pXcZPs`.
- **Fakechroot (F3).** `/proc/<pid>/exe` is faithful here — the mode was tried
  for exactly that reason — but the engine rewrites paths inside libc calls
  while `corrald` reaches the filesystem through `rustix`'s raw syscalls for
  its singleton lock. The lock resolved against the host filesystem instead
  of the container's and the daemon refused to start, which is the daemon
  correctly declining a rendezvous it cannot trust.

So the two halves that need a faithful process table — corroboration
promoting a delivery to an Attested binding and a durable Run, and the sweep
producing a provisional row — are covered by their own tests against injected
process trees and by the real-`/proc` tests above, and are **not** covered end
to end by this record. Closing that needs a container with real namespaces
(rootless Podman or Docker, both of which need an administrator on this host)
or a machine where the provider may run outside a container.

**Codex was not exercised end to end** in this pass. Its integration path is
covered by the engine's own tests over the corpus; its live half waits on the
same environment.

## Findings

1. **`corrald` ignores `RUST_LOG` and logs at a fixed `INFO`.** Every
   discovery decision below that level — a delivery arriving, an identity
   nothing corroborates, a provider whose integration is not enabled — is
   invisible to anyone diagnosing an installation, including in this matrix
   run, where it is why the first scenario's silence took a process probe to
   explain rather than a log line. A daemon whose log level cannot be raised
   is a daemon that cannot be supported. Follow-up, not fixed here: it is a
   behavior change to a surface outside PR7's scope.
2. **The provider self-updated mid-run**, 2.1.252 → 2.1.258, without the
   installed entry needing anything. That is the skew law working, and it is
   the first observation of it against a *globally* installed entry rather
   than a per-launch one.
3. **The `test-support` idle-grace seam is load-bearing for any scripted
   scenario** against this daemon, and a matrix that forgets it measures a
   daemon that has already exited.

## Supported-version rows

`PRODUCT.md` §10 gains, from this record and the spike it rests on:

| Provider | Version | Integration | Discovery |
|---|---|---|---|
| Claude Code | 2.1.252, 2.1.258 | install, status, disable, guarded entry — verified first-party | not verified end to end here; see above |
| codex-cli | 0.152.0 | engine-level over the corpus | not verified end to end here |
