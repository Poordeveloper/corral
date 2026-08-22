# S2 — Provider session identity across resume, verified first-party

> Spike evidence (`ROADMAP.md` §3 S2, the identity-stability half). Gates
> ADR 0002 D3. Facts observed on this machine on 2026-08-22; every claim
> below is from a run performed for this spike, not from documentation or
> memory. The versions matter: this is evidence about these versions, and
> the PR5/PR7 hook compatibility matrix re-verifies against whatever ships
> then.

## Versions and method

| | |
|---|---|
| Claude Code | 2.1.239, headless `-p`, project-scoped hooks |
| codex-cli | 0.145.0, `codex exec`, `-c notify=[…]` CLI override |

No global configuration was touched. Claude payloads were captured by
`SessionStart` / `UserPromptSubmit` / `Stop` / `SessionEnd` hooks in a
scratch project's `.claude/settings.json` writing stdin to a log; Codex by
a `notify` script injected per-invocation and by reading the rollout file.

## Claude Code 2.1.239

**Resume keeps the identity.** `claude -p --resume <id>` and
`claude -p --continue` both continue with the **same `session_id` and the
same `transcript_path`**; the transcript file is appended, not copied.
`SessionStart.source` distinguishes the cases: `startup`, `resume`, and
`fork`. Every hook payload observed carried
`{session_id, transcript_path, cwd, hook_event_name}`.

```text
run 1   -p                      session e670c1cf…  source: startup
run 2   -p --resume e670c1cf…   session e670c1cf…  source: resume     same file
run 3   -p --continue           session e670c1cf…  source: resume     same file
run 4   -p --resume … --fork-session
                                session 4ae35761…  source: fork       new file
```

**Fork carries no parent pointer.** `--fork-session` mints a new
`session_id` and a new transcript file. The `SessionStart` payload says
`source: "fork"` but **does not name the parent**, and the forked
transcript contains **zero references to the parent session id**. The
copied messages do keep their message-level `uuid`s — all 14 parent
message uuids reappear in the fork — so parentage is recoverable only by
prefix overlap, which is Heuristic assurance by definition.

## codex-cli 0.145.0

**Resume keeps the identity and the file.** `codex exec resume <id>`
continued the **same session id** and **appended to the same rollout
file** (`~/.codex/sessions/YYYY/MM/DD/rollout-<ts>-<uuid>.jsonl`); after
the resume the file still holds exactly one `session_meta` line:

```json
{"id": "01a02902-a37a-7e42-9c68-09e654ace54d", "source": "exec",
 "thread_source": "user", "history_mode": "legacy", …}
```

Session ids are UUIDv7 (time-sortable). The `notify` hook fires on turn
completion and **carries the identity**:

```json
{"type": "agent-turn-complete", "thread-id": "01a02902-…",
 "turn-id": "01a02903-…", "cwd": "…", "client": "codex_exec",
 "input-messages": […], "last-assistant-message": "ok2"}
```

Operational note: `codex exec` blocks reading stdin until EOF when stdin
is an open pipe — a spawner must close or null stdin or the run hangs.

## What this settles for ADR 0002

- **D3's blocking unknown is resolved for current versions: the provider
  session id is stable across resume on both providers.** NativeResume
  recognition by binding uniqueness on `(node, provider, external_id,
  kind)` works as drafted; no continuity-signal fallback is needed today.
  The fallback design question stays dead unless a re-verification fails.
- **D4 gains a hard fact: Claude fork lineage is not attested.** The hook
  says a fork happened but not of what. A `SessionForkedFrom` edge can be
  Deterministic when Corral itself launched the fork (it knows the parent
  it passed to `--resume`), and at best Heuristic (message-uuid overlap)
  when observed externally — never Attested from the payload alone. D7's
  "no guessed edges" therefore does real work.
- **Both providers expose an identity-bearing turn/attention signal**
  (Claude `Stop` hook; Codex `notify`), relevant to PR5's evidence model
  though not this ADR.

## Residual risks, stated

- Interactive-path resumes (Claude TUI `--resume` picker and `/resume`;
  Codex TUI `codex resume`) were not driven first-party — they share the
  session stores verified here, but PR5's matrix should touch them.
- Concurrent resume of an already-running session was not exercised.
- Codex `history_mode: "legacy"` implies a successor mode exists; a
  format change there is exactly the drift the version-pinned matrix
  exists to catch.
- The S2 scope items beyond identity — the real-world settings corpus,
  merge-ambiguity taxonomy, and fail-safe trigger set — are PR5/PR7
  material and remain open.
