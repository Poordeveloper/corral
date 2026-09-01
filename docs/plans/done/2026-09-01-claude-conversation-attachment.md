---
status: done
class: C
writes: [corrald, corral]
reads: [docs/adr/0011-conversation-attachment-is-corrals-to-authorize.md, docs/decisions/2026-09-01-claude-conversation-attachment.md, docs/adr/0010-argv-transport-and-refusal-grounds.md, docs/adr/0009-codex-notify-delivery.md, docs/adr/0004-hook-delivery.md, docs/adr/0007-managed-session-lifetime.md, docs/references/2026-08-27-pr5-claude-code-hook-matrix.md, ARCHITECTURE.md]
---

# Claude conversation attachment — closing the bypass PR6 closed for Codex

**Class C, and why.** ADR 0011 is a new decision: a third ground for refusing a
caller argument, accepted 2026-09-01
(`docs/decisions/2026-09-01-claude-conversation-attachment.md`). It supersedes
part of ADR 0010 D2 and changes what a managed Claude launch accepts, which is
user-visible.

## Goal

`corral new claude -- --resume <id>` — and every other spelling that attaches a
fresh managed launch to a conversation Claude already has — is refused before
anything is minted, written, or spawned, for the reason ADR 0011 D1 names.

## Non-goals

No declared managed surface for Claude, and so no refusal of its other
subcommands (`mcp`, `update`, `doctor`, `agents`, …): pending decision, ruling
R2. No change to what Codex refuses — ground three is now the honest reason
`resume` and `fork` are refused, but they were refused already. No change to
`session.resume`, its claim, or its ladder. No new durable state, no wire
change, no epoch movement. No hook-injection change: an attaching launch
reports normally, which is exactly why nothing already in the code catches it.

## Existing owner / architecture involved

`corrald`'s `provider::claude` owns Claude's command-line knowledge;
`provider::ArgumentRefused` owns why an argument is refused and what a person
is told. `connection::read_session_new` already asks before anything is minted.
`managed_launch`'s continuation claim and `resume_plan`'s ladder are the things
being protected and are not touched.

## Design

**1. The third ground in the type.** `ArgumentRefused` gains
`AttachesToAnExistingConversation(String)`. Its sentence names what the person
wrote and says Corral continues a session it already knows rather than starting
a second agent on one — the same voice as the other two, no surface syntax.

**2. Claude's attaching arguments, measured on 2.1.251.** `-r`/`--resume`,
`-c`/`--continue`, `--from-pr`, `--cloud`, and the `attach` subcommand. Each is
refused in every spelling this CLI accepts, which the matrix seals.

**3. Reading the command line the way commander reads it** (ADR 0010 D2's rule,
and the part PR6 got wrong twice before getting right). Four measured facts
drive it:

- `--` ends option parsing; after it a flag-looking word is prompt text
  (measured: `claude -- --resume <id>` starts fresh and says so).
- A long flag takes `=`: `--resume=<id>` attaches.
- A short flag takes an attached value: `-r<id>` attaches, and `-r=<id>`
  attaches with a value commander does not strip the `=` from.
- Short flags cluster, and a value-taking letter eats the rest of the cluster:
  `-pc` continues, while `-nc`, `-dc`, and `-wc` do not, because `n`, `d`, and
  `w` take the remainder as their value. So a cluster is refused when `c` or
  `r` appears before the first of `n`, `d`, `w`, `r`.

No value-flag table for Claude, deliberately, and this is the one place the
Codex shape is not copied. There the refused things were bare words a value
could plausibly equal — a directory called `app`, a profile called `review` —
so a table was the difference between refusing a filename and not. Here every
refused thing is a flag spelling, and the one word-shaped check is a subcommand
commander only reads as the first argument (measured: `claude attach foo`
dispatches, `claude -p attach foo` does not). A value equal to `--continue` is
not a case that happens.

**4. `attach` is refused as the first caller word only,** because that is the
only place it means anything.

**5. What is not refused, and why it is written down.** `--fork-session` alone
attaches nothing — the help says it works only with `--resume` or `--continue`,
both already refused. `--bg`, `--remote-control`, and Claude's other
subcommands are outside this ground; ADR 0011 D2 names them as separate
questions rather than leaving them unmentioned.

**6. `--cloud` is refused although it can also create.** One argument, two
meanings, chosen by the provider from the value's shape at runtime. Corral
cannot tell them apart without interpreting a value it has no business
interpreting, and ADR 0011 D2 rules that direction: refuse. The over-refusal —
`--cloud "a description"`, which would have created — is stated in the refusal
list's own docs rather than discovered later.

**7. Codex keeps its behaviour and gains the honest reason.** `resume` and
`fork` are refused by the surface ground today; ground three is why that
matters. The adapter's comment says so; no code moves.

**8. The matrix.** A dated first-party record against the installed 2.1.251
with PR5's fields: every spelling above driven, the separator, the cluster
rule, the subcommand position, and what was not driven. `PRODUCT.md` §10's
Claude row gains it beside the PR5 hook matrix.

## Interfaces or persistence changed

No wire change, no durable change. `session.new` refuses argument sets it
previously accepted, which is user-visible and is the compatibility surface
this carries: the answer is `invalid_params` with a sentence naming the
argument, exactly as the other two grounds answer.

## Failure / unknown states

A spelling a later Claude release adds is one this list does not know — the
same version-sensitivity every refusal here carries, and the same degradation:
the launch proceeds and Corral learns what it learns. A person who meant the
word as a prompt passes it after the agent's own `--`, which is measured to
work. Nothing about an already-running Session changes.

## Tests

- Unit: every measured spelling of every attaching argument is refused —
  separated, `=`-joined, short-attached, and inside a cluster; `attach` as the
  first word; nothing after `--`; `-nc`/`-dc`/`-wc` and `--fork-session` alone
  are the caller's; `--settings`/`--safe-mode` still refuse for their own
  ground and still say their own sentence.
- Regression shape: each new refusal fails on the pre-fix behaviour.
- End-to-end: `session.new` with `provider: claude` and `args: ["--resume",
  <id>]` is refused, nothing is spawned, and no Session is created — the same
  test PR6 has for Codex, on the other provider.
- Codex: its refusals are unchanged, asserted by the tests already there.

## Definition of done

- Designs 1–8 implemented; `./scripts/verify` green on the final tree.
- Matrix recorded with PR5's fields; the pending surface decision named in it.
- Human-merged: Class C carrying ADR 0011.
- Plan moves to `done/`; `STORAGE_EPOCH` untouched.

## Follow-ups

- Whether Claude gets a declared managed surface, and with it a refusal of its
  remaining subcommands (ADR 0011 D2, ruling R2).
- Whether an argument that relocates execution should be refused, degraded, or
  left alone — Claude's `--bg` and `--remote-control`, Codex's `--remote`.
- The Linux single-argument ceiling still wants measuring rather than citing
  (PR6 follow-up, unaffected by this work).
