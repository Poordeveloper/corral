# PR8 attention matrix: what the providers show, say, and write, measured first-party

> Compatibility evidence for PR8 (`ROADMAP.md` §3; plan Design 0). Every
> claim below is from a run performed for this record, not from
> documentation, memory, or an earlier matrix. It is evidence about
> **these versions** and seals only what it measured (grill Q13, Q28):
> Claude Code 2.1.258 and Codex 0.152.0, on this host, for exactly the
> capabilities each scenario exercised. It is the load-bearing evidence
> ADR 0015 and ADR 0016 wait on, and the acceptance reconciliation reads
> its closing section against grill Q32's conditions.

## Method

| | |
|---|---|
| Host | Linux `ne`, Ubuntu 24.04, x86_64, kernel 6.8.0-110 — inside the udocker 1.3.17 container of the PR7 spike (`node:22-bookworm`, PRoot engine), by the same founder direction: provider CLIs, configuration, and credentials never touch the host account |
| Claude Code | **2.1.258**, native installer at `/root/.local/bin/claude`; the founder's claude.ai subscription, signed in inside the container; model `haiku` for every run |
| Codex | **0.152.0**, npm global at `/usr/local/bin/codex`; the founder's ChatGPT account; `model_reasoning_effort = "low"` |
| Corral commit | `9c3c87b` (`main`, after PR7) — no Corral code ran; the captures are rendered afterwards by `cargo run -p corrald --example replay_capture` through the emulator the daemon owns |
| Date | 2026-09-02 |

The driver (`scripts/matrix/drive.py` with `hookcap.py`, uploaded to the
host) forks each provider on a real 120×40 PTY with a scrubbed
environment, records every output byte with a wall-clock timestamp
(`stream.bin`), every keystroke (`input.bin`), and named checkpoints
(`marks.jsonl`), and drives the keyboard from regexes over the raw stream
and from the provider's own events. Claude's hooks are declared in a
`--settings` file whose every event runs `hookcap.py`, which appends the
verbatim payload with a timestamp; Codex's `notify` points at the same
program. The container's global `settings.json` still carries Corral's PR7
relay entries, so every Claude event also invoked the token-less relay
with no daemon listening: silent, exit 0, as ADR 0004 requires.

The captures are committed under `crates/corrald/fixtures/screens/
<provider>/<version>/<scenario>/`; rendered screens are derived and are
quoted here, not committed. Timings in this record are seconds from the
scenario's `SessionStart` (Claude) or from the driver's start (Codex),
rounded as stated.

**What PRoot changes, and what it does not.** PRoot rewrites
`/proc/<pid>/exe` and re-parents everything above the provider process
(PR7 matrix); nothing here reads either. Screen bytes, hook and notify
payloads, timings between them, and the providers' own files are the
provider's, unaltered. Codex's bundled bubblewrap warns at startup and its
read-only sandbox still blocks the writes the approval scenarios need.
The Linux external-Know chain — recognition through projection — is not
measured here and remains under grill Q16's gate.

## Claude Code 2.1.258

Twelve scenarios; every one captured the raw PTY byte stream, the driver's
keystrokes, and the hook payloads (`hooks.jsonl`) from a `--settings` file
declaring `SessionStart`, `UserPromptSubmit`, `PreToolUse`, `PostToolUse`,
`Notification`, `Stop`, `SubagentStop`, `PreCompact`, `SessionEnd`, and
`PermissionRequest`. The settings file loaded without complaint, which is
itself the first result: **`PermissionRequest` is a hook event on 2.1.258.**
Model `haiku` throughout — the UI does not change with the model, and the
founder's Opus quota does; one consequence is recorded under C1.

### C1 — startup, one turn, idle prompt — **pass**

Hook sequence, seconds from `SessionStart`:

```text
SessionStart(startup)@0.0 → UserPromptSubmit@3.0 → Stop@4.4
  → SubagentStop@9.1 → Notification(idle_prompt)@64.5
  → SessionEnd(prompt_input_exit)@67.8
```

Observed:

- The OSC 0 title is a **state signal**: `✳ Claude Code` at the idle
  prompt; a spinner glyph (`◐ ◑ ◒ ◓`) replaces `✳` for as long as the agent
  runs (`◐ Claude Code` 0.05 s after `UserPromptSubmit`); the generated
  session title arrives mid-turn (`◐ Ok confirmation`); `✳` returns 0.03 s
  after `Stop`. The glyph says *running or not*; it does not distinguish
  idle from blocked (C2).
- Output after `Stop` ends within **31 ms** — the "Churned for 1s · done"
  line — and then the stream is silent: 0 bytes in the 57 s from Stop+3 s
  until the idle-prompt notification, which itself redraws nothing.
- **`SubagentStop` fires 4.7 s after `Stop` with no subagent in the
  turn.** The title-generation turn ends with it. Every scenario with a
  generated title shows the same trailing `SubagentStop` (C2 2.4 s, C7
  4.2 s, C9 6.2 s, C10 4.6 s after each `Stop`). Herdr's "late SubagentStop
  reviving idle panes" is this event; it is noise for turn state.
- `Notification` `idle_prompt` ("Claude is waiting for your input") fires
  **60.1 s after `Stop`** (C7 60.0 s, C9 60.1 s, C10 60.0 s) and means
  "still idle", never "blocked": a Ready re-observation.
- The bottom bar reads `⏸ manual mode on · ? for shortcuts · ← for agents`
  at the prompt and `⏸ manual mode on · esc to interrupt · ← for agents`
  while running; with `haiku` the bar also says "auto mode unavailable for
  this model" and the session runs in manual (ask) mode, which is what
  makes the permission scenarios below possible without a flag.
- `/rc connecting…` sits at the right of the bar from startup, and C4/C5
  render a startup banner: "Keep working from anywhere · Check progress or
  reply to any session from the mobile app, desktop app, or
  https://claude.ai/code/session_… · To keep a session in this terminal
  only, run /remote-control". 2.1.258 registers every interactive session
  with claude.ai remote control by default on this account. An S3 fact,
  recorded here and not interpreted.

### C2 — tool permission, approved — **pass**

```text
UserPromptSubmit@1.9 → PreToolUse(Bash)@3.8 → PermissionRequest(Bash)@3.9
  → Notification(permission_prompt)@9.9 → [Enter]
  → PostToolUse(Bash)@36.0 → Stop@39.8 → SubagentStop@42.2
```

- `PermissionRequest` fires **70 ms after `PreToolUse`**, with
  `tool_name`, `tool_input` (`command`, `description`), and
  `permission_suggestions` — an answerable request entity, delivered at
  the moment the dialog appears.
- `Notification` `permission_prompt` ("Claude needs your permission")
  fires **6.0 s after `PermissionRequest`** (C3 6.0 s, C5 6.0 s): a
  delayed "still waiting" signal, not the request itself.
- The screen while blocked:

```text
────────────────────────────────────────────────────
 Bash command
   ls -la /tmp
   List detailed contents of /tmp directory
 Do you want to proceed?
 ❯ 1. Yes
   2. Yes, allow reading from /tmp from this project
   3. No
 Esc to cancel · Tab to amend
```

  The mode bar is **absent** for the life of the dialog (the last row is
  the dialog's own `Esc to cancel · Tab to amend`), and the title shows the
  idle glyph (`✳ Tmp directory listing`).
- **The dialog is not quiet.** The driver waited for two seconds of
  silence and got none in 30 s: the blocked screen keeps redrawing. A
  Working-from-activity rule that did not yield to a visible blocker would
  report a blocked agent as working (ADR 0015 D4's "blocker beats
  activity" is load-bearing).
- After Enter: `PostToolUse` 0.2 s later, the title glyph back to a
  spinner, `Stop` 3.8 s after that, the ordinary Ready screen with the
  command output and `✻ Churned for 5s · done 1:04 PM`.

### C3 — tool permission, rejected with Esc — **pass, and no `Stop`**

```text
PreToolUse(Bash)@2.7 → PermissionRequest(Bash)@2.8
  → Notification(permission_prompt)@8.8 → [Esc]
  → (nothing) → SessionEnd@100.3
```

Esc dismisses the dialog with `⎿ Interrupted · What should Claude do
instead?` and the Ready-shaped footer `✻ Worked for 2s · done 12:51 PM`;
**no `Stop` and no other hook follows**. A hook-only observer holds Needs
You until fresher evidence or rot: exactly the fidelity limitation grill
Q7 recorded, measured.

### C4 — AskUserQuestion — **pass**

```text
PreToolUse(AskUserQuestion)@3.3 → PermissionRequest(AskUserQuestion)@3.4
  → [Enter] → PostToolUse(AskUserQuestion)@7.4 → Stop@8.5
```

The question is a `PermissionRequest` too, and its screen is a dialog of
the same family:

```text
 ☐ Color
Which do you prefer?
❯ 1. Red
     The color red
  2. Blue
     The color blue
  3. Type something.
────────────────────────────────────────────────────
  4. Chat about this
Enter to select · ↑/↓ to navigate · Esc to cancel
```

No `Notification` arrived before the answer at 4 s; the 6 s delay of C2
had not elapsed.

### C5 — plan-mode approval (`--permission-mode plan`) — **pass, and no `Stop` on Esc**

```text
UserPromptSubmit@3.2 → PreToolUse(Write)@7.1 → PostToolUse(Write)@7.2
  → PreToolUse(ToolSearch)@8.9 → PostToolUse(ToolSearch)@9.0
  → PreToolUse(ExitPlanMode)@10.6 → PermissionRequest(ExitPlanMode)@10.7
  → Notification(permission_prompt, "Claude Code needs your approval for the plan")@16.7
  → [Esc] → (nothing) → SessionEnd@143.6
```

`Write` and `ToolSearch` ran without a prompt in plan mode. The approval
dialog:

```text
 Ready to code?
 Here is Claude's plan:
╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌
 Context …  Plan …
╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌
 Claude has written up a plan and is ready to execute. Would you like to proceed?
 ❯ 1. Yes, auto-accept edits
   2. Yes, manually approve edits
   3. Tell Claude what to change
      shift+tab to approve with this feedback
```

Three `PermissionRequest` dialogs, three wordings: "Do you want to
proceed?", "Which do you prefer?", "Would you like to proceed?". The
stable structure is the `❯ 1.` option list under a horizontal rule with
the mode bar gone — not any one sentence.

### C6 — thinking turn — **pass**

28 output frames in the 2.8 s between `UserPromptSubmit` and `Stop`,
longest gap 225 ms; the status line `✽ Manifesting… (running stop hooks…
1/2 · 1s · ↓ 38 tokens · thinking)` is on screen at the instant the
`Stop` hook fires — the hook precedes the screen's Ready by up to a
second.

### C7 — silent long tool (`sleep 8`) — **pass, and no permission**

`sleep 8` ran with no `PermissionRequest`: `PreToolUse@2.1 →
PostToolUse@10.3`. During the 8.2 s the tool produced nothing, the screen
produced **101 frames, longest gap 226 ms** — `⎿ Running… (3s)` and the
`✽ Drizzling… (5s · ↓ 121 tokens)` timer redraw every few hundred
milliseconds. On a Corral-owned PTY, a silent tool is not a silent
screen; PTY activity holds Working through it.

### C8 — compaction, resume picker, help, resize, typing, paste — **pass**

- `/compact`: `PreCompact(manual)` fires; the answer here was "Not enough
  messages to compact" — compaction itself was not exercised.
- `/resume` opens a full-screen picker listing sessions with age, branch
  and size ("17 times 23 calculation · 2 minutes ago · master · 22.8KB");
  Esc returns with `⎿ Resume cancelled`.
- `?` replaces the mode bar with a two-column shortcut table (`! for
  shell mode … /keybindings to customize`); Esc restores it.
- Resize to 100×30 and back: two redraws, no hook, no title change.
- Typing 50 characters at 60 ms: **50 output frames** (one echo per
  key) — PTY activity that is a person, not an agent.
- A 720-byte bracketed paste: one frame, rendered as
  `❯ [Pasted text #1 +60 lines]` with the bar `paste again to expand ·
  Ctrl+Y to paste deleted text`.

### C9 — permission-like words as ordinary output — **pass**

`printf 'Do you want to proceed?\n1. Yes\n2. No\nAllow Bash(ls)?\n'` ran
without a prompt and its output sits on the Ready screen twice — in the
tool result and in the reply — with the ordinary mode bar beneath:

```text
  ⎿  Do you want to proceed?
     1. Yes
     2. No
     Allow Bash(ls)?
● Here's the full output verbatim: …
✻ Brewed for 4s · done 12:58 PM
────────────────────────────────────────────────
❯
────────────────────────────────────────────────
  ⏸ manual mode on · ? for shortcuts · ← for agents
```

A whole-screen substring rule on "Do you want to proceed?" fires here; a
rule anchored on the dialog structure — the `❯ 1.` list directly above
the dialog's own last row, with no mode bar — does not. The adversarial
fixture for the Needs You rule.

### C10 — subagent, background task — **pass, with two noise findings**

```text
UserPromptSubmit@0.4 → ToolSearch, TaskCreate, Bash, Bash (no prompts)
  → Stop@15.1 → SubagentStop@19.7
UserPromptSubmit@23.6 → PreToolUse(Bash)@25.7 → PostToolUse(Bash)@25.8
  → Stop@27.5 → SubagentStop@32.5
UserPromptSubmit@40.8 → Stop@42.3 → SubagentStop@46.8      ← nobody typed
  → Notification(idle_prompt)@102.3
```

- The model did not use the Task tool for the first prompt (TaskCreate +
  Bash instead), so `SubagentStop` here is again the title turn.
- **A background task's completion arrives as a full turn**: 13 s after
  the second `Stop`, with no keystroke, `UserPromptSubmit` → `Stop` →
  `SubagentStop` fire again (the `sleep 15` finishing and the agent
  reporting it). A hook-only observer sees Working for 1.5 s and a new
  Ready item; a person sees `✶ Channelling… (17s)` beside the task list.
  A new Ready item on a turn nobody submitted is a notification the
  noise catalog has to own.

### C11 — fresh directory trust dialog — **pass**

Before any hook fires, an untrusted directory shows:

```text
 Accessing workspace:
 /root/proj-fresh-pr8
 Quick safety check: Is this a project you created or one you trust? …
 Claude Code'll be able to read, edit, and execute files here.
 Security guide
 ❯ No, exit
   Yes, I trust this folder
 Enter to confirm · Esc to cancel
```

Title empty, no `SessionStart` yet. Blocked on the user, invisible to
hooks — a Needs You only the screen can see, and one that precedes the
session's own identity.

### C12 — external session, global hooks only — **not run**

The container's global `settings.json` carries Corral's PR7 relay entries,
so every scenario above also exercised the token-less relay path with no
daemon listening (fail-open, silent). A dedicated run adds nothing the
Linux entry gate does not already require.

## Codex 0.152.0

Eight scenarios with `notify` pointed at the capture program (payload as
the final argument, as ADR 0009 D2 measured) and `model_reasoning_effort =
"low"` from the container's `config.toml`. Two driver corrections cost
three reruns and are themselves facts about the surface: the composer
treats a burst of characters followed by Enter as a paste and inserts a
newline instead of submitting (Claude's does the same for a long prompt,
C2's first run), so prompts are typed at 25 ms per character; and the
header reads `model: loading` for the first 8–13 s, during which Enter
does nothing.

### X1 — startup, one turn, idle — **pass**

- Startup emits `ESC[>7u` (kitty keyboard flags 7), bracketed paste, and
  OSC 10/11 colour queries the emulator must answer. A bubblewrap warning
  heads the screen under PRoot (the sandbox binary is bundled but the
  host is not what it expects); the approval scenarios below still
  produced prompts.
- Turn: `› Reply with exactly: ok` → `• Working (1s • esc to interrupt)` →
  `• ok`; 169 output frames in the 4.7 s turn; `agent-turn-complete`
  0.1 s after the last content; **a second `agent-turn-complete` 4.0 s
  later with a different `thread-id`** — the title-generation turn, whose
  `input-messages` begins "Generate a concise, single-line task title" —
  and its spinner keeps the stream busy 4 s past the first notify.
- The OSC 0 title is the working directory's name, `proj`, with a
  braille spinner prefix (`⠸ proj`, `⠴ proj`, …) for as long as a turn
  runs, on the user's turn and on the title turn alike.
- Idle: 0 bytes between the end of the title turn and the driver's exit
  65 s later. No idle notification exists.

### X2 — command approval, approved — **pass**

`-a on-request -s read-only`, prompt: create a file in the workspace.

```text
submitted@11.6 → agent-turn-complete (title thread)@16.4
  → approval dialog visible@25.5 → [Enter]@57.5
  → agent-turn-complete (session thread)@71.2
```

- **The title-thread notify arrives 9 s before the approval dialog and
  55 s before the session's own turn completes.** Keyed on the event
  alone, an observer reads Ready while the agent is blocked. The two
  threads are told apart only by identity: the session's `thread-id`
  names the rollout file the session wrote at startup; the title thread
  has no file (see Session stores).
- The dialog:

```text
• Running wc -c … && command printf 'hello\n' > probe.txt && command cat probe.txt
  Would you like to run the following command?
  Environment: local
  Reason: May I create probe.txt in /root/proj with the requested contents using a shell command?
  $ wc -c … && command printf 'hello\n' > probe.txt && command cat probe.txt
› 1. Yes, proceed (y)
  2. Yes, and don't ask again for commands that start with `…` (p)
  3. No, and tell Codex what to do differently (esc)
  Press enter to confirm or esc to cancel
```

- **The title announces the block**: `[ . ] Action Required | proj` and
  `[ ! ] Action Required | proj` alternate once a second for the life of
  the dialog (25.49 → 57.53 s), so the blocked screen is never quiet
  either. On Enter the spinner returns at once and the plain `proj` at
  the turn's end. An in-band signal for Needs You on a Corral-owned PTY,
  and the only one Codex has.
- After approval the command ran unsandboxed and the reply
  `✔ You approved codex to run … this time` preceded the result.

### X3 — command approval, rejected with Esc — **pass, and no notify**

Blocked 28.2 → 60.3 s under the blinking title; Esc → `✗ You canceled the
request to run …` and `■ Conversation interrupted - tell the model what to
do differently.`; the title goes through one spinner frame (40 ms) to the
plain `proj`; **no `agent-turn-complete` follows.** Codex, like Claude
(C3), ends a rejected request without a turn-end event.

### X4 — `tui.notifications = true`, terminal unfocused — **pass: a bare BEL, nothing else**

`-c tui.notifications=true` loads without complaint. With the driver
sending focus-out (`ESC[O`) after submitting, the approval dialog raised
**one bare BEL (0x07)** at the moment it appeared (36.9 s) and one more at
a second approval (91.5 s); no OSC 9, OSC 777, or any other notification
escape appeared at any point, and none appeared focused (first run). The
`Action Required` title blinked throughout, unfocused as focused. A BEL
is not attributable — any program rings one — so the in-band signal that
seals for Codex approvals is the title, and the BEL is at most a
corroborating tick.

### X5 — "ask me a question" — **pass: there is no question surface**

Codex has no user-input tool. Asked to clarify first, it printed
`• Do you prefer red or blue?` as ordinary output and ended the turn
(`agent-turn-complete`, then the title thread's), sitting at the plain
prompt until the answer was typed as the next turn. A Codex "question" is
Ready, not Needs You; the approval dialog is the provider's one blocking
surface.

### X6 — help, resize, typing, paste, compaction — **partial**

- `/help` is not a command on 0.152.0 (`Unrecognized command '/help'.
  Type "/" for a list of supported commands.`); the popup that `/` opens
  was not captured.
- Resize 100×30 and back: redraws only.
- Typing at 60 ms per character echoes one frame per key; the composer
  does not clear on Ctrl-U once it holds several lines.
- A 60-line bracketed paste is inserted literally — no placeholder — and
  the following `/compact` was appended to it and submitted as one
  message, so compaction was not exercised. The model's reply ("I see the
  pasted lines ending with /compact/help. What would you like me to do
  with them?") is a Ready turn.

### X7 — approval-like words as ordinary output — **pass**

With approvals off and no sandbox (`-a never -s danger-full-access`),
`printf 'Allow command?\nApprove running ls?\n> Yes\n  No\n'` ran and its
text sits on the Ready screen as a bulleted reply — `• Allow command? /
Approve running ls? / > Yes / > No` — under the plain composer, with the
plain title and `agent-turn-complete` delivered. The dialog's own lines
(`Would you like to run the following command?`, `Press enter to confirm
or esc to cancel`) and the title are what a rule anchors on; the words a
person might type are not. The adversarial fixture for the Codex Needs
You rule.

### X8 — `codex resume` picker — **pass**

A full-screen list — `Resume a previous session · Type to search ·
Filter: [Cwd] All · Status: [Active] Archived · Sort: [Updated] Created`,
rows like `❯ 11s ago  Reply ok`, a footer `enter resume · ctrl+a archive ·
esc start new · ctrl+c quit · …` — with an empty title, before any
session exists. Nine sessions listed for `~/proj`, including two from
14 hours earlier.

## Second run, 2026-09-03: the scenarios the first run could not reach

Four gaps in the run above — Claude compaction and API error, Codex's `/`
popup and compaction — plus the version accident that came with them. The
driver and the environment are the same; only `DISABLE_AUTOUPDATER=1` was
added, for the reason the accident gives below.

**Version drift, and what it costs.** Claude Code updated itself between the
two runs and again *during* the first attempt at C14: that attempt drew
`Claude Code v2.1.258` and ended with `✔ Update installed · Restart to
update`, and the next scenario drew `v2.1.259`. The 2.1.258 binary is gone —
the installer keeps `2.1.252`, `2.1.257`, `2.1.259` and removed the one every
scenario above was measured on. So **C13–C15 are evidence about 2.1.259 and
nothing else**, the first run's C1–C12 remain evidence about 2.1.258, and
2.1.258 can no longer be re-measured here. The captures below were taken with
the updater disabled.

### C13 — compaction, actually exercised — **pass**

C8 reached `/compact` and was told "Not enough messages to compact"; six
turns first, and it compacts.

- `PreCompact` fires with `trigger: "manual"`, then the screen shows
  `✻ Compacting conversation…` under a spinner glyph.
- The **OSC title carries the working spinner throughout**: `◐`/`◑`
  alternating for about 8 s, then back to `✳`.
- **No `Stop` fires for the compaction.** Seven `UserPromptSubmit` and seven
  `Stop` across the whole capture — six turns plus the one after — and the
  compaction sits between them with neither.
- A second **`SessionStart` fires with `source: "compact"`, carrying the same
  `session_id`**. A mid-session `SessionStart` is a compaction marker, not a
  new session, and must not mint one or reset identity.
- After: `✻ Conversation compacted (ctrl+o for history)` above the prompt,
  and the next turn behaves normally.

### C14 — a turn the API refuses — **pass, and it looks Ready**

`ANTHROPIC_BASE_URL` pointed at a local server answering every request `500`,
so no account request was made.

```text
● API Error: 500 Internal server error. This is a server-side issue, usually temporary — try again in a moment.
  If it persists, check your inference gateway (127.0.0.1:8787).

✻ Cogitated for 2m 55s · done 3:59 AM
```

- The prompt bar and the mode bar come back exactly as they do after a
  successful turn: `⏸ manual mode on · ? for shortcuts · ← for agents`. **A
  failed turn's screen is a Ready screen with an error line in the
  transcript.**
- **No `Stop`, no `Notification`.** The whole capture is `SessionStart`,
  `UserPromptSubmit`, `SessionEnd`. A hook-driven state that entered Working
  on `UserPromptSubmit` has nothing to leave it with, and 30 s later the
  screen has not changed.
- The title stays `✳ Claude Code` — no spinner, no attention glyph.

### C15 — nothing answering at all — **pass, same shape**

`ANTHROPIC_BASE_URL` at a closed port. `● API Error: Connection refused — a
firewall or proxy may be blocking it (ConnectionRefused)`, the same Ready-
shaped screen, the same three hooks and no `Stop`.

### X9 — the popup `/` opens — **pass**

X6 could not capture it. `/` lists this version's whole command inventory;
`/com` filters to `/compact  summarize conversation to prevent hitting the
context limit`, which is how X10 knew the command exists.

The inventory contains **`/approve  approve one retry of a recent auto-review
denial`** — the word a naive approval rule would match, drawn by a popup
nobody is being asked to answer. It is in the noise catalog.

### X10 — Codex compaction — **pass**

X6's `/compact` was swallowed by a preceding paste; on its own line after
five turns it runs.

- During: `• Working (1s • esc to interrupt)` in the transcript and the `⠋`
  spinner in the OSC title, for about 3 s.
- After: `• Context compacted`, and the composer returns to
  `› Ask Codex to do anything`.
- **No `agent-turn-complete` notify for the compaction**: six notifies for six
  turns, none for the compaction between them.

### What the two compactions agree on

Neither provider treats compaction as a turn: no `Stop`, no
`agent-turn-complete`. While it runs, the only positive signal is the
provider's own spinner — in the OSC title on both, and in the transcript on
Codex. An engine that derived Working from turn events alone would read a
compacting session as idle, and one that derived it from PTY activity or the
title spinner reads it correctly.

## Session stores, as the providers left them

Measured on the container's own stores after the runs, file names and
sizes only — no content was read (ADR 0016 D1).

**Claude Code.** `~/.claude/projects/<encoded cwd>/` holds one
`<uuid>.jsonl` per session — ten after these runs, including the one a
headless `claude -p` probe left, which is a resumable session like any
other — and a `memory/` directory. No per-session directories and no
`agent-*.jsonl` files appeared on this version. The encoded directory
name for `/root/proj` is `-root-proj`: separators and literal dashes share
one character, so the name is not reversible (grill Q25). The file's
modification time advanced with each turn.

**Codex.** `~/.codex/sessions/YYYY/MM/DD/rollout-<timestamp>-<uuid>.jsonl`
carries the **session's own `thread-id` in its name** — the id that its
`agent-turn-complete` reports — and one file per session was written at
startup; **the title-generation thread wrote no file** (three files, three
session threads, six title threads without one). That is the discriminator
PR7 lacked: a `thread-id` with a rollout file is the user's session; one
without is internal. Beside the sessions directory, 0.152.0 keeps
`session_index.jsonl` — an append-only index whose records carry exactly
`id`, `thread_name`, `updated_at`, several per id — plus
`thread_history_1.sqlite` and `history.jsonl`, and a `migrate-rollouts`
subcommand that calls the rollout files "legacy local sessions". The index
is the provider's explicit, separate metadata surface grill Q9/Q25 said
could be ruled on separately; nothing here reads it.

## Provider version, where it can be read

Neither provider's events carry a version. Measured shapes (grill Q12):

| Channel | Where the version is | Cost |
|---|---|---|
| Claude native installer (this host, macOS local channel) | `<install>/node_modules/@anthropic-ai/claude-code/package.json` `"version"`; the macOS local channel has the same file under `~/.claude/local/` | file read |
| Claude versioned channel | `…/claude/versions/<version>/` in the executable's path (the shape `provider::recognition` already seals) | none |
| Codex npm | `<prefix>/lib/node_modules/@openai/codex/package.json` beside `bin/codex.js` | file read |
| `claude --version` | `2.1.258 (Claude Code)` | 10–20 ms (macOS local channel) |
| `codex --version` | `codex-cli 0.152.0` | 60 ms warm, 550 ms cold |

Version file metadata is not bound to a running process by itself: the
installed Claude on the founder's macOS moved from 2.1.252 to 2.1.258
between PR7 and this record without any process restarting.

## What this record seals, and what it leaves open

Sealed for Claude Code 2.1.258 and Codex 0.152.0, exactly as measured:

- **Needs You surfaces.** Claude: tool permission, AskUserQuestion, and
  ExitPlanMode are each a `PermissionRequest` hook (70–100 ms after
  `PreToolUse`) and a dialog with the `❯ 1.` option list under a rule and
  the mode bar absent; the fresh-directory trust dialog is screen-only.
  Codex: the command approval is a dialog with `Would you like to run the
  following command?` and a title that blinks `Action Required`; there is
  no question surface; `tui.notifications` adds only a bare BEL, unfocused.
- **Ready surfaces.** Claude: `Stop`, the `✳` title glyph, the
  `✻ … · done HH:MM AM` footer, the mode bar's `? for shortcuts`.
  Codex: `agent-turn-complete` with the session's `thread-id`, the plain
  title, the `› Ask Codex to do anything` composer placeholder.
- **Working surfaces.** Claude: the spinner title glyph and the mode
  bar's `esc to interrupt`; Codex: the braille title prefix and
  `• Working (Ns • esc to interrupt)`. Screen Working stays diagnostic
  (grill Q14).
- **Activity.** Both providers redraw continuously while running — a
  silent tool included — and redraw while blocked; both are silent at
  idle. Claude's post-`Stop` redraw ends within 31 ms.
- **Noise.** Recorded in `docs/references/provider-noise-catalog.md` with
  the fixture that shows each.

Not sealed, and why:

- Claude's API error / retry state and compaction (`/compact` declined
  with "Not enough messages"); Codex's `/` command popup and compaction.
  Not induced; a later run adds them.
- Anything above the provider process, anything about `/proc`, and the
  Linux external-Know chain (PRoot; grill Q16).
- Claude 2.1.252 and Codex 0.145.0 (the founder's macOS installs) for
  attention semantics: not run here; nothing is inherited (grill Q28).

## Evidence map for the acceptance reconciliation (grill Q32)

| Load-bearing fact | Status |
|---|---|
| ADR 0015: Claude `Notification` types, blocked vs idle | measured: `permission_prompt` (6 s after the request), `idle_prompt` (60 s after `Stop`) |
| ADR 0015: `PermissionRequest` exists, payload | measured: yes; `tool_name`, `tool_input`, `permission_suggestions` |
| ADR 0015: screen shapes for the Needs You / Ready / Working / negative inventory | measured for every scenario, compaction and API error included (second run, 2.1.259) |
| ADR 0015: Codex approval announcement, in-band | measured: title `Action Required` (focused or not); `tui.notifications` adds a bare BEL when unfocused, no OSC 9/777 |
| ADR 0015: redraw-after-turn-end window | measured: 31 ms (Claude); 4 s of title-thread spinner (Codex) |
| ADR 0015: late lifecycle events after `Stop` | measured: `SubagentStop` 2–6 s after every titled turn; background-task turns |
| ADR 0016: Claude store layout, sub-agent files, mtime on resume | measured layout; no sub-agent files on this version; mtime advances per turn; resume-touches-mtime not isolated |
| ADR 0016: Codex thread id in the file name; headless vs interactive | measured: id in the name, session threads only; headless rollouts not exercised here (`codex exec` was not run) |
| ADR 0016: `--resume` from another directory | not measured |
| ADR 0016: enumeration cost on a large store | not measured (small stores) |
| Grill Q12: version metadata shapes per channel | measured, table above |
