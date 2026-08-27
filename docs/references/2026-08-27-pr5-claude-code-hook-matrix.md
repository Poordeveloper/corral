# Claude Code hook injection and identity, re-verified first-party

> Compatibility evidence for PR5 (`ROADMAP.md` §3; plan design 9). Every claim
> below is from a run performed for this record on this machine, not from
> documentation, from memory, or from S2. It is evidence about **these
> versions**: `PRODUCT.md` §10's supported provider/version matrix begins here,
> and the follow-up that automates it under `verify-release` is named in the
> PR5 plan.

## Method

| | |
|---|---|
| Claude Code | **2.1.247**, installed at `~/.claude/local/claude` (local install, self-updating channel) |
| OS | macOS, Darwin 25.5.0, arm64 |
| Corral commit | `c091382` (branch `task/pr5-claude-managed-sessions`, before the implementation landed) |
| Date | 2026-08-27 |

Hooks were injected **only** through `--settings <file>` on the command line,
pointing at a scratch file that declares `SessionStart`, `UserPromptSubmit`,
`Stop`, `SessionEnd`, and `Notification`, each running a capture script that
appends its verbatim stdin to a log. No global or project configuration was
written or modified. Interactive scenarios were driven on a real pty by a
scripted keyboard, with every `CLAUDE*` environment variable of the enclosing
session scrubbed from the child.

Captured payloads are committed as
`crates/corrald/fixtures/claude-hooks/`, sanitized only where they carried a
developer's own paths and prompts; the structure is exactly what Claude Code
wrote.

## Scenarios

### 1. `--settings` injection fires the injected events — **pass**

Command: `claude -p "…" --settings <file>` and the same interactively.

Expected: the five injected events fire; each payload names the session.

Observed: `SessionStart`, `UserPromptSubmit`, `Stop`, and `SessionEnd` fire on
every run, headless and interactive. `Notification` fires **interactively**,
with `message: "Claude is waiting for your input"`, when the agent has finished
and is idle at the prompt; it does not fire in a headless `-p` run, which never
waits for a person. Every payload carries `{session_id, transcript_path, cwd,
hook_event_name}`; 2.1.247 adds `prompt_id`, `permission_mode`, `effort`,
`stop_hook_active`, `last_assistant_message`, `background_tasks`,
`session_crons`, `reason` on the events that have them.

Limitation: `Notification`'s trigger is idleness at the prompt, not every block
on the user. Corral treats it as one reported fact among several and asserts
nothing from its absence.

### 2. Identity holds across `--resume` and `--continue` — **pass**

```text
run 1   -p                       session d2dfcafd…  source: startup
run 2   -p --resume d2dfcafd…    session d2dfcafd…  source: resume
run 3   -p --continue            session d2dfcafd…  source: resume
```

Expected: the same `session_id` and the same `transcript_path`.

Observed: both, on both flags. `SessionStart.source` distinguishes `startup`
from `resume`. This is S2's finding, re-verified on 2.1.247.

### 3. `--settings` composes with `--resume` — **pass**

Expected: a resumed run still runs the injected hooks.

Observed: yes. Runs 2 and 3 above carried `--settings` and fired the full event
sequence. This is what makes a Corral continuation attested rather than a
resume Corral cannot see.

### 4. `--settings` is read once, at startup — **pass**

Expected (the assumption plan design 8 refuses to rely on until verified): the
injected file is read when the process starts, not re-read per hook.

Observed: an interactive session was started with a settings file pointing at
log A. After its first turn completed, the file was overwritten in place with
one pointing at log B. The second turn's `UserPromptSubmit` and `Stop`, and the
final `SessionEnd`, all still wrote to **log A**. Log B stayed empty.

Consequence: nothing in PR5 leans on this. It means a Corral-owned file removed
after its Run's established exit cannot affect a session that is still running
— which is a reason the cleanup rule is *safe*, never the reason it is
*allowed*. The allowance is still ownership evidence (grill Q10).

### 5. In-session conversation switching produces a second identity — **pass, and this is the contested path**

Expected: unknown. The design assumed a runtime could come to represent a
different provider conversation; ADR 0004 D8 makes that durable and inert.

Observed, in **one** interactive process, under **one** injected settings file
and therefore one Corral launch token:

```text
SessionStart  e614e016…  source: startup     the conversation it began with
SessionEnd    e614e016…
/resume  →  picker  →  a different conversation selected
SessionStart  a3fd3168…  source: resume      a different session_id
SessionEnd    a3fd3168…
```

Consequence: the contested condition is real and reachable on 2.1.247, not a
hypothetical. A managed launch can report an identity that contradicts the one
Corral accepted, over a valid token, without the process restarting. The
`binding-contested` fact and the `IdentityContested` refusal are answering an
observed behaviour.

### 6. Concurrent resume of a still-running session is **permitted by the provider** — **observed, and refused by Corral anyway**

Expected: unknown. Grill Q7 ruled that Corral refuses a continuation whose
previous run's exit is not established, and asked for observation rather than a
license.

Observed: while an interactive session was live and holding conversation
`074576f6…`, a second process ran
`claude -p "…" --resume 074576f6…` from the same directory. It **succeeded**,
exit 0, and produced a reply. Two live executions therefore can drive one
provider conversation.

A first attempt against a conversation with no completed turn failed with
`No conversation found with session ID: …` — a session id is resumable only
once it has been written to the transcript store.

Consequence: the provider permits what grill Q7 refuses. That the provider
allows it is not evidence that it is safe, and it is exactly why Corral's
refusal is hard: no `--force`, no "I know it is dead", no pid heuristic.

### 7. The interactive `/resume` picker path — **pass, covered by scenario 5**

S2 left the interactive picker as residual risk. It was driven here: `/resume`
opens a searchable picker of the project's conversations, and selecting one
switches the running process to that conversation's identity, as scenario 5
records.

### 8. Repeated `--settings`: the **last** one wins — **pass, and it changed the code**

Expected: unknown. The implementation had assumed the opposite.

Command: `claude -p "…" --settings <A> --settings <B>`, where A and B declare
hooks writing to different logs.

Observed: log B received both hooks; log A stayed empty. The last `--settings`
is the one loaded, and the earlier one is ignored entirely.

Consequence: Corral's injected file goes **after** anything a caller passes, or
a caller's own `--settings` would displace it and the session would launch
looking managed and reporting nothing. A caller-supplied `--settings` is also
refused outright, so a person is told rather than having their file silently
ignored. There is no short alias for the flag on 2.1.247.

### 9. `/clear` starts a new conversation in the same runtime — **pass, and it is a known cost**

Expected: unknown; scenario 5 raised the question for the picker, and `/clear`
is the same shape by another route.

Observed, in one interactive process under one injected settings file:

```text
SessionStart   8593a016…  source: startup
UserPromptSubmit / Stop / SessionEnd   8593a016…
SessionStart   cbb54bac…  source: clear
```

`SessionStart.source` distinguishes it: `clear` rather than `resume`.

Consequence, stated plainly because it is a real cost a person can hit: under
the accepted design, `/clear` in a Corral-managed session contests that
Session's provider binding, and contested is monotonic with no way to clear it
in M1 (ADR 0004 D8). Continuing that Session is refused from then on.

That is fail-closed rather than wrong — after a `/clear` the conversation
Corral recorded is not the one the runtime is on, and continuing the recorded
one would silently resume what the person just cleared away. But it makes a
routine action permanently costly, and it is the strongest argument yet for the
correction / re-identification mechanism ADR 0004 D8 already names as future
work. Corral records the normalized origin (`replaced`) in its diagnostics so
the cause is findable; nothing decides on it.

### 10. A caller's `--` swallows Corral's own flags — **pass, and it changed the code**

Expected: unknown. Scenario 8 fixed *where* Corral's `--settings` goes; this
asks whether position is enough.

Command: `claude -p "…" -- --settings <file>`.

Observed: the run completed normally and **not one hook fired**. Everything
after the end-of-options marker is a positional argument, so `--settings` was
read as prompt text.

Consequence: an injection placed *after* a caller's arguments is reachable by
anything the caller writes — a separator, or a value-taking flag with its value
missing. Scenario 12 settles where it goes instead.

### 11. `--setting-sources` does not suppress the injected file — **pass**

Expected: unknown. `--setting-sources` restricts which of the user's own
settings files load, and the help text claims `--settings` still applies.

Command: `claude -p "…" --setting-sources user --settings <file>`.

Observed: both hooks fired. The CLI-supplied file is not one of the sources
that flag governs.

Consequence: it stays a pass-through argument. It is recorded because the test
that allows it should rest on a driven scenario rather than on a help string —
an allow-list resting on an assumption would bless the same silent-unmanaged
failure scenarios 8 and 10 exist to prevent.

### 12. The injection placed first survives everything after it — **pass, and it settled where it goes**

Expected: unknown. Scenarios 8 and 10 ruled out placing it last; this asks
whether placing it first is reachable from the other side.

Commands: `claude --settings <file> -p -- "…"`, and
`claude --settings <file> -p "…" --add-dir` (a value-taking flag with no
value).

Observed: with the injection first, a caller's `--` is harmless — both hooks
fired. A caller's trailing value-taking flag with no value fails the launch
outright (`error: option '--add-dir <directories...>' argument missing`) rather
than degrading it.

Consequence: Corral's `--settings` goes first, before anything the caller
passed. Nothing written after it can reach it, and the one thing position
cannot answer — a caller repeating the flag, where the last wins (scenario 8) —
is answered by refusing that flag. A caller mistake now fails loudly instead of
producing a session that looks managed and reports nothing.

## Relay interference, measured

Not a provider behaviour, but the number ADR 0004 D4 asks for evidence on. One
whole `corral hook-relay` invocation — process start, argument parsing, stdin
read, connect, framed delivery, and the acknowledgement — against a live
`corrald` on this machine:

```text
n=100   min 2.7 ms   p50 2.8 ms   p90 3.0 ms   p99 3.3 ms   max 3.3 ms
```

Against no daemon at all (the fail-open path, which is the common case when
Corral is not running): p50 2.9 ms, max 3.2 ms over 60 runs.

Measured after the relay stopped building an async runtime it never used. The
earlier figures — p50 3.0 ms, p99 3.7 ms — are what that construction cost,
and it sat outside the budget rather than inside it.

The 50 ms budget holds with an order of magnitude to spare. It is not asserted
per run: a deadline that tight on a loaded machine is a flake generator, and
the flake law owns that trade (`AGENTS.md` §Tests).

## Limitations of this record

- One machine, one OS, one install channel. A second platform is the
  automation follow-up's job, not this document's.
- `Notification` was exercised through the idle-at-prompt trigger only. Whether
  a permission prompt fires it was not driven.
- Scenario 6 was run once, in one direction (headless resuming an interactive
  session). Whether the reverse, or two interactive sessions, behaves the same
  was not driven; the refusal does not depend on the answer.
- Scenario 9 drove `/clear`. `/compact` was not driven; it is treated as the
  same normalized origin on the documented `source` value alone, and nothing
  decides on that value.
- Scenario 8 exercised two `--settings`. Three or more were not driven.
- Scenarios 10 and 12 drove one `--` on either side of Corral's flags, and one
  value-taking flag with its value missing. The space of caller mistakes is not
  enumerable; what was established is that the injection placed first is not
  reachable from after it.
- Every scenario used a project-local scratch directory. Behaviour with a
  project that has its own `.claude/settings.json` declaring the same hooks was
  not driven — ADR 0004 D6's additive claim rests on the flag's documented
  semantics plus scenario 1, not on a merge test. PR7 owns merge.
