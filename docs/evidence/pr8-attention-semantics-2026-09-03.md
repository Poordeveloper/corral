# PR8 — the attention semantics this build seals

> Required by ADR 0015 before it may be accepted, and named by every
> `sealed_by` in `crates/corrald/manifests/` and by
> `corrald::attention::sealing`. Sealing is a human act on evidence: this
> record is the evidence, and the merge that lands it is the act (ADR 0015
> D9). Narrative and every screen is in
> `docs/references/2026-09-02-pr8-attention-matrix.md`; the captures are in
> `crates/corrald/fixtures/screens/`.

## What was measured, and on exactly what

| | |
|---|---|
| First run | 2026-09-02, ~12:30–13:30 +08 |
| Second run | 2026-09-03, ~03:45–04:05 +08 |
| Host | `ne`, Ubuntu 24.04 x86-64, bare metal |
| Container | udocker 1.3.17, PRoot engine, image `node:22-bookworm` |
| Claude Code | **2.1.258** (first run), **2.1.259** (second) |
| Codex | **0.152.0** (both runs) |
| Driver | `scripts/matrix/drive.py`, a real PTY, framed byte stream plus hook/notify capture |
| Screens | replayed through `qwertty-term-vt`, the emulator `corrald` itself uses |

Claude Code replaced itself between the runs and **deleted the 2.1.258
binary**; the installer keeps `2.1.252`, `2.1.257`, `2.1.259`. Nothing on
2.1.258 can be re-measured here. The second run's driver sets
`DISABLE_AUTOUPDATER=1`.

Sealing follows exactly that split, one row per version per fact. No range,
no "same minor", no inheritance: an unmeasured version is Limited awareness
(grill Q13, Q28).

## The hook and notify rows sealed here

`corrald::attention::sealing::sealed`.

| Provider · version | Fact | Provider event | Asserts | Captures |
|---|---|---|---|---|
| Claude Code 2.1.258 | `TurnStarted` | `UserPromptSubmit` | Working | C1, C2, C6, C7, C9, C10 |
| Claude Code 2.1.258 | `TurnEnded` | `Stop`, and `Notification(idle_prompt)` | Ready | C1, C6, C7, C9, C10 |
| Claude Code 2.1.258 | `AwaitingInput` | `Notification(permission_prompt)` | Needs You | C2, C3, C5 |
| Codex 0.152.0 | `TurnEnded` | `agent-turn-complete` | Ready | X1, X5, X7, X10 |

Sealed for nothing, deliberately:

- **Claude Code 2.1.259** — the second run measured compaction and failure on
  it, not the turn events. Its hook facts are observed and assert nothing.
- **Codex `TurnStarted` and `AwaitingInput`** — Codex has no turn-start
  notify, and announces an approval on the screen and in the OSC title rather
  than out of band. An adapter that later invents either does not inherit the
  row above.
- **Every other version**, including Claude Code 2.1.252 and Codex 0.145.0,
  which are the founder's own macOS installs and were never exercised.

### The `Notification` split, which the row above depends on

`Notification` is one event name carrying two facts, and the adapter reads
`notification_type`:

- `permission_prompt` — fires ~6 s after a `PermissionRequest` that is still
  pending. A request. Confirms a standing item; never mints one.
- `idle_prompt` — fires ~60 s after `Stop` at a prompt nobody has typed at,
  with the message "Claude is waiting for your input". **Not** a request: it
  re-observes a Ready prompt. Read as a blocker it would put every session a
  person walked away from into Needs You a minute later.
- Anything else, or absent — `UnknownEvent`: tolerated, counted, asserting
  nothing, so a later release cannot mint or clear a blocker on a variant
  nobody measured.

Both payloads are fixtures: `fixtures/claude-hooks/Notification.json` is the
idle one as captured, `Notification-permission.json` the request, lifted from
the C2 capture.

## The screen rules sealed here

Both manifests carry `sealed_versions`, and a rule asserts only where its own
`sealed_by` **and** the running version agree. The version is bound at the
launch boundary from the installation the program resolved to, and only when
that metadata predates the process (grill Q12).

| Manifest · rule | Asserts | What it anchors on | Captures, positive and negative |
|---|---|---|---|
| claude `permission-dialog` | Needs You | the `❯ 1.` option list and `Esc to cancel`, with the mode bar **absent** | C2, C3; negative C9, where the same words sit in ordinary output under the mode bar |
| claude `plan-approval` | Needs You | `Would you like to proceed?` with `❯ 1.`, mode bar absent | C5 |
| claude `idle-prompt` | Ready | `? for shortcuts` without `esc to interrupt` | C1, C7, C9, C10 |
| codex `action-required-title` | Needs You | the words `Action Required` in the OSC title, not the glyph | X2, X3, X4; negatives X7 and X9 |
| codex `approval-dialog` | Needs You | the command prompt's two fixed lines | X2, X3 |
| codex `composer-idle` | Ready | `Ask Codex to do anything` without `esc to interrupt` | X1, X5, X7, X10 |

Unsealed on purpose:

- **claude `trust-dialog`** — screen-only, drawn before `SessionStart`, and
  the catalog holds it `unresolved` (`claude.trust-dialog.pre-session`).
- **`running-bar` / `working-line`** — Working from a screen is diagnostic by
  decision; PTY activity and hook facts carry Working (grill Q14).

## The negatives, which are half of what sealing means

Every one is in `docs/references/provider-noise-catalog.md` with an id, a
disposition, and the captures that hold it. The ones that shaped a rule:

- Permission vocabulary appears verbatim in ordinary output (C9) and in
  Codex's `/` popup, which lists `/approve` (X9). Both rules anchor on
  structure the popup and the transcript do not have.
- Codex's title blinks between two glyphs while blocked (X2, X3); the rule
  matches the words.
- A `SubagentStop` follows most turns with no subagent in them (C1, C2, C7,
  C9, C10) and asserts no state.
- Codex emits a second `agent-turn-complete` per turn from its title thread,
  sometimes *before* the user's turn ends (X2); a notify counts only when its
  `thread-id` is the session's.
- Esc on a dialog produces no `Stop` and no notify (C3, X3) — a measured
  fidelity limitation, not repaired.

## What compaction and failure established, and what they did not seal

Measured on Claude Code 2.1.259 and Codex 0.152.0 (second run, C13–C15, X10):

- Neither provider treats compaction as a turn: no `Stop`, no
  `agent-turn-complete`. While it runs the only positive signal is the
  provider's own spinner — in the OSC title on both, in the transcript on
  Codex. Working through a compaction therefore rests on PTY activity, which
  needs no version row, and not on turn events.
- Claude fires a second `SessionStart` with `source: "compact"` carrying the
  **same** `session_id`. A mid-session `SessionStart` is a compaction marker,
  never a session boundary and never a reason to mint identity.
- A turn the API refuses leaves a **Ready-shaped screen** — the error line in
  the transcript, the ordinary prompt and mode bar beneath it — and fires
  **no `Stop` and no `Notification` at all**. A hook-only observer never
  leaves the Working it entered on `UserPromptSubmit`.

The last one is recorded `unresolved` in the catalog
(`claude.api-error.ready-shaped`) and seals **nothing**. It is a question for
ADR 0015 D4's composition — a screen asserting Ready against a hook Working
that nothing will ever close — and this build answers it only through the
ordinary rules: the hook claim rots on its horizon, and on 2.1.259 the screen
rule is unsealed and asserts nothing either, so such a session reads Unknown.
Making it read Ready would need a sealed rule on a measured version, which is
a later change with its own evidence.

## Where a person can check this

- Captures: `crates/corrald/fixtures/screens/<provider>/<version>/<scenario>/`
- Replay: `cargo run -p corrald --example replay_capture -- <scenario dir>`
- Narrative, per scenario: `docs/references/2026-09-02-pr8-attention-matrix.md`
- Ways evidence misleads: `docs/references/provider-noise-catalog.md`
- Store layout and resume location: `docs/evidence/pr8b-history-store-and-resume-2026-09-02.md`
