# PR8b — provider session stores, and resuming from another directory

> Required by ADR 0016 before it may be accepted, and by the founder's
> round-5 ruling (`docs/decisions/2026-09-02-pr8-attention-grill.md`): the
> history and resume measurements must exist as durable, version-specific
> evidence. Narrative and the rest of the run are in
> `docs/references/2026-09-02-pr8-attention-matrix.md`; this record is the
> support claim.

## Scope, and what it does not extend to

| | |
|---|---|
| Measured | 2026-09-02, ~13:00–13:30 and ~22:50 +08 |
| Host | `ne`, Ubuntu 24.04 x86-64, bare metal |
| Container | udocker 1.3.17, PRoot engine, image `node:22-bookworm`, container `spike` |
| Claude Code | **2.1.258**, native installer, `/root/.local/bin/claude` |
| Codex | **0.152.0**, npm install |
| Accounts | the founder's own, signed in inside the container |
| Driver | `scripts/matrix/drive.py` for the scenario captures; the resume runs below were single shell commands |

**These facts are sealed for these exact versions and no others.** The
founder's macOS installs — Claude Code 2.1.252, Codex 0.145.0 — were not
exercised, and nothing here transfers to them. In particular Codex 0.152.0's
resume and working-directory behavior does not seal Codex 0.145.0 for the
same fact. A version whose store layout has not been measured is not
enumerated (grill Q28).

PRoot rewrites `/proc/<pid>/exe`, so this environment can measure stores,
screens, hooks, and notifications, and cannot measure the Linux external-Know
process chain. That is the separate grill Q16 gate and is untouched here.

## Store layout

### Claude Code 2.1.258

```
/root/.claude/projects/-root-proj/48e6a7ca-5ed2-4748-9c89-c66acf33f80b.jsonl
/root/.claude/projects/-root-proj/memory/
```

- One directory per working directory, named by encoding the absolute path
  with `-` for every separator: `/root/proj` → `-root-proj`. The encoding is
  lossy — a path that already contains `-` is indistinguishable — so it is a
  label, never decoded back into a path (ADR 0016 D1, grill Q25).
- One `.jsonl` per session, named by the provider session id exactly as the
  hook payloads and `--resume` use it.
- A `memory/` directory sits beside the session files and is not a session.
- No sub-agent session files appeared on this version.
- The file's modification time advances as the session takes turns.

### Codex 0.152.0

```
/root/.codex/sessions/2026/09/02/rollout-2026-09-02T13-20-41-01a06247-6739-7052-ada5-f5bae6e4b904.jsonl
```

- `YYYY/MM/DD` directories under `~/.codex/sessions`.
- The thread id is the last 36 characters of the file's stem; the timestamp
  before it is the session's start, not its recency.
- Beside the rollouts: an append-only `session_index.jsonl` of
  `{id, thread_name, updated_at}`, a `thread_history_1.sqlite`, and a
  `migrate-rollouts` subcommand that calls the rollout files legacy. Whether
  Corral ever reads the index is reserved (grill Q9/Q25); it is not read now.
- Only interactive session threads were produced. `codex exec` rollouts were
  not exercised, so telling a headless rollout from an interactive one by
  file name alone is unmeasured.

## Resume from a directory other than the session's

Run from a fresh `/root/elsewhere`, which is not a git repository, against
sessions the scenario captures had left under `/root/proj`. Non-interactive
resume was used so no driver was needed; it is the same session loader the
interactive pickers use, which were not exercised separately.

### Claude Code 2.1.258

```
cd /root/elsewhere
claude --resume 48e6a7ca-5ed2-4748-9c89-c66acf33f80b \
  -p "Reply with exactly: pong" --output-format json
```

| | |
|---|---|
| Exit | 0, `"is_error": false` |
| Identity | `"session_id": "48e6a7ca-…"` — unchanged |
| History file | `projects/-root-proj/48e6a7ca-….jsonl` grew 29 919 → 37 772 bytes; mtime advanced; the file stayed where it was originally filed |
| Working directory | the conversation continued with `/root/elsewhere`; a new `projects/-root-elsewhere/` appeared holding only `memory/`, no session file |

### Codex 0.152.0

```
cd /root/elsewhere
codex exec -s read-only --skip-git-repo-check \
  resume 01a06247-6739-7052-ada5-f5bae6e4b904 "Reply with exactly: pong"
```

| | |
|---|---|
| Exit | 0; answered `pong` |
| Banner | `workdir: /root/elsewhere`, `session id: 01a06247-…` — unchanged |
| History file | the same rollout grew 87 062 → 93 863 bytes; mtime advanced |
| Working directory | the new turn records `"cwd":"/root/elsewhere"` beside the original `"cwd":"/root/proj"` lines |

### What this establishes

1. A provider session id resolves **without** its original directory, on both
   providers, on these versions.
2. Resuming does not fork or rename the session: the id survives and the
   original history file is what grows.
3. The resumed process adopts the **new** working directory, and the store
   records that it did.

Therefore identity and location are independent, and a store entry cannot
supply the directory a continuation should run in. ADR 0016 D5 requires the
initiating client to state it; the daemon never substitutes one (grill Q35).

## What this record does not establish

- Whether resuming touches Claude's file modification time in isolation
  (measured only together with taking a turn, which advances it anyway).
- Enumeration cost on a large store; both stores here were small.
- Any behavior of Claude Code 2.1.252 or Codex 0.145.0.
- Anything about the Linux external-Know chain (grill Q16 gate).
