---
status: accepted
read_when:
  - listing a session Corral has never seen run
  - reading a provider's session store or history directory for any purpose
  - deciding whether a Session may be continued, and what a person must be told first
  - adding a provider whose session store has a different shape
---

> Accepted 2026-09-03. Structural rulings founder-accepted 2026-09-02,
> rounds 1–4; the working-directory policy in D5 is round 5's Q35
> (`docs/decisions/2026-09-02-pr8-attention-grill.md`). The load-bearing
> store and resume facts are measured, version-specific, and durable in
> `docs/evidence/pr8b-history-store-and-resume-2026-09-02.md`: they seal
> Claude Code 2.1.258 and Codex 0.152.0 and no other version.

# History-enumerated sessions: what a provider's own store may claim, and when Continue in Corral is offered

`PRODUCT.md` §9 promises a first list of live sessions plus recent
resumable ones "reliably discovered from supported provider history", each
offering Continue in Corral, and the UX contract froze the boundary: recent
resumable sessions in the normal list are M1; browsing, search, and
timeline are M2 (`docs/decisions/2026-08-21-m1-ux-contract.md` §6).
`ARCHITECTURE.md` §1 makes history a facet, keeps it provider-owned, and
says a heuristic binding never controls. ADR 0014 D6 left the surface that
offers Continue in Corral for discovered sessions to PR8 and put the
refusal grounds in place. ADR 0009 rejected reading history files as live
evidence. Between those, one question is open and it is this ADR's: what a
session that exists only in the provider's store may claim, and whether
that is enough to continue it. Scheduled by `ROADMAP.md` §3 for PR8.

**The invariant.** A provider's own session store proves that a session
exists and what the provider calls it. It proves nothing about whether
that session is running anywhere. Corral continues such a session only
after saying exactly that.

## D1 — Enumeration, not parsing

In this phase Corral reads a provider's session store for three facts and
no others: the provider session identity, from the file's name; recency,
from the file's modification time; and a location hint, from the directory
the provider files it under. The layout each fact is read from is sealed
per provider and version by the PR8 matrix, the way a recognizer shape is
(ADR 0014 D2), because a store can also hold files that are not sessions —
a sub-agent transcript beside its parent, a rollout of a headless run —
and the file that is not a resumable session must be told apart by shape
rather than guessed at.

A location is shown only when the evidence supports the display claim
(grill Q25): Corral holds an exact working directory for the Session, or
the provider's encoding is proven reversible, or a sealed metadata source
supplies the path. Claude's project directory encodes the working
directory with dashes for separators, which is not reversible when a path
contains a dash of its own, and a decoded candidate that exists on disk
proves nothing about which path the provider meant — so a pure history
row from that store shows no location, and never shows the encoded name
as though it were a path. A hint may be optional; it may not be
fabricated because identity does not depend on it.

File content is not read. A title, a summary, a last message, a turn count
are history parsing, and history parsing is M2's with its own
`HistoryParser` and its own index (`ARCHITECTURE.md` §5). Provider-owned
files stay read-only (`ARCHITECTURE.md` §6); enumeration opens nothing for
writing and holds nothing open. The `HistorySource` seam
(`ARCHITECTURE.md` §10) is instantiated here for the one thing it does.

Enumeration runs at daemon start and on a bounded cadence, and it is
evidence about the store's *contents*, never about a process: ADR 0009's
rejection of file-watching as live evidence stands untouched, because
nothing here asserts that anything is running.

## D2 — Resolution before creation, and a row that is not yet a Session

An enumerated identity resolves first, by `(node, provider, external_id)`
across the binding kinds, against the Sessions Corral already holds. A
match decorates that Session — recency, the location hint — and mints
nothing: a managed session that exited yesterday and its history file are
one row, and a discovered session whose Run is still open and its history
file are one row, because discovery is idempotent and the provider-id-keyed
record wins (`ARCHITECTURE.md` §1).

A decorated Session is still a row. Nothing live outlives the daemon that
held it, so once a continued Session's Run has ended and that daemon is
gone, the store is the only thing left saying the Session exists — and a
session that vanished from the list by having been continued once is the
disappearance this rule exists to prevent. The store's rows are therefore
listed under the identities they resolved to, and the ones a live tier is
already showing are dropped there rather than never produced.

Resolution and publication are one answer. A pass resolves its entries
against the registry one at a time and publishes them together; a
continuation landing in between gives one of those identities a Session,
and the pass's answer for it is older than that. Publishing it anyway
would mint a second id for a provider session that now has one — and
offer it for Continue, where the provider process is spawned before the
store refuses the duplicate. A pass whose answers were overtaken is
dropped, and the next one reads the store as it stands.

An identity that resolves to nothing is a **history row**: live daemon
state, origin `history`, with no Run, no runtime, and no durable Session.
It enters the durable log at its first durable-grade fact — the person's
Continue in Corral — as `SessionCreated`, `BindingAdded` of kind
`HistoryBinding`, and the continuation's own `RunStarted`, in one
transaction, exactly as a provisional external Session enters it at its
first Attested identity (ADR 0014 D5). A daemon restart re-enumerates
rather than replays. Nothing is fabricated to avoid an empty list, and
nothing is hidden to make one shorter than the recent window.

## D3 — What a history-claimed identity is entitled to: assurance is claim-scoped

A `HistoryBinding` whose external id was read from the provider's own store
at a sealed path carries assurance **Attested** for the claim it makes,
and the claim is exactly this: *the provider's own store holds a session
it calls X*. It is not, and never becomes, the claim *the runtime observed
here is carrying X*. That is a live binding claim; it needs live
corroboration by an observed process, and no amount of history supplies it
(grill Q4).

This is the decision that has to be made out loud, because the glossary's
Attested reads "live provider-native evidence … corroborated by an
observed process" and its Heuristic reads "cwd / time / process / history
correlation" — both written for the live claim. Corroboration is what a
`ProviderSessionBinding` needs, because it asserts a live association; a
`HistoryBinding` asserts none, and the store proves its own contents by
construction, which is the authority the provider's own `--resume`
consults. "History correlation" stays Heuristic and means what it says:
matching a live process to a history record by cwd, time, or start
proximity. A history row is therefore never display-merged with a sweep's
provisional row on cwd, and never gains a runtime by correlation. On
acceptance the glossary is corrected so that no wording makes every
history-derived identity Heuristic:

> Assurance qualifies a claim, not an object globally. Attested means the
> evidence directly supports the specific claim being made; for a live
> binding claim, provider history alone is insufficient.

The alternatives each break something already accepted. Heuristic would
put `AGENTS.md` §Core model — a heuristic binding never enables control —
in front of the one operation the row exists to offer, and PRODUCT §9
becomes unimplementable. Manual would spend the level reserved for a
person's identity assertion on a click that asserts nothing about identity.
A fifth level reopens a settled four-level vocabulary for one binding kind.

What Attested buys here is exactly identity: the safe history operations
D4 lists may name the id. It buys no runtime control, because there is no
runtime binding to drive (ADR 0014 D6's structural read-only), and no main
state, because a history record is entitled to nothing about the present
(ADR 0015 D3).

## D4 — Continuation eligibility, by what Corral knows about the Run

`session.resume` walks the existing ladder — sufficient assurance,
Confirmed identity, and the state of the Session's Runs — and the last
rung now has three answers, because "no live Run" and "exit established"
were written for Runs Corral watched and there is now a Session with none:

```text
managed Run open                            ordinary Running; Open it
managed Run ended Unverifiable              refused: "Corral couldn't
                                            verify that the previous
                                            process ended, so
                                            continuation is unavailable."
                                            Q7 stands, and the copy never
                                            says "may still be running"
                                            without evidence for it
external Run open (discovered, Attested)    unavailable in this phase:
                                            "Still running outside
                                            Corral. Continuation is
                                            unavailable while this
                                            session remains live."
last Run ended Exited (any origin)          eligible
no Run known (a history row)                eligible with a disclosure,
                                            in the directory the client
                                            stated (D5): "Corral can't
                                            tell whether this session is
                                            still running somewhere else.
                                            Continuing starts another
                                            <Provider> process for this
                                            session in <directory>."
                                            — possible concurrency, never
                                            a claim another process
                                            exists (grill Q33); refused
                                            outright if no usable
                                            directory was stated
```

The second row is a phase limitation, and it is never written down as the
product invariant "live observed sessions may never Continue" (grill Q5).
It is deliberately narrower than the UX contract's rung 3, which permits
continuing a live observed session under a fork disclosure with the
left-behind branch kept as its own row — a topology this phase cannot yet
express truthfully: the original branch retained visibly, the continuation
as its own row, relation text, attention ownership kept honest, the old
branch never presented as resolved. A control action without the product
capability to show what it did is the fork-now-explain-later shape the
grill rejected, and building the branch model early inside PR8b was
rejected with it. On Linux with integration on, a live external session is
already a discovered one with an open Run, and the answer that case
actually wants is rung 2, which S3 has not yet earned; if M1 still needs
the rung 3 fallback once S3 has ruled, the phase that owns the branch
surface completes it. The fourth row is where the disclosure carries the
weight, because on macOS and with integration off Corral cannot discover
the live process at all (ADR 0014 D2), and refusing every history row
would make the first-run list a list of things Corral will not do.

A continuation from a history row is a managed launch like any other:
Deterministic runtime binding, launch token, injected hooks, and the
provider's first identity report confirming the `HistoryBinding`'s claim
as a `ProviderSessionBinding` at Attested — or contesting it (ADR 0004
D8), which is the one way the store's claim can be found wrong.

Sealing is asked again when a history row's continuation is decided, not
only when the store was read. The row is an observation; sealing is what
makes it evidence, and it is a property of the binary a continuation
launches — the one installed now, which an in-place upgrade can change
between one enumeration pass and the next. An unmeasured version inherits
nothing, so a row learned under a sealed version is not a licence to start
an unmeasured one for the length of a cadence, and a decision that finds
its provider unsealed retracts the rows rather than leaving them offered.
The working directory is rechecked on the same path for the same reason:
both are mutable state that the decision, not the enumeration, is
answerable for.

The check binds to a file, not to a name. A version is sealed on the
executable it was read from, so that executable is what the continuation
runs — carried into the launch rather than resolved again from the
provider's program name, which an in-place upgrade would answer
differently between the check and the exec. Continuations that rest on a
durable provider binding make no version claim and resolve the program
the way any command does. What remains is the filesystem race between
reading a file's version and executing it, which is a different and much
smaller thing.

Enumeration reads the sealed shape, not an approximation of it. A tree of
the right depth ending in a plausible identity is not what was measured:
Codex's dated directories are dates and its rollout names carry the time
the measurement recorded, and a file that does not have that shape is one
of the other things a provider keeps in its store (grill Q25).

Enumeration follows no symlink. The sealed layouts describe what a
provider writes *under* its store; a link is a name in the store pointing
at a file that is not, and following one would enumerate whatever the
filesystem can reach from there and grant it the assurance a history
record carries. Directory entries are classified by their own type and a
session file's time is its own, never a target's.

## D5 — The disclosure is the daemon's, correlated, and never assumed

Which of D4's answers applies is the daemon's to decide and the client's
to show in the daemon's words. `session.continuation` returns the
decision, the disclosure text and code, and a `disclosure_revision` bound
to that exact decision; `session.resume` carries the revision back. At
resume the daemon recomputes eligibility and, where a disclosure is
required, accepts the call only if the revision still matches — otherwise
it refuses and requires a fresh preflight (grill Q18). A bare "disclosed"
flag was rejected because it cannot say *which* disclosure a client
showed, and a stale one would let a person continue under yesterday's
answer.

**Where a history row is continued (grill Q35).** A store entry supplies no
directory, and the measured providers resume an id from anywhere and then
run *in the directory they were started in*
(`docs/evidence/pr8b-history-store-and-resume-2026-09-02.md`). So identity
does not imply location, and the location is stated by the initiating
client: `session.continuation` and `session.resume` both carry a
`working_directory`, and the daemon never substitutes its own working
directory, a path decoded from the provider's store label, the store file's
location, the account home, or a previous guess. A continuation that needs a
directory and was given none is refused, not defaulted. The directory must
be absolute, exist, and be a directory; it is checked again on the way to a
spawn, and a directory that has since gone fails the continuation rather
than falling back to another one. Which directory a client asks for is that
client's policy — the CLI and the TUI send their own working directory — and
a later directory picker replaces that default without changing any of this.
A Session Corral launched keeps the working directory Corral recorded for
it; a requested directory neither overrides that nor is silently adopted
from it.

The requested directory is one of the facts the decision is computed from,
so it is one of the facts the revision covers: changing directory after the
preflight is a different decision, and the resume is refused rather than
starting a process somewhere nobody was shown.

The revision means one thing: the client obtained the disclosure
associated with this exact continuation decision. A client convenience
that skips its own confirmation step — `corral continue --yes` — still
runs the preflight, renders the disclosure, and carries the revision; the
daemon recomputes and a stale revision is refused regardless (grill Q34). It is not consent, and
it is not authorization; whether the text was rendered in front of a
person is the client's UX contract, which no wire can prove. The wire
document says so in those words: disclosure correlation, not consent. A
daemon that assumed a person had been told would be the fork without the
disclosure that `PRODUCT.md` §3 forbids; a client that decided for itself
when one was needed would be a client deriving eligibility.

## Rejected

- **Watching history files for live status.** ADR 0009 D1's rejection
  stands; enumeration asserts nothing about a process.
- **Reading the first line, or the last, for a title.** It is parsing, it
  is M2, and a title read today becomes a contract tomorrow.
- **Merging a history row with a sweep row on cwd.** Heuristic
  correlation never binds; two honest rows and a weak hint is the ruled
  shape (`PRODUCT.md` §6), and even the hint waits for a phase that
  renders it.
- **Projecting `exited` for a history row.** Corral observed no end; the
  execution dimension is `unknown`, and the row says so.
- **Refusing every history row for safety.** Honest about one hazard and
  silent about the product: the disclosure is the honest shape, and the
  hazard is the provider's, which already permits concurrent resume
  (PR5 matrix, scenario 6).

## Load-bearing facts, measured and open

Measured 2026-09-02, durable in
`docs/evidence/pr8b-history-store-and-resume-2026-09-02.md` (narrative in
`docs/references/2026-09-02-pr8-attention-matrix.md`). Every fact below is
sealed for **Claude Code 2.1.258 and Codex 0.152.0 only**; a version whose
layout has not been measured is not enumerated, and the founder's macOS
installs (2.1.252, 0.145.0) inherit nothing (grill Q28):

- Claude 2.1.258 files one `<uuid>.jsonl` per session, headless `-p`
  runs included, beside a `memory/` directory; no sub-agent files
  appeared on this version. The directory name encodes the working
  directory irreversibly (`-root-proj`). The file's modification time
  advances with each turn.
- Codex 0.152.0 names each rollout file with the session's own
  `thread-id`, writes it at startup, and writes none for the
  title-generation thread — the discriminator between a session identity
  and an internal one. Beside the rollouts it keeps an append-only
  `session_index.jsonl` of `{id, thread_name, updated_at}` records, a
  `thread_history_1.sqlite`, and a `migrate-rollouts` subcommand that
  calls the rollout files legacy; whether the index is ever read is the
  separate ruling grill Q9/Q25 reserved.

Resume from a directory other than the session's, measured 2026-09-02 on
these versions and recorded in
`docs/evidence/pr8b-history-store-and-resume-2026-09-02.md`: both providers
resolve the id without its directory, append the new turn to the original
file, keep the id, and carry on with the new directory as the working
directory — Codex records it in the rollout, Claude files a `memory/` under
the new directory's project name. So D4's fourth rung is mechanically
possible from anywhere, and the directory is not the store's to supply;
D5 says whose it is and that the disclosure names it.

Still open, and none of it load-bearing for the decisions above: whether
resuming touches Claude's file time; how a headless Codex rollout is told
from an interactive one by name alone (`codex exec` was not run);
enumeration cost on a large store.

## What this does not decide

History parsing, indexing, search, and the history library (M2). Archive
and delete. Deleting anything provider-owned, ever. The recent window's
value (tuning). The left-behind-branch surface for continuing a live
external session (follow-up, after S3). Remote nodes' stores (M3).
