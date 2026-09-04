---
status: done   # merged in PR #38
class:  C
writes:
  - crates/corral-state (durable schema, RunStarted event, Run)
  - crates/corrald/src/managed_launch (resume eligibility)
reads:
  - crates/corrald/src/runtime (the handle the directory came from)
---

## Goal

A Session Corral launched can be continued by the next daemon.

Corral records where it started every Run it started, durably. Today that
fact lives only in the launching daemon's runtime handle, so a restart
loses it and `resume_plan` refuses:

```text
session_not_continuable — this session was not started by the running
Corral daemon, so Corral does not know where it ran and will not continue
it somewhere else
```

The refusal is honest about what the daemon knows; the defect is that it
had no reason not to know. ADR 0016 D4 says `last Run ended Exited (any
origin) → eligible`, and a history-continued Session drops out of that the
moment the daemon that continued it goes away — so continuing a row once
makes it *less* continuable than it was before, which is the disappearance
D2 forbids seen from the control side rather than the list side.

D5 says the opposite of what the code can deliver:

> A Session Corral launched keeps the working directory Corral recorded
> for it

"Recorded" is a runtime handle, not a record. This makes the sentence
true.

## Non-goals

- Guessing a directory for a Run whose own is unknown. A provider resolves
  which of its sessions an id names by where it is started (Q35); an
  ambient substitute is the silent fallback that rule exists to forbid.
- Reading a discovered process's cwd from the OS. That is platform work
  for a later phase; here such a Run records no directory and says so.
- Changing what a client may ask for. `session.resume`'s `working_directory`
  still governs the history rung alone.
- The identity question when a continuation's provider mints a different
  id — recorded as open in ADR 0016 §What this does not decide.

## Existing owner / architecture involved

`corral-state` owns durable Corral facts. A Run's start is
`SessionEvent::RunStarted`, produced by four store paths — three Corral
launches (`start_managed_session`, `resume_managed_session`,
`continue_history_session`) and the discovery path (`record_run_started` /
`record_withheld_run_started`, both through `start_run`).

`managed_launch::resume_plan` is the one consumer: it asks
`runtime.sessions` for the directory and refuses `NotThisDaemon` when the
handle is absent.

## Design

**The directory belongs to the Run, not the Session.** A Session's Runs
may each run somewhere different — a history continuation runs where the
client said — so a Session-level field would be a single answer to a
per-episode question.

`RunStarted` gains `working_directory: Option<PathBuf>`, beside
`started_at: Option<SystemTime>` and for the same reason: absent means the
fact is unknown, never that it is empty or that a default applies
(`AGENTS.md` §Protocol). The three launch paths always record one; the
discovery path never does, because Corral did not start that process and
has not looked.

`resume_plan` reads the last Run's directory from the store. The runtime
handle stops being consulted for it — one owner, and the durable one.

`ResumeRefused::NotThisDaemon` is renamed `DirectoryUnknown`. After this,
a Run a previous daemon started is continuable and the surviving case is a
Run Corral never started, so the old name would describe the wrong fact.
Its sentence changes with it.

**Schema.** `runs` gains `working_directory TEXT`. `SCHEMA_VERSION` 4 → 5.
No migration is written: `STORAGE_EPOCH` is `dev`, development databases
are disposable, and a store at another version is refused rather than
guessed at — which is the mechanism already in place, not a new one.

## Interfaces or persistence changed

- **Durable schema**: `runs.working_directory TEXT`, nullable.
  `SCHEMA_VERSION` 5.
- **Durable event**: `RunStarted.working_directory`, optional, omitted when
  absent. No existing field changes meaning.
- **Wire protocol**: nothing. The directory is not projected to clients in
  this change.
- **No Corral-owned fact is reinterpreted or discarded.** Under `dev` an
  existing development store is refused at startup and recreated, which is
  what the epoch declares; from `dogfood` this same change would need a
  migration instead.

## Failure / unknown states

- A Run recorded with no directory — every discovered Run, and any Run in a
  store written before this change — refuses continuation with
  `DirectoryUnknown`. Unchanged behaviour, correctly named.
- A directory that has since been deleted or replaced fails the
  continuation at the spawn, as `usable_directory` already does on the
  history rung. Recording where a Run *ran* is not a promise the path still
  exists.
- A path that is not valid UTF-8 is stored losslessly or the Run's start is
  refused; it is never lossily transcoded into a different directory.

## Tests

- Store: a Run started with a directory reads it back; a Run started
  without one reads back `None` and is not defaulted.
- Store: the event round-trips, and a stored `run-started` without the
  field decodes as `None` (future-input coverage).
- `resume_plan`: a Session whose last Run recorded a directory is eligible
  with no runtime handle present — the pre-fix red, since this is exactly
  `NotThisDaemon` today.
- End-to-end: the existing history-continuation test continues the row,
  ends the Run, restarts the daemon, and continues it *again*. Today the
  restart half of that test asserts only that the row is still listed; the
  continuation after it is the behaviour this plan adds and is red before
  it.

## Definition of done

- `./scripts/verify` green on the final tree.
- The `resume_plan` and end-to-end regressions seen red before the change.
- ADR 0016 D5's sentence says what the code does, and ADR 0002 D6's record
  of what is durable names the new field.
- `scripts/check-schema-gate` satisfied by a `DURABLE-APPROVED-BY:` line a
  human added. Never an agent (`AGENTS.md` §Durable state).
