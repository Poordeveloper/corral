# Codex notify injection and identity, re-verified first-party

> Compatibility evidence for PR6 (`ROADMAP.md` §3; plan design 9). Every claim
> below is from a run performed for this record on this machine, not from
> documentation, from memory, from the acceptance spike, or from S2. It is
> evidence about **this version**: `PRODUCT.md` §10's supported
> provider/version matrix gains its Codex row here, and the follow-up that
> automates it under `verify-release` is named in the PR5 plan and still open.
>
> It re-runs the acceptance spike's scenarios
> (`docs/references/2026-08-31-codex-0.145.0-notify-spike.md`) against the
> installed release and adds what the implementation turned out to need
> sealed: the caller spellings that can displace the override, identity across
> an interactive resume, the in-place new thread, the zero-turn session, and
> whether Codex waits for its notify program.

## Method

| | |
|---|---|
| codex-cli | **0.145.0** (`codex --version`), installed at `~/.local/node/bin/codex` (npm global) |
| OS | macOS, Darwin 25.5.0, arm64 |
| Corral commit | `8b570b4` (branch `task/pr6-codex-managed-sessions`, before the implementation landed) |
| Date | 2026-08-31 |

Every scenario ran under a scratch `CODEX_HOME` (auth copied, mode 0600,
`model_reasoning_effort = "low"`, deleted afterwards) so that a user-level
`notify`, a profile, and the directory-trust answer could all be exercised
without touching `~/.codex`. The scratch home was confirmed in effect before
each interactive run — its model effort is visible in the TUI header. The real
`~/.codex/config.toml` was checked afterwards and holds no `notify` key.

Interactive scenarios were driven in a tmux pane by a scripted keyboard. The
capture program records its full argv, a bounded stdin probe, its cwd, and the
time; its own first argument is its output path, so anything Codex appends
follows it.

The captured payloads are committed as
`crates/corrald/fixtures/codex-notify/`, sanitized only where they carried a
developer's own paths; the structure is exactly what Codex wrote.

## Scenarios

### 1. The interactive TUI fires top-level `notify` — **pass**

Command: `codex -c 'notify=["<capture>","<out>"]'`, one completed turn.

Expected: the notify program is invoked on turn completion, from the
interactive client.

Observed: invoked, with `client: "codex-tui"`. This is the managed surface —
`codex exec` is not (ADR 0009 D1) — and S2 had only ever proven `exec`.

### 2. The payload is the final argv item, and argv only — **pass**

Observed: exactly one argument appended after the configured ones
(`argc_after_out=1`), holding the notification JSON verbatim; the stdin probe
read zero bytes. Payload (this run, before sanitizing):

```json
{"type":"agent-turn-complete",
 "thread-id":"01a057b2-7f33-7f11-9bbc-16e2fa444534",
 "turn-id":"01a057b2-bb99-7e22-b379-ded6e470f8da",
 "cwd":"<run dir>","client":"codex-tui",
 "input-messages":["Reply with exactly: ok"],
 "last-assistant-message":"ok"}
```

The same shape from `codex exec` was captured in the same session
(`client: "codex_exec"`, thread-id `01a057a3-f2f8-76f2-b505-5225da397872`,
`argc_after_out=1`, stdin empty) and is committed beside it: the parser must
not depend on `client` holding one value.

This is what `RELAY_PAYLOAD_ARGV_FLAG` exists for. A relay left to discover the
channel would wait out its interference budget on a pipe nobody opens.

### 3. The runtime `-c notify` beats a configured `notify` — **pass**

Command: scratch `CODEX_HOME` whose `config.toml` sets `notify` to a user
capture, launched with two `-c notify=[…]` flags (a decoy and Corral's), one
completed turn.

Observed: only the **last** `-c` program fired. The configured program stayed
silent, and so did the decoy. One run settles both halves of ADR 0009's
override strategy: the runtime layer wins (D4), and a caller's later flag
displaces an earlier one (D5's refusal rationale).

### 4. A `--profile` that configures `notify` does not displace it — **pass**

Command: `$CODEX_HOME/work.config.toml` sets `notify` to a profile capture;
launched with `--profile work -c 'notify=[…]'`, one completed turn.

Observed: the `-c` program fired; the profile's stayed silent. `--profile` is
therefore **not** a spelling Corral has to refuse — which is the question that
made this scenario worth running, since the spike had only measured the user
layer.

### 5. Every refused spelling is a real config-override spelling — **pass**

Five spellings, each carrying `model=corral-bogus-model` so the override is
observable for free — Codex names the model in a warning before any request is
made, so this costs no completed turn:

```text
-c model=…            accepted
--config model=…      accepted
-cmodel=…             accepted
-c=model=…            accepted
--config=model=…      accepted
```

All five take effect. `refuse_arguments` refuses `notify` in each of them plus
the dotted form; none of the five is a defensive guess, and scenario 3 is why
refusing matters — the last override wins, so a caller's later one silently
displaces Corral's.

### 6. Identity is stable across an interactive `codex resume <id>` — **pass**

Command: `codex -c 'notify=[…]' resume 01a057b2-7f33-7f11-9bbc-16e2fa444534`,
one completed turn.

Observed: the prior conversation was restored on screen, and the notify payload
carried the **same** `thread-id`. S2 had verified this for `codex exec resume`;
the interactive path is the one `session.resume` composes, and it is now
first-party.

On exit the TUI printed, unprompted:

```text
To continue this session, run codex resume 01a057b4-0d96-75e2-9735-77cc4f3fa119
```

— the resume verb Corral composes, stated by the provider itself.

### 7. An in-place new thread reports a different id — **pass**

Command: in the resumed session above, `/new` ("start a new chat during a
conversation"), then one completed turn.

Observed: the next notify carried `thread-id`
`01a057b4-0d96-75e2-9735-77cc4f3fa119` — a different conversation over the same
launch and the same token. That is ADR 0004 D8's contest, unchanged, now
observed on a second provider: Corral withdraws the identity claim once,
durably, and refuses `session.resume` while Open and attach stay untouched.

### 8. A session that completes no turn reports nothing — **pass**

Command: launch, then quit without submitting a prompt.

Observed: the notify program was never invoked; no output file was created at
all. A managed Codex session that exits before any turn completes therefore
never binds, and `session.resume` answers `IdentityUnknown` — Corral knowing
only that it lacks sufficient identity, asserting nothing about what Codex left
behind (ADR 0009 D3, grill Q3).

### 9. Codex does not block its interactive loop on notify — **characterized**

Command: notify program that sleeps 5 s; one completed turn; keystrokes sent
while it slept.

Observed: the assistant's answer was already rendered and the composer accepted
new input during the sleep. Codex does not wait for the notifier before
continuing.

Characterization, not a licence. The relay's 50 ms budget and its poverty
contract stay exactly as they are: two relay contracts is a drift trap, the
strictest consumer sets the bar, and the measurement above is about **this**
version of one provider (ADR 0009 D2).

### 10. Configuration residue — **pass, with the same attributed exception**

Observed: after the runs above, the scratch `config.toml` held no `notify` key
and no other override residue. Its bytes changed exactly once, at the moment a
person answered Codex's own first-run directory-trust prompt, which Codex
persisted as `[projects."…"] trust_level = "trusted"`. Every later run in the
already-trusted directory left the file byte-identical — including the run that
declined the TUI's own update offer (0.145.0 → 0.151.0).

The trust prompt itself appeared in the pane and was answered there: under
Corral it will appear the same way, through the PTY, and answering it stays the
user's act and Codex's write.

### 11. An oversize payload in argv — **pass on Corral's side**

Command: the real `corral hook-relay --provider codex --token … --payload-argv`
with a 300 KiB payload, past the 256 KiB channel cap.

Observed: exit 0, nothing on stdout or stderr, returned in 14 ms. `ARG_MAX` on
this machine is 1 MiB, so a payload of that size reaches the relay as an
argument at all. The cap itself is applied provider-neutrally in
`corral_protocol::hook` and is covered by its own tests; what this adds is that
the argv path does not fail differently at size.

## Limits

- **This is 0.145.0.** 0.151.0 exists and was declined in-session; the record
  is about the version that is installed. A version outside it is not gated —
  launch proceeds, evidence is best-effort, and unknown notify types assert
  nothing.
- Scenarios 3, 4, and 5 were driven through `codex exec`. They are questions
  about the CLI's argument parsing and config layering, not about the managed
  surface, and exec answers them for a fraction of the cost. The surface-shaped
  scenarios — 1, 2, 6, 7, 8, 9, 10 — were all driven interactively.
- `--remote <ADDR>` (experimental) was not driven: it connects the TUI to a
  remote app server, and whether the override reaches wherever the turn
  actually runs is unmeasured. It is **not** refused. A managed launch that
  learns nothing degrades to an identity that never binds, which is scenario
  8's honest outcome rather than a false one.
- A caller passing a subcommand — `corral new codex -- exec …` — is not
  refused either. S2 and scenario 2 both show `exec` firing notify, so it does
  not defeat the injection; what it does is put a surface outside M1's managed
  scope under a Corral PTY. Named as a follow-up, not decided here.
- Codex 0.145.0 has a `hooks` feature of its own (`--dangerously-bypass-hook-trust`
  in `--help`). It was not examined: ADR 0009 decided the channel is `notify`,
  and re-opening that is a decision, not a matrix scenario.
- Organization- or enterprise-managed configuration was not driven and cannot
  be on this machine.
- Oversize payloads were not generated *by Codex*; producing a 256 KiB
  assistant message costs real model output for a property that is Corral's,
  not the provider's.
