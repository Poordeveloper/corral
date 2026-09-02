# Provider configuration shapes the merge engine is proven against

Real user configuration, so that the one code path that writes into files
Corral does not own is exercised against what people actually have rather
than against shapes a test author imagined (AGENTS.md §Tests; the PR7 merge
gate, grill Q7′).

Provenance and sanitization are per file below. Two rules held throughout.
Nothing here was invented: every file is either captured first-party on a
named machine and version, or taken from a public repository named below.
And nothing here widens what Corral owns — a file the engine cannot merge
into safely is preserved and refused honestly, never normalized until it
becomes editable.

| File | Where it came from | What it exercises |
|---|---|---|
| `claude-third-party-hooks.json` | `github.com/nekorush14/dotfiles`, `configs/claude/settings.json`, fetched 2026-09-02. Public repository. Sanitized: none needed — no secrets, and its paths are already `$HOME`-relative. | Third-party hooks on three events, two of them carrying a `matcher`; a large `permissions` allow/deny list; `env`; `statusLine`; `enabledPlugins`; and eight unrelated scalar keys. No Corral slot present. |
| `claude-hooks-disabled.json` | Written for this corpus from the 2026-09-02 spike's measurement, which is the source of the fact rather than of the bytes. | `disableAllHooks: true` beside the user's own settings: the D4 trigger Corral must refuse to override, at the one layer Corral writes. |
| `claude-not-json.json` | Written for this corpus from the same measurement. | A settings file carrying `//` comments. Measured: Claude Code 2.1.252 rejects it as `Invalid or malformed JSON` and silently drops *every* setting in it — so Corral must refuse to write into one, and must never produce one. |
| `codex-user-config.toml` | Captured first-party on the 2026-09-02 spike container (codex-cli 0.152.0), plus the comment and key layout a person keeps. Sanitized: the project path is a placeholder. | The user's own comments, unrelated keys, and the `[projects."…"] trust_level` entries Codex appends to this same file behind the user's back — corroborated independently by `openai/codex` issues #15433, #11061 and #5160. No `notify`. |
| `codex-notifier-occupied.toml` | Written for this corpus from the spike's measurement. | Codex's single notifier slot already holding somebody else's program. Corral preserves it, degrades, and explains; it never takes the slot to obtain awareness (ADR 0013 D7). |
| `codex-notify-ill-typed.toml` | Written for this corpus from the spike's measurement. | `notify` as a bare string. Measured: codex-cli 0.152.0 refuses to start on this file with a line and column, so it is the user's to fix and never something Corral quietly normalizes into an array. |

The two Claude files marked "written for this corpus" carry shapes the spike
measured rather than bytes it captured: what makes them evidence is the
recorded provider behavior in
`docs/references/2026-09-02-pr7-global-integration-spike.md`, not their
authorship.

Still open, and named so the gap is not mistaken for coverage: this corpus
has one public third-party Claude file and no public third-party Codex file.
A second and third of each — different hook layouts, a `[mcp_servers]` tree,
a profile — is the corpus work the PR7 matrix finishes.
