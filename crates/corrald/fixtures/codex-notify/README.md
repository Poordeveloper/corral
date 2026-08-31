# Codex notify payload fixtures

Real payloads, captured first-party on codex-cli 0.145.0, sanitized only where
they carried a developer's own paths: the structure is exactly what Codex
appended to its notify program's argv. They exist so the payload parser is
proven against the format rather than against a shape a test author imagined
(AGENTS.md §Tests).

Both files are the same notification from the two clients that produce it, and
keeping both is the point. `agent-turn-complete-tui.json` is the interactive
TUI — the whole managed surface (ADR 0009 D1) — and
`agent-turn-complete-exec.json` is `codex exec`, which is out of managed scope
and is here because it is the same event family from a different client: a
parser that needed `client` to hold one value would be reading a field it has
no business deciding on.

Recorded in `docs/references/2026-08-31-pr6-codex-notify-matrix.md`, on top of
the acceptance spike
(`docs/references/2026-08-31-codex-0.145.0-notify-spike.md`) and S2's earlier
`codex exec` evidence
(`docs/references/2026-08-22-s2-session-identity-verification.md`).
