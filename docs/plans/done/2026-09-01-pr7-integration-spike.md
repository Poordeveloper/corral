---
status: done   # reference: docs/references/2026-09-02-pr7-global-integration-spike.md; grill rounds 3–4 ruled over it, ADRs 0013/0014 accepted 2026-09-02
class: B
writes: [docs/references]
reads: [docs/adr/0013-global-hook-integration.md, docs/adr/0014-external-session-evidence.md, docs/adr/0004-hook-delivery.md, docs/adr/0009-codex-notify-delivery.md, docs/references/2026-08-27-pr5-claude-code-hook-matrix.md, docs/references/2026-08-31-codex-0.145.0-notify-spike.md, docs/plans/done/2026-09-01-pr7-external-sessions.md]
---

# PR7 spike — the facts ADR 0013 / ADR 0014 stand on

## Goal

Measure, first-party, every load-bearing fact the two proposed ADRs list,
so the grill accepts, amends, or rejects them on evidence rather than
documentation. Output: one dated reference,
`docs/references/<date>-pr7-global-integration-spike.md`, with the PR5
matrix fields per scenario — provider version, install channel, OS,
scenario, exact command, expected, observed, SHA of any fixture, date,
pass/fail — plus a captured-fixture directory for the corpus.

## Non-goals

No production code, no relay or daemon change, no merge engine. No ADR
acceptance pre-empted: a scenario that contradicts a proposed D-item is a
finding for the grill, never a silent edit to the ADR. No corpus entry
invented to fill a cell — an unmeasurable scenario is recorded as such.

## Method

S1/S2's bar: every claim comes from a run performed for this spike on
named versions. Providers under test: the installed Claude Code and Codex
CLIs at spike time (record exact versions and channels). One machine, macOS, by
founder direction (2026-09-01): no Linux container. The Linux
process-table and ancestry facts are a recorded limitation of this spike
— the `/proc`-side observation lands behind the platform boundary and is
measured when a Linux dev loop exists, before the PR7 matrix seals
recognition rules for that platform.

Instrumentation stands in for the relay: a script that appends its own
pid, ppid, full ancestor chain (pid, start time, executable path, argv[0]
per hop), environment provenance, and verbatim stdin/argv payload to a
log — the observation ADR 0014 D1/D2 needs, measured without touching
`corral`.

## Scenarios

**ADR 0013 — the file and its owners**

1. **Corpus.** Collect real `~/.claude/settings.json` and Codex
   `config.toml` shapes: this machine's own, plus public dotfiles
   (provenance recorded per file; secrets scrubbed before commit). Record
   per file: strict JSON or JSONC constructs, comments, key order,
   third-party hook layouts, `disableAllHooks` occurrences, `notify`
   occupancy. Seals D3's parser choice and D4's trigger list.
2. **Does Claude accept JSONC?** Feed a settings file with comments and
   trailing commas through a real launch; observe accept/reject. If
   rejected, real-world files cannot legally carry comments and D3's
   Claude parser may be strict JSON with format preservation.
3. **Provider rewrite behavior** (ROADMAP §9.6). Seed each file with
   foreign keys, a Corral-shaped hook entry, comments, and odd formatting;
   make the provider write its own file (Claude: change a setting via its
   own commands; Codex: answer a trust prompt, change a TUI setting); diff.
   Record: foreign entries preserved? comments? formatting? entry order?
4. **Layer semantics.** Hook entries in user vs project vs local settings:
   do layers add or replace for hooks? Where must a global entry live to
   observe sessions in any project? `disableAllHooks: true` at each layer
   against a user-level entry — which combinations silence it?
5. **Codex config-layer notify.** `notify` set in `config.toml` (no `-c`),
   interactive TUI session, complete a turn: does it fire? With what argv
   payload? ADR 0009's spike measured only the `-c` path.

**ADR 0014 — the process and its ancestry**

6. **Ancestry per host.** For each of: direct terminal, tmux, screen,
   `nohup`/`setsid`, a shell-script wrapper — run Claude (hook) and Codex
   (notify), fire an event, capture the instrumented chain. Question per
   run: does the chain reach a recognizable provider process while the
   hook still lives, and how many hops and what intermediaries (sh, node)
   sit between?
7. **Recognition shapes.** For each install channel available (npm global,
   native installer, homebrew): what the provider process shows in the
   macOS process table — executable path, argv, process name. The
   measured shapes become D2's recognition-rule inputs.
8. **Double-fire and order.** Global-shaped user-settings entry plus a
   `--settings` injected entry, one launch, one event: do both fire,
   in what order, with what timing gap? (Feeds ADR 0014 D4.)
9. **Platform identity APIs.** On macOS: obtain
   `(pid, start time, executable)` for a live process, a zombie, a
   just-exited pid, and a pid the user cannot inspect; record API used,
   failure mode, and whether start time disambiguates a reused pid.
10. **Sweep cost.** Full process enumeration with the design-9 fields,
    100 iterations on a loaded machine, p50/p95: the number a sweep
    cadence is derived from rather than guessed.

**ADR 0013 D8 — the stop check (grill Q6)**

11. **Missing command.** For each provider, configure the integration
    entry (Claude hook entry; Codex `notify`) with a command path that
    does not exist; run a real session and fire events. Record:
    user-visible warning/error, per-event/per-turn repetition, whether
    the agent continues, latency or blocking, exit-status interpretation,
    stdout/stderr behavior, and whether the provider disables or retries
    the integration. Silent/fail-open → the default-install shape stands;
    visible per-event disruption → **stop**, and the residual-failure
    shape is redesigned on this measurement before ADR 0013 can be
    accepted. No mechanism is preselected here.

## Definition of done

- All eleven scenarios recorded with the header fields, or recorded as
  honestly unmeasurable with the reason; corpus fixtures committed with
  provenance. Scenario 11's verdict is stated explicitly against ADR 0013
  D8's stop condition.
- Findings that contradict a proposed D-item are listed in their own
  section addressed to the grill.
- The reference lands with this plan moving to `done/`; the PR7 plan's
  design 1 then points at it and the grill is requested.

## Closed 2026-09-02

Reference: `docs/references/2026-09-02-pr7-global-integration-spike.md`.
Method deviation from the founder's 2026-09-01 macOS-only direction,
by founder direction 2026-09-02: session scenarios ran on Linux host `ne`
inside a udocker/PRoot container (the local machine was ruled out for
spike testing); macOS facts (scenarios 7, 9, 10) kept their Host A
measurements, and no ancestry conclusion above the provider was drawn
from the container. Scenario 11's verdict: the D8 stop condition fired
for Claude's bare command and is disarmed by the ruled guarded form;
Codex is silent. The public-corpus fixture half was reassigned by grill
Q7′ from this spike to the PR7 merge gate; macOS upper ancestry and the
Homebrew channel are post-merge matrix expansion. Grill rounds 3–4 ruled
over the record and accepted ADRs 0013/0014.
