# Claude Code hook payload fixtures

Real payloads, captured first-party, sanitized only where they carried a
developer's own paths and prompts: the structure is exactly what Claude Code
wrote. They exist so the payload parser is proven against the format rather
than against a shape a test author imagined (AGENTS.md §Tests).

`SessionStart-2.1.239.json` is the smaller payload S2 recorded on the earlier
version (`docs/references/2026-08-22-s2-session-identity-verification.md`).
Keeping it is the point: a parser that needs a field a supported version does
not send is a parser that breaks on the version matrix.

`Notification.json` and `Notification-permission.json` are the two halves of
one event name: the first carries `notification_type: "idle_prompt"` and the
message "Claude is waiting for your input", the second
`"permission_prompt"` and "Claude needs your permission". Only the second is
a request. Keeping both is what stops the parser from reading the event name
alone (matrix C1, C2; noise catalog `claude.notification.idle-prompt`).
`Notification-permission.json` is from 2.1.258, lifted verbatim from
`fixtures/screens/claude/2.1.258/c02_permission_approve/hooks.jsonl`.

Everything else is from 2.1.247, recorded in
`docs/references/2026-08-27-pr5-claude-code-hook-matrix.md`.
