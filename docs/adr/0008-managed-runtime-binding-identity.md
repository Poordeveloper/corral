---
status: accepted
read_when:
  - creating or resolving a binding for a runtime Corral launched itself
  - adding a provider's own session identity to a Session
  - choosing what a Runtime binding's external id or provider field holds
  - deciding what may occupy the reserved `corral` provider namespace
---

# How Corral names and assures the managed runtime it owns

`ARCHITECTURE.md` §1 fixes that a Binding is an edge from a Session to an
external identity, and ADR 0002 fixes the Run vocabulary those edges carry
facts under. Neither says what identity a runtime *Corral itself launched*
has — there is no external system to have named it, and the two obvious
answers are both forbidden: a pid is never identity, and grill Q10 ruled that
no pid is persisted.

That gap is why `corrald` recorded nothing at all. `record_run_started`
requires a Runtime `BindingId`, and no rule said what one for a managed session
would name, so the durable path was never wired.

Ruled in the durable-lifecycle grill, Q1 and Q12. Acceptance evidence:
`docs/decisions/2026-08-24-pr3-durable-lifecycle-grill.md`, which carries the
founder's rulings verbatim; accepted 2026-08-24 by merging the pull request
that landed this ADR and instructing implementation to begin.

**The invariant.** Corral names what it owns, and the name says only that. A
managed runtime's identity must be stable enough to carry a Session's Runs
across a resume, and narrow enough that it never becomes a claim about which
coding agent is running — because the phase that adds real providers will
otherwise inherit one field meaning two things.

## D1 — The managed RuntimeBinding, frozen

```text
RuntimeBinding {
  node,
  kind        = Runtime,
  provider    = "corral",
  external_id = an opaque Corral-minted stable binding identity,
  provenance  = CorralCreated,
  assurance   = Deterministic,
  …
}
```

`Assurance::Deterministic` is already documented as *"`corrald` spawned and
owns the runtime; identity holds by construction"*. This is that case; no new
assurance level is introduced.

## D2 — What the external id is, and the four things it is not

It is stable for the life of a Session's managed-runtime binding, and a resume
or replacement Run reuses it. It is **not** the pid, **not** the `RunId`,
**not** a concrete runtime occurrence, and **not** a provider session id.

So it is not a "process handle" and not a "Run handle", and neither name should
be used for it. What it names is:

> Corral's control-capable managed-runtime binding for this Session.

A concrete runtime occurrence is always expressed by a `Run`. That separation
is the whole point: one Session has one managed-runtime binding and many Runs
under it, and a store that permits at most one control-capable Runtime binding
per Session is consistent with a Session that runs, ends, and runs again.

Resolution is therefore lookup-first: a Session that already has a
control-capable Runtime binding **reuses** it; only a Session with none mints
one, once.

## D3 — The reserved `corral` provider namespace, and its direction

`provider = "corral"` on a managed RuntimeBinding means exactly:

> Corral is the authority that minted this runtime-binding identity.

It does **not** mean the coding-agent provider is Corral.

The reservation is directional rather than a blanket refusal of the string —
the Corral-owned RuntimeBinding is precisely the thing that needs it:

- a `CorralCreated` Runtime binding's provider **MUST** be the reserved id;
- a `ProviderSession` binding's provider **MUST NOT** be the reserved id;
- no other provider-derived identity binding may occupy the reserved
  namespace.

`corral-core` names the reserved id once; the store and domain validation
enforce the direction. Without that enforcement D2's durable meaning survives
only by convention, and the first provider phase is where conventions go.

## Consequences for PR5 and PR6

A managed Claude session carries two bindings on one Session, with no
ambiguity:

```text
RuntimeBinding.provider  = corral      Corral owns and named this runtime
ProviderSession.provider = claude      the agent's own session identity
```

and PR6 the same with `codex`. Provider identity arrives as a *separate*
binding; it never rewrites `RuntimeBinding.provider` or
`RuntimeBinding.external_id`. Runtime ownership and provider identity are two
facts, and this ADR exists so a later phase cannot quietly make them one field.

## Storage epoch

`STORAGE_EPOCH` is `dev`, so development databases that violate the D3
direction may be reset destructively rather than migrated. The epoch answers
migration cost only: it does not lower this from a durable semantic invariant,
and from `dogfood` onward the ordinary rules bind.

## Alternatives rejected

**pid, or pid plus start time.** Forbidden twice over: identity is never a pid
(`AGENTS.md` §Core model), and grill Q10 ruled that no pid is persisted, read
back, probed, or used as a kill target.

**`CorralSessionId` as the external id.** It works and needs no minting, but it
binds a Session to "external identity: itself" — circular in a field whose
whole purpose is to name something outside Corral's key space.

**No Runtime binding for managed sessions.** It would mean recording no Runs,
and `Run` is the vocabulary ADR 0002 accepted for exactly this.

**A globally refused `corral` provider string.** Refusing it everywhere would
refuse the one binding that must carry it. The invariant is directional, and
stating it as a blanket ban would have been simpler and wrong.
