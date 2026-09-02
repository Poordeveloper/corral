# PR7 global integration — what the ADRs' load-bearing facts measured

> Evidence for the PR7 spike (`docs/plans/2026-09-01-pr7-integration-spike.md`),
> addressed to the grill that must accept, amend, or reject ADR 0013 and
> ADR 0014 (`docs/decisions/2026-09-01-pr7-integration-grill.md`). Every claim
> below is from a run performed for this record on the named versions — not
> from documentation, from memory, or from the PR5/PR6 matrices.
>
> Scenarios that could not be measured are recorded as such, with the blocking
> input named; §Blocked says exactly which ADR items still stand on nothing.

## Method

| | |
|---|---|
| Claude Code | **2.1.252** (`claude --version`), commit `c0778c45886d` |
| codex-cli | **0.152.0** (npm global) on Linux; **0.145.0** (npm global) on macOS |
| Host A | macOS, Darwin 25.5.0, arm64 — scenarios 7, 9, 10 and the local half of 1 |
| Host B | Linux `ne`, Ubuntu 24.04, x86_64, kernel 6.8.0-110 — scenarios 2, 3, 4, 5, 8, 11, and the container half of 1 |
| Corral commit | `3c2b98f` (branch `main`, before any PR7 implementation) |
| Date | 2026-09-02 |

Host B runs every provider inside a **udocker 1.3.17 container** (Debian 12
`node:22-bookworm`, PRoot engine), by founder direction (2026-09-02): the
provider CLIs, their config, and their credentials never touch the host
account. The host has no Docker or Podman and the test account has no root;
`kernel.apparmor_restrict_unprivileged_userns=1` blocks unprivileged user
namespaces, so rootless Podman is unavailable there and udocker's PRoot engine
was the only container that runs without administrator action. Claude Code was
installed inside the container by its native installer, Codex by npm. Claude
was signed in inside the container with the founder's claude.ai subscription
(OAuth code handed across by the founder), Codex with the founder's ChatGPT
account (device auth); no credential file was copied from any machine. Session
scenarios ran real turns against Opus 5 and `gpt-5.6-sol`.

**Measurement distortion, recorded rather than hidden.** PRoot gives no PID
namespace: container processes are ordinary host processes with host PIDs, and
`ps` inside the container sees the host's process table. The hook-to-provider
half of an ancestry chain sits *below* the PRoot layer and is measured here;
everything above the provider process (terminal, tmux, wrapper shapes) is
PRoot-distorted, and no conclusion about that half is drawn from Host B.

The relay stand-in is a script that appends its own pid, ppid, an eight-hop
ancestor chain (pid, ppid, start time, `comm`, `/proc/<pid>/exe`, cmdline),
its argv, and its verbatim stdin to a log. It replaces `corral`'s relay so the
observation ADR 0014 D1/D2 needs is measured without any Corral code existing.

## Scenarios

### 1. Corpus — the real files, and what a fresh install has — **partial**

This machine's real `~/.claude/settings.json` (macOS, 2.1.252) is strict JSON,
carries no `hooks` key and no `disableAllHooks`. The real `~/.codex/config.toml`
carries no `notify` key and does carry `[projects."…"] trust_level` entries,
which proves Codex persists its own answers into the same file Corral wants a
`notify` key in.

On a **fresh install neither file exists**: after installing Claude Code and
Codex into the clean container, `~/.claude/settings.json` and
`~/.codex/config.toml` were both absent (`~/.claude/` held only `backups`,
`downloads`, `sessions`). ADR 0013 D3's primary path is therefore *create*, not
*merge*; the merge engine's hard case is a file the provider itself wrote
later, not one the user hand-authored. Claude's own first write was observed
directly: answering the first-run theme prompt created `settings.json` with
exactly `{"theme": "dark"}`.

The public-dotfiles half of this scenario (third-party hook layouts,
`disableAllHooks` occurrences in the wild) is **not done**.

### 2. Claude rejects JSONC, and says so quietly — **pass, and it refutes a fallback**

Seven `~/.claude/settings.json` variants, each read by `claude doctor`
(documented to read settings without a trust prompt), file compared before and
after:

| settings.json | Claude 2.1.252 verdict | file rewritten? |
|---|---|---|
| strict JSON with a hook entry | accepted | no |
| `// line comment` | **`Invalid or malformed JSON`** | no |
| `/* block comment */` | **`Invalid or malformed JSON`** | no |
| trailing comma | **`Invalid or malformed JSON`** | no |
| unknown top-level key `corralIntegrationVersion: 1` | **accepted silently** | no |
| truncated JSON | `Invalid or malformed JSON` | no |
| `disableAllHooks: true` beside a hook entry | accepted, no complaint | no |

Two consequences, both load-bearing:

1. **Comments are fatal to the whole file, not to the hook block.** An invalid
   `settings.json` drops *every* setting in it — theme, permissions, env — not
   just hooks. Any design in which Corral writes a comment marker into
   `settings.json` (an ownership marker, a `corral start`/`corral end` fence)
   would silently break the user's entire Claude configuration. ADR 0013 D2's
   choice of **structural ownership plus an unknown-key version discriminant**
   is confirmed as the only safe shape, and the comment-marker fallback is
   refuted rather than merely disfavored.
2. **The unknown-key discriminant survives.** `corralIntegrationVersion` was
   accepted with no warning and preserved byte-for-byte.

Note for D4: `claude doctor` reports invalid settings but still prints
`No installation issues found.` and **exits 0**. A settings file Corral breaks
is a quiet failure the user may never be told about.

### 3. Claude rewrites the whole file, and keeps what it does not understand — **pass**

`~/.claude/settings.json` was seeded with four-space indentation, an unknown
key first, a `permissions` block, an `env` block, a Corral-shaped `hooks` entry,
and no `theme` key. Removing `theme` makes the provider re-prompt for it, which
forces a real provider write into an existing file with no Corral code involved.
After answering the theme prompt:

- key order **changed** — `corralIntegrationVersion` moved from first to last;
- indentation **normalized** — four spaces to two;
- the unknown key **survived**, value intact;
- `permissions`, `env`, and the `hooks` entry **survived**, structure intact.

Codex was measured on the same question through its own write path: with a
`#` comment, `model_reasoning_effort`, and `notify` already in `config.toml`,
answering the workspace-trust prompt made Codex persist
`[projects."/root/proj"] trust_level = "trusted"` by **appending** — the
comment, key order, and existing lines survived byte-for-byte. The two
providers sit at opposite ends: Claude reserializes the whole file, Codex
patches surgically and preserves what it did not write.

Two consequences for ADR 0013 D3:

1. **Format preservation buys nothing against Claude.** Whatever formatting
   Corral preserves is normalized away the next time the provider writes. The
   honest engine is parse → additive merge → serialize; a CST is unnecessary
   for `settings.json`. (It remains necessary for Codex's TOML, where comments
   are legal — see scenario 5.)
2. **The provider can silently drop Corral's block, and this is normal.** The
   provider reads the whole file and writes back what it parsed. A Corral write
   landing between that read and that write is lost with no error on either
   side. Corral's atomic same-directory rename protects the *user's* file from
   Corral; nothing protects *Corral's block* from the provider. This is measured
   support for grill Q10's ruling that **missing is not modified**: a hook entry
   absent at startup is the expected outcome of an ordinary race, not evidence
   that the user removed it, and repairing it is not a tug-of-war.

### 4. Layer semantics — **pass**

Which files are read at all, probed by poisoning one layer at a time with
malformed JSON and asking `claude doctor` which files it calls invalid:

| candidate path | read by 2.1.252? |
|---|---|
| `~/.claude/settings.json` (user) | **yes** |
| `<cwd>/.claude/settings.json` (project) | **yes** |
| `<cwd>/.claude/settings.local.json` (project-local) | **yes** |
| `/etc/claude-code/managed-settings.json` (enterprise) | **yes** |
| `~/.claude/settings.local.json` | **no such layer** |

For ADR 0013 D5 this fixes the ownership scope precisely: the only layer Corral
may write is the user layer `~/.claude/settings.json`. There is no user-local
layer to place a Corral-owned entry in, so Corral's block and the user's own
settings necessarily share one file — which is why D3's merge and D4's
fail-safe triggers exist at all.

Live sessions (print mode, one real turn each, relay stand-in counting
deliveries per layer tag):

- **Layers add.** The same event hooked at user, project, and project-local
  fired **all three entries, once each**, in one session.
- **`disableAllHooks: true` at *any* layer silences everything.** Placed in the
  project file, the project-local file, the enterprise file, or beside the
  user's own entry, it suppressed the user-level hook completely. The agent
  continues normally; nothing in the TUI or exit status says hooks were
  disabled. For D4 this means the trigger must be evaluated across **all four
  layers**, three of which Corral will never write — and a session under
  `disableAllHooks` is indistinguishable, on the hook channel, from a session
  that does not exist. ADR 0014's external-evidence path is the only thing that
  can see such a session.

### 5. Codex config parsing, and the `notify` slot — **pass**

Six `~/.codex/config.toml` states, each loaded by `codex login status`:

| config.toml | codex 0.152.0 verdict |
|---|---|
| absent | loads (`Not logged in`) |
| `#` comment + `model` + `notify = ["…"]` + `[projects."…"]` | loads |
| `notify` as a bare string | **`Error loading configuration: config.toml:2:10: invalid type: string "…", expected a sequence`**, exit 1 |
| unknown key `corral_integration_version = 1` | **accepted silently** |
| unclosed array | **`Error loading configuration: config.toml:2:12: unclosed array, expected `]``**, exit 1 |
| `notify` occupied by a third-party notifier with flags | loads |

Codex never rewrote `config.toml` on any of these paths.

The asymmetry between the providers is the finding. **A malformed
`settings.json` is a silent soft failure in Claude; a malformed `config.toml` is
a fatal hard error in Codex** that stops the CLI with a line and column. Both
outcomes are unacceptable for a default-installed integration, but they fail
differently, and D4's trigger list must treat a Codex write as the stricter of
the two. Unknown-key discriminants and `#` comments are both legal here, and
Corral can detect an occupied `notify` exactly — ADR 0013 D7's premise holds.

The firing half, measured on a logged-in interactive TUI (codex 0.152.0,
`gpt-5.6-sol`): **a config-layer `notify` (no `-c`) fires on turn completion.**
ADR 0009's open gap is closed. The payload arrives as **argv[1]**, one JSON
object:

```json
{"type":"agent-turn-complete","thread-id":"01a05f0a-8248-…","turn-id":"01a05f0a-d63d-…",
 "cwd":"/root/proj","client":"codex-tui","input-messages":["reply with exactly: ok"],
 "last-assistant-message":"ok"}
```

stdin is empty. Two structural facts beyond ADR 0009's `-c` measurement:

1. **Codex spawns the notify program directly — there is no shell.** The
   probe's parent was the codex native binary itself, not `sh`. The identity
   fields (0.152.0 says `thread-id`/`turn-id`) and the `client` discriminator
   are argv-JSON, and any guard construction that relies on shell syntax (see
   scenario 11) does not exist on this provider.
2. **One user turn produced two notifies.** 1.7 s after the real turn's
   notify, a second `agent-turn-complete` arrived for Codex's internal
   title-generation turn — carrying a **different `thread-id`** and the
   internal prompt as its `input-messages`. The notify channel emits internal
   utility turns as if they were sessions. A consumer that mints a Session per
   observed `thread-id` mints ghosts; this is direct evidence for ADR 0014
   D3/D5 (weak evidence collects freely, durable Runs and user-visible rows
   need corroboration).

### 6. Ancestry per host — **partial, lower half only**

The half below the provider is measured. A `SessionStart` hook process's chain,
captured live: **probe → `/bin/sh -c <command>` → the provider process itself**
(`exe` = the versioned `claude` binary), two hops, one `sh` intermediary.
Claude runs hook commands through `/bin/sh -c`, so the immediate parent is
never the provider and always a shell. The provider process was recognizable by
resolved executable at hop 2 while the hook still ran.

The half above the provider — terminal, tmux, screen, `nohup`/`setsid`,
wrapper-script shapes — is **not measurable on Host B** (PRoot inserts itself
into exactly that region of the chain) and remains blocked on a macOS run.

### 7. Recognition shapes per install channel — **partial**

macOS, Claude Code native/local channel: argv carries
`~/.claude/local/node_modules/.bin/claude`, which is a **symlink**; the true
executable is
`~/.claude/local/node_modules/@anthropic-ai/claude-code/bin/claude.exe`, a
native Mach-O arm64 binary whose `comm` reads `claude.exe` — and `comm` is
truncated at 16 characters, which was observed directly.

macOS, Codex npm channel: **two processes**, not one. A node wrapper
(`exe=node`, argv `node …/bin/codex`) spawns a native child
`…/@openai/codex-darwin-arm64/vendor/aarch64-apple-darwin/bin/codex`, `comm`
`codex`. Codex also spawns `git` children for plugin sync, so a naive
"descendant of a provider process" rule sees processes that are not the agent.

Linux container, Claude Code native installer: `~/.local/bin/claude` is a
symlink to `~/.local/share/claude/versions/2.1.252`, an **ELF x86-64
executable** — the same "argv path is not the executable" shape as macOS.
Linux, Codex npm: `/usr/local/bin/codex` is a `#!/usr/bin/env node` script that
`spawn`s its native child — the same two-process shape as macOS.

For ADR 0014 D2 the rule that survives both platforms and both channels is:
**resolve the executable, never trust argv[0], and expect the provider's own
identity to sit one hop below a language runtime.** The homebrew channel was
not exercised.

### 8. Double-fire and order — **pass**

User-settings entry plus a `--settings`-injected entry for the same two events,
one launch (print mode), one real turn:

```
1788281344.017330396  inj-ss     (SessionStart, injected)
1788281344.026493664  user-ss    (SessionStart, user)      +9.2 ms
1788281346.150538487  user-ups   (UserPromptSubmit, user)
1788281346.154881119  inj-ups    (UserPromptSubmit, injected)  +4.3 ms
```

**Both entries fire, every event.** Neither displaces the other; there is no
dedup and no override. The gap is milliseconds and the **order is not stable**
— injected won SessionStart, user won UserPromptSubmit, in the same session.
ADR 0014 D4's shape is confirmed with numbers: dedup is the daemon's job, keyed
on the delivery's session identity and event, arrival order must not be load-
bearing, and the managed channel stays authoritative.

### 9. Platform identity APIs on macOS — **pass**

`proc_pidinfo(PROC_PIDTBSDINFO)` plus `proc_pidpath`, measured against four
process states:

| target | result |
|---|---|
| live process, same user | `(pid, start time, executable path)` all available; start time has **microsecond** resolution |
| process owned by another user | `EPERM` — identity unavailable, and the failure is distinguishable |
| nonexistent pid | `ESRCH` |
| zombie | `ESRCH` — a zombie is **invisible** to this API |

Microsecond start times settle ADR 0014 D2's reuse question: `(pid, start_time)`
disambiguates a reused pid, because a reused pid necessarily has a later start
time. The `EPERM` and `ESRCH` split matters for the claim ladder: "I may not
look" and "it is gone" are different answers, and only the second supports
reporting a Run ended. The zombie result is the honest one — a zombie reports
`ESRCH`, i.e. gone, which is what Corral should say.

### 10. Sweep cost on macOS — **pass**

`proc_listallpids` plus per-pid `PROC_PIDTBSDINFO` and `proc_pidpath` over a
real desktop's process table (535 pids): **p50 0.79 ms, p95 1.85 ms, max
4.64 ms**; 317 pids returned full info (same-user only, the rest `EPERM`).

A full sweep is roughly a millisecond. ADR 0014 D2's periodic sweep does not
need a cadence chosen to protect the machine from the sweep; it needs one
chosen from how stale a Run may be, which is a product question and not a cost
question.

### 11. Missing command — the D8 stop check — **both providers measured; the stop condition fires on Claude only, and a construction disarms it there**

A hook entry pointing at a nonexistent command
(`/root/corral-missing-relay hook`) on `SessionStart`, `UserPromptSubmit`, and
`Stop`; a real interactive TUI session; two full turns. Observed, verbatim:

```
⎿  SessionStart:startup hook error
⎿  Failed with non-blocking status code: /bin/sh: 1: /root/corral-missing-relay: not found
```

- **Visible, and per-event.** The same two-line error appeared at session
  start, on **every** prompt (`UserPromptSubmit hook error`), and at **every**
  turn end (`Stop hook error`), with no per-session dedup — six error surfaces
  in a two-turn session.
- **Fail-open on progress.** The agent answered normally every time; the
  missing command adds no measurable latency (`sh` fails instantly, exit 127,
  reported as a "non-blocking status code").
- Control run: the same events with an existing command that exits 0 fast
  produced **zero** hook output in the TUI.

**Verdict against ADR 0013 D8 / grill Q6: the stop condition is met** for the
bare command shape. A stale default-installed integration whose relay binary is
gone would print two error lines on every prompt and every turn of every Claude
session — exactly the repeated disruption the ruling names. The bare-command
residual-failure shape cannot ship as a default install.

**The disarming construction, also measured.** Because Claude runs hook
commands through `/bin/sh -c`, the command string can carry its own fail-open
guard. The same three events configured as

    /root/corral-missing-relay hook || true

produced **zero visible output** across session start and a full turn: no
error, no hook line, agent normal. Claude's hook UI judges the **exit status
only** — `sh`'s own `not found` stderr line did not surface once the exit
status was 0. A guarded command makes the residual failure silent *by
construction*, independent of any uninstaller running. The cost is symmetric
and must be named: the guard also silences real relay crashes from the
provider's surface, so failure reporting becomes entirely Corral's own
responsibility (daemon-side, where ADR 0013 D6/D8 already place it). Whether
the guard becomes D8's mandated entry shape is the grill's call, not this
record's.

**The Codex half: silent by default.** `notify = ["/root/corral-missing-relay"]`
(nonexistent), interactive TUI, two full turns: **no visible output of any
kind** — no error line, no status change, no added latency, and nothing in
`~/.codex/log/`. Codex's direct spawn simply fails and the failure is
discarded. The D8 stop condition does **not** fire for Codex, and no guard is
needed there (nor possible — there is no shell to host one). The residual-
failure asymmetry is now fully mapped: Claude is loud per event unless the
entry carries its own guard; Codex is silent always.

## Unplanned findings

**An unauthenticated provider process is not a Session.** Hooks fired only
after login *and* after the workspace-trust answer; during onboarding and at
the login screen, valid `SessionStart`/`UserPromptSubmit` entries produced
nothing. Direct support for ADR 0014 D3's display gate and grill Q5: a `claude`
process in the process table may be a login prompt that will never have a
session id, never fire a hook, and never be controllable. Only a delivery
proves a Session.

**The delivery payload identifies the session on the first event.**
`SessionStart`'s stdin payload carried `session_id`, `transcript_path`, `cwd`,
`hook_event_name`, `source: "startup"`, and the model — the fields ADR 0014
D1's token-less delivery needs, present with no launch token involved.

**User-settings hook changes reach live sessions in seconds, with no review.**
A `settings.json` hook swap landed mid-session: an event a moment after the
write ran the *new* command while an adjacent event still ran the old one, and
no review or confirmation UI appeared for user-level changes. For D5 this means
repair-on-startup also heals already-running sessions; it also means Corral
writes to the user file take effect on sessions Corral did not launch,
immediately — one more reason D3's write path must never produce an invalid
intermediate state.

**A hung hook blocks its event for ~30 s, then the provider proceeds.** A
`sleep 70` UserPromptSubmit hook delayed the turn to 33 s wall (control: 3 s):
Claude 2.1.252 enforces roughly a 30-second hook timeout, then continues
without the hook's output. The provider's backstop is thirty seconds per event,
which is why ADR 0004's 50 ms relay self-budget — not the provider — is the
real protection, and why the relay must never wait on `corrald`.

## Blocked — and on what

- **ADR 0014 D1/D2 upper ancestry** — the provider-to-terminal half of the
  chain per host shape (direct terminal, tmux, screen, `nohup`/`setsid`,
  wrapper). PRoot distorts exactly that region; this is a macOS (Host A)
  measurement when a maintenance window allows, and the recognition rules for
  it must not be sealed from Host B data.
- **Corpus, public half** (scenario 1) — third-party dotfiles with hook
  layouts and `disableAllHooks` in the wild; a collection-and-provenance
  exercise, not blocked on anything but time.
- **Homebrew install channel** (scenario 7) — not present on either host.

## Findings addressed to the grill

1. **Comment-marker ownership is refuted, not merely rejected** (scenario 2).
   A comment in `settings.json` invalidates the user's entire Claude
   configuration silently. D2's structural ownership stands on measurement now.
2. **Format preservation is unnecessary for Claude and necessary for Codex**
   (scenarios 2, 3, 5). D3 should not describe one merge engine for both
   providers: `settings.json` is provider-normalized JSON where a CST buys
   nothing, while `config.toml` legally carries user comments a write must
   preserve.
3. **The provider can silently drop Corral's hook entry** (scenario 3). This
   is measured support for Q10's `missing ≠ modified` ruling, and it means
   repair-on-startup is the ordinary path rather than an exceptional one. The
   recurrence limit that Q10 deferred should be derived with this race in
   mind: a repair that follows a provider write is not evidence of a competing
   authority.
4. **The two providers fail with opposite loudness** (scenarios 2, 5). D4's
   trigger list is written once but lands on a silent-soft-failure provider and
   a fatal-hard-failure provider. The stricter provider sets the bar.
5. **The only writable layer is the shared user file** (scenario 4). There is
   no private layer for a Corral-owned entry; D3 and D4 are not avoidable by
   choosing a different file.
6. **`disableAllHooks` is a four-layer kill switch, three layers of which
   Corral may never write** (scenario 4). D4's trigger must read all four; a
   silenced session is invisible on the hook channel and only ADR 0014's
   external evidence can see it.
7. **D8's stop condition fires for Claude's bare command, and the `|| true`
   guard disarms it by construction; Codex is silent either way** (scenario
   11). The grill must either mandate the guarded entry shape for Claude
   (accepting that Corral alone reports relay failures — Codex already gives
   nothing else) or redesign the residual-failure shape some other way before
   ADR 0013 can be accepted.
8. **Double-fire is real, milliseconds apart, order-unstable** (scenario 8).
   ADR 0014 D4's daemon-side dedup, arrival-order-independent, is the only
   shape that survives the measurement.
9. **A process-table hit is not a Session** (unplanned). D3's display gate has
   direct evidence.
10. **The notify channel emits internal turns as separate `thread-id`s**
    (scenario 5). Minting a Session per observed provider identity mints
    ghosts; ADR 0014 D5's "durable Runs only at attested corroboration" is the
    only shape that survives, and recognition rules must expect provider-
    internal utility turns on the same channel as real ones.
11. **The two providers' own write styles bracket Corral's merge problem**
    (scenario 3). Claude reserializes everything it parsed; Codex appends and
    preserves. D3's engine must survive the first and imitate the second.
