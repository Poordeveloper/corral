# Managed Claude arguments — founder ruling on a known grammar

> Founder decision, 2026-09-01, on the second review round of the Claude
> attachment work. Classified by the founder as a **high-consequence Class B
> decision**, human-gated and settled here: it resolves a discovered ambiguity
> surface inside the accepted managed-session envelope and does not reopen
> ADR 0009 or ADR 0011.

## What the evidence forced

Probing the installed 2.1.251 established that an unknown Claude option is not
merely a feature Corral does not understand — **it can change the syntactic
role of the tokens after it**. A free-text required-value option swallows a
following `--`, after which the words behind it are options again.

That makes a denylist the wrong safety model, because its failure mode is:

> a missing attaching spelling → Corral accepts the argv → Claude's parser
> reads it differently → a new managed launch silently attaches to another
> session.

That is wrong-target / identity corruption, and it must not rest on "we should
have found every dangerous spelling by now".

## R1 — An allowlist, scoped to where the option grammar has authority

> Corral accepts only caller-supplied Claude root options whose parsing
> semantics are explicitly known for the supported Claude version family. An
> unknown caller-supplied root option is rejected before Claude is spawned.
> This is intentional fail-closed behaviour.

Scope is narrow, and deliberately so. The rule applies **only inside the
caller-controlled argument region where Claude's option grammar still has the
right to interpret**. Content Corral has already established to be prompt or
data is not checked against a provider option allowlist — otherwise a
legitimate prompt such as `Explain why --foo behaves differently from --bar`
would be read as provider flags.

## R2 — A grammar, not a string scan

The authority question stops being "is this word in `ATTACHING_FLAGS`?" and
becomes **"do I know how Claude parses this word?"**:

```text
known root option
├─ forbidden semantics (--continue, --resume, --teleport, --cloud/--remote, …)
│    → reject
├─ known boolean            → consume the flag
├─ known required value     → consume exactly what the verified grammar says
├─ known optional/special   → explicit per-option rule
└─ unknown option           → reject
```

`ATTACHING_FLAGS` keeps its job, but only as the forbidden-semantics subset
*inside* a known grammar. It no longer carries completeness responsibility for
safety.

## R3 — `--` is not a natural boundary

> `--` is treated as an option terminator only when Corral's verified Claude
> grammar establishes that the parser is currently in a state where Claude
> would interpret it as one.

Ruled on the measured `--append-subagent-system-prompt -- --continue`, where it
was a value rather than a terminator. This gets a permanent regression.

## R4 — Two layers, and both stay

The conservative reading added in the previous commit is kept: an unknown flag
must never cause the tokens after it to be treated as safely consumed or
separated. The launch policy then adds the second layer — an unknown option
means the launch is refused rather than handed to Claude.

```text
parsing layer   unknown ⇒ never launder what follows
launch policy   unknown ⇒ never execute
```

## R5 — No runtime inventory, and no escape hatch

The `strings` sweep and the 928 parser probes are **research**, not product.
Corral must not reverse-engineer a provider's parser at launch time. The
pipeline is: spike → verified inventory → grammar checked in for the supported
version family → tests.

Forward compatibility pays the cost in the safe direction: a Claude release
that adds an option may have that option temporarily refused until it is
verified and added. That is better than a hidden attach alias quietly driving
the wrong session.

No raw-passthrough escape hatch in M1 — no `--raw-claude-arg`, no
`--trust-unknown-claude-flags`. A person meeting an unvalidated option is told
so and stops there. If dogfood shows the limit is painful, an explicit mode
that leaves managed guarantees behind is its own product decision, not
something taken in passing.

## The invariant

> For managed Claude launches, parser uncertainty fails closed. A
> caller-supplied provider option whose parsing semantics Corral has not
> validated for the supported Claude version must never be passed through
> merely for forward compatibility.

## What the inventory now means

The 69 required-value options, the 40 the help omits, the 8 hidden root
options, the `--remote → --cloud` alias, the `--teleport` family membership,
the free-text-swallows-`--` behaviour and the typed-value loud failure stop
being a denylist that must never miss anything. They are **verified parser
grammar evidence for the supported build**. A gap in them now costs a false
rejection, not a wrong attach — which is the failure direction to optimise.
