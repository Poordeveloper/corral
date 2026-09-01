---
status: accepted
read_when:
  - deciding whether a caller's provider argument may be passed through to a managed launch
  - adding or updating a provider's verified command-line grammar
  - tempted to add a raw-passthrough escape hatch for provider arguments
  - deciding what `--` means while validating a provider command line
---

# A managed launch passes only arguments whose parsing Corral has verified

**Supersedes in part:** ADR 0010 D2's grounds for refusing a caller argument,
and the completeness responsibility ADR 0011 D1 implicitly placed on the
attachment list. Both decisions stand; what changes is that neither is load
bearing for safety any more.

Accepted 2026-09-01
(`docs/decisions/2026-09-01-claude-argument-allowlist.md`, R1–R5), classified
by the founder as a high-consequence Class B decision inside the accepted
managed-session envelope. Measured evidence:
`docs/references/2026-09-01-claude-2.1.251-attachment-matrix.md`, scenarios
9–11.

## D1 — Unknown parsing fails closed

> For managed launches, parser uncertainty fails closed. A caller-supplied
> provider option whose parsing semantics Corral has not validated for the
> supported provider version must never be passed through merely for forward
> compatibility.

The reason is not tidiness. Measured on Claude Code 2.1.251, an unknown option
does not merely name a capability Corral does not understand — it can change
the syntactic role of every token after it. A free-text required-value option
swallows a following `--`, and the words behind that `--` go back to being
options. So a refusal list built as a denylist fails like this:

```text
one attaching spelling missing
  → Corral accepts the argv
  → the provider's parser reads it differently
  → a new managed launch silently attaches to another session
```

That is a wrong-target failure — Corral driving a conversation nobody pointed
it at — and safety must not rest on having already found every dangerous
spelling.

The cost is paid in the other direction and accepted: a provider release that
adds an option may have that option refused until it is verified. A refused
launch is loud, local, and recoverable; a wrong attach is none of those.

## D2 — The authority is a grammar, not a list

The question a validator asks stops being "is this word forbidden?" and becomes
**"do I know how this provider parses this word?"**

```text
known root option
├─ forbidden semantics   → refuse, naming which ground
├─ known boolean         → consumes the flag
├─ known required value  → consumes exactly what the verified grammar says
├─ known optional value  → its own explicit rule
└─ unknown               → refuse
```

The forbidden-semantics sets — competing with the injection (ADR 0009 D5,
ADR 0010 D2), selecting an unmanaged surface (ADR 0010 D2), attaching to an
existing conversation (ADR 0011 D1) — keep their meaning. They are now subsets
*inside* a known grammar rather than the whole of the defence.

## D3 — The scope is exactly where the option grammar has authority

The rule applies inside the caller-controlled region the provider's option
parser still interprets, and stops where Corral has established that the
provider will read data instead. Content already established as prompt or data
is never checked against an option allowlist: a person whose prompt discusses
`--foo` is writing prose, not flags.

Measured for Claude 2.1.251: options are still parsed after a positional, so
that region runs to a terminator and not to the first prompt word.

> `--` is an option terminator only where the verified grammar establishes that
> the parser is in a state to read it as one.

`--append-subagent-system-prompt -- --continue` is why: there the `--` was a
value. This carries a permanent regression.

## D4 — The grammar is checked in, never discovered at runtime

Reverse-engineering a provider's parser is research, and the pipeline runs one
way only:

```text
spike → verified inventory → grammar checked in for the supported version
      → tests → the provider/version matrix
```

Corral must not inspect a provider binary, probe its parser, or infer a grammar
from error wording while starting a session. That would make a provider's
internals a runtime dependency of the control plane and turn every provider
release into an unbounded behaviour change.

## D5 — No raw passthrough in M1

No `--raw-claude-arg`, no `--trust-unknown-provider-flags`, no per-launch
override. Any of them re-opens D1 the day it ships, and the invariant is worth
more than the convenience. A person who meets an unvalidated option is told
which one and stops there.

Deferred rather than rejected: an explicit mode that leaves managed guarantees
behind — with what Corral then stops claiming stated plainly — is its own
product decision, to be taken on dogfood evidence rather than in passing.

## What this does not decide

Which surfaces of a provider are managed (ADR 0009 D1 for Codex; still open
for Claude). What a provider's grammar contains — that is evidence per version,
recorded in the matrix, not law. Whether Codex's validator adopts the same
shape: its parser does not swallow a terminator (`codex -C -- resume` errors
instead), so the same reasoning reaches a different implementation, and
tightening it is a separate task with its own evidence.

The cross-provider rule this does settle:

> Share the safety invariant, not the parser assumptions.

Every provider may be held to "a managed launch must not let unverified argv
rewrite its control semantics". How each one's option/data boundary is found is
that provider's own parser evidence, per version — copying one adapter's
reading into another is how a measured fact quietly becomes an assumption.
