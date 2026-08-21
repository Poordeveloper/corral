# Contributing to Corral

Thanks for your interest. Corral is pre-release and moving fast, so the most
useful contributions right now are bug reports with reproductions, provider
format fixtures, and focused fixes.

Before writing code, read [`AGENTS.md`](AGENTS.md) — the repository's hard
rules — and the section of
[`docs/ENGINEERING_WORKFLOW.md`](docs/ENGINEERING_WORKFLOW.md) that matches
your change.

## Ways to contribute

- **Report bugs.** Include your OS, provider CLI versions, and what you
  expected versus what happened. A session Corral failed to discover is a
  useful report on its own.
- **Contribute fixtures.** Real-format provider history and hook payloads —
  scrubbed of anything private — are directly valuable; Corral's parsers are
  tested against real formats, never synthetic guesses.
- **Fix bugs and implement features** through pull requests, after an issue.

**Security vulnerabilities**: do not open a public issue. Contact the
maintainer directly.

## Open an issue first

Issue-first is required for external contributions and for anything that
adds product scope. It is not bureaucracy — Corral's architecture has
settled invariants, and an issue is where we find out in five minutes
whether your change crosses one, instead of after you have written it.

## How a pull request is handled

Corral develops heavily with coding agents, so every change — internal or
external — moves through the same machinery:

1. **Classification.** A maintainer assigns the change class. You are not
   expected to know Corral's class system: the PR template asks a few
   surface questions (does this touch the protocol, storage, runtime
   ownership, provider integration, or security?) and a wrong guess is never
   held against you.
2. **Review.** Automated verification plus one or more fresh-context
   reviews, then a human maintainer's review.
3. **Merge.** Every external pull request is merged by a human maintainer.
   Nothing merges automatically.

If your patch reaches an architectural, storage, security, or compatibility
decision, it is marked `BLOCKED — DECISION REQUIRED`. The decision is pulled
out into an issue or an ADR and settled first; then the PR continues, is
revised, or is split. This is not a judgement on the code — the decision bar
does not move because code already exists, and pre-writing a large patch is
exactly the effort the issue-first rule is meant to save you.

Changes to `AGENTS.md`, `docs/GOVERNANCE.md`, or
`docs/ENGINEERING_WORKFLOW.md` are issue-first and require explicit
maintainer acceptance.

## Verification

```bash
./scripts/verify-fast     # while iterating
./scripts/verify          # before asking for review
```

`./scripts/verify` is the definition of done. Do not invent another one, and
do not disable, skip, or loosen a check to make it pass — a red result that
is not yet understood is information, not an obstacle.

Commits follow Conventional Commits (`feat|fix|refactor|docs|test|chore`),
one focused topic per pull request.

## AI-assisted contributions

Corral is itself built with AI assistance, so this section is a working
standard rather than a disclaimer. The policy is adapted from
[CC Switch](https://github.com/farion1231/cc-switch)'s, which states it well.

We welcome AI-assisted contributions, but **the responsibility stays with
you**. AI tools lower the cost of writing code — they do not lower the cost
of reviewing it. Maintainers are not obligated to clean up AI-generated
output.

By submitting a PR, you agree to the following:

1. **You have read and understood your code.** You must be able to explain
   any line in your PR. If you cannot, it is not ready for review.
2. **You have tested it yourself.** Every change must be verified locally —
   not just "it looks right". Do not submit code for platforms or features
   you cannot test.
3. **PRs must be small and focused.** One issue, one PR. Large, sprawling,
   multi-topic PRs will be closed.
4. **Open an issue first.** Drive-by PRs with no prior discussion —
   especially AI-generated ones — may be closed without review.
5. **Maintainers may close without explanation.** PRs that appear to be
   unreviewed AI output — hallucinated fixes, unnecessary refactors, bulk
   changes with no context — may be closed at the maintainer's discretion.

In short: AI is a tool, not a substitute for understanding.

## License

By contributing, you agree that your contributions are dual-licensed under
Apache-2.0 and MIT, matching the project.
