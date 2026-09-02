# Provider noise catalog

Measured ways provider evidence can mislead attention derivation, and
what Corral does about each. The matrix says what evidence may mean
(`docs/references/2026-09-02-pr8-attention-matrix.md`); this file records
the confusions it measured. Tests cite ids; runtime code never parses this
file (ADR 0015 D9). A positive semantic rule never lives here — "the idle
prompt means Ready" is the matrix's — only the confusion beside it.

Dispositions: `unresolved` · `suppressed by adapter` · `excluded by
manifest negative` · `diagnostic-only` · `not semantic evidence`.

| Id | Provider · version · surface | Phenomenon | Risk if misread | Disposition | Evidence |
|---|---|---|---|---|---|
| `claude.subagent-stop.title-turn` | Claude Code 2.1.258, hooks | `SubagentStop` fires 2–6 s after every `Stop` whose turn generated a session title, with no subagent in the turn | Late "stop" evidence revives or re-arms a state after the real turn ended (Herdr's rollback cause) | `diagnostic-only` — `SubagentStop` asserts no state | matrix C1, C2, C7, C9, C10 |
| `claude.notification.idle-prompt` | Claude Code 2.1.258, hooks | `Notification` `idle_prompt` ("Claude is waiting for your input") fires 60 s after `Stop` at an idle prompt | Read as Needs You it is a false blocker on every idle session | `suppressed by adapter` — a Ready re-observation, never `AwaitingInput` | matrix C1, C7, C9, C10 |
| `claude.notification.permission-delay` | Claude Code 2.1.258, hooks | `Notification` `permission_prompt` fires 6 s after `PermissionRequest`, only while the request is still pending | Treated as the request it is 6 s late; treated as a second request it double-notifies | `suppressed by adapter` — confirms the standing item, never mints one | matrix C2, C3, C5 |
| `claude.background-task.turn` | Claude Code 2.1.258, hooks | A background task's completion arrives as a full `UserPromptSubmit` → `Stop` → `SubagentStop` trio with no keystroke | A Ready item and notification for a turn nobody submitted | `unresolved` — recorded; dogfood decides whether it is a true Ready | matrix C10 |
| `claude.reject.no-stop` | Claude Code 2.1.258, hooks | Esc on a permission or plan dialog produces no `Stop` and no other hook | Hook-only observers hold Needs You until fresher evidence or rot | `unresolved` — the fidelity limitation grill Q7 accepted; measured, not repaired | matrix C3, C5 |
| `claude.dialog.not-quiet` | Claude Code 2.1.258, screen | A permission dialog redraws continuously; the stream never goes quiet for 2 s while blocked | Activity read as Working overrides a visible blocker | `excluded by manifest negative` — activity yields to a Needs You claim (ADR 0015 D4) | matrix C2 |
| `claude.output.permission-words` | Claude Code 2.1.258, screen | Tool output and replies can contain "Do you want to proceed?", "1. Yes", "Allow Bash(…)?" verbatim, under the ordinary mode bar | A whole-screen substring rule fires on ordinary output | `excluded by manifest negative` — rules anchor on the dialog structure with the mode bar absent | matrix C9 |
| `claude.echo.activity` | Claude Code 2.1.258, PTY | Typing at the prompt echoes one output frame per key; a paste renders once as a placeholder | A person typing reads as an agent working | `diagnostic-only` — a false Working, never a false Needs You; input Corral wrote is discounted (plan A2) | matrix C8 |
| `claude.trust-dialog.pre-session` | Claude Code 2.1.258, screen | A fresh directory shows the trust dialog before `SessionStart`, with an empty title | A blocked prompt invisible to hooks and to identity | `unresolved` — screen-only; a candidate rule, unsealed | matrix C11 |
| `codex.title-thread.notify` | Codex 0.152.0, notify | Every user turn is followed by a second `agent-turn-complete` carrying a different `thread-id` (the title-generation thread), and it can arrive *before* the user turn completes — 9 s before an approval dialog in X2 | Ready asserted while the agent is blocked; a second identity minted per turn (ADR 0014 Q6′) | `suppressed by adapter` — a notify counts for the session only when its `thread-id` is the session's; the session's id is the one its rollout file carries | matrix X1, X2, X5, X6; PR7 spike |
| `codex.reject.no-notify` | Codex 0.152.0, notify | Esc on the approval dialog ends the turn with no `agent-turn-complete` | As `claude.reject.no-stop` | `unresolved` — measured limitation | matrix X3 |
| `codex.paste.literal` | Codex 0.152.0, screen | A bracketed paste is inserted literally into the composer, lines and all | Screen rules keyed on prompt-area text see user content | `diagnostic-only` | matrix X6 |
| `codex.title.blink` | Codex 0.152.0, OSC title | The blocked title alternates `[ . ]` and `[ ! ] Action Required | proj` once a second | A rule matching one spelling flaps | `excluded by manifest negative` — the rule matches `Action Required`, not the glyph | matrix X2, X3 |
| `codex.bel.unfocused` | Codex 0.152.0, PTY, `tui.notifications = true` | A bare BEL at approval time while the terminal is unfocused; no OSC 9/777 | A BEL is any program's; read as a request it is unattributable | `not semantic evidence` — the `Action Required` title carries the claim | matrix X4 |
| `pty.echo.person` | both, PTY | Output caused by the person's own keystrokes | False Working | `diagnostic-only` | matrix C8, X6 |
