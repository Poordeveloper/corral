# Founder Decision Record — the dependency policy gpui brings with it

> Status: founder-ruled in `2026-09-04-pr9-spike-grill.md` Q4 (2026-09-04);
> materialized by the `deny.toml` change that lands with this record.
> Human-reviewed and human-merged: the merge gate's own policy is a
> third-party surface change. Lands before the PR9 plan so that plan is
> written against the gate it will actually meet.

## What the spike measured

`cargo deny check` over gpui 0.2.2's dependency tree with the policy as it
stood: 25 crates under licences outside the allow list, seven unmaintained
advisories, 64 duplicated crate versions
(`docs/references/2026-09-04-pr9-gpui-integration-spike.md`, scenario 2).
Scoped to the platforms Corral builds for, 22 licences and six advisories
remain. The image codecs (`rav1e`, `ravif`, `exr`, `avif-serialize`) arrive
through gpui's default `image` features and cannot be shed without a fork.

## D1 — The graph is resolved for macOS and Linux only

`[graph] targets` names the four triples Corral builds. Windows-only
dependencies are neither audited nor blessed; adding Windows as a supported
Desktop target widens this list first, so their review happens then. The
list is a statement of the supported envelope, never a way to avoid reading
a dependency.

## D2 — Five licences join the allow list, one of them deliberately

BSD-2-Clause, BSD-3-Clause, ISC, CC0-1.0, and `Apache-2.0 WITH
LLVM-exception` are permissive or public-domain and carry notice
obligations at most. **MPL-2.0 is accepted as file-level weak copyleft, not
mistaken for permissive**: modifications to MPL-covered files must be
published under the MPL; Corral modifies none, and linking carries no
obligation; any applicable source obligation stands. The compound
expressions in the tree (`ring`: Apache-2.0 AND ISC; `encoding_rs`:
(Apache-2.0 OR MIT) AND BSD-3-Clause) are satisfied by the list and need no
per-crate exception. The LLVM exception was not in the grill's list because
the grill's own parse of the report missed it; it is strictly more
permissive than the Apache-2.0 already allowed.

## D3 — Six advisories are ignored as itemised debt, each with an exit

`async-std` (RUSTSEC-2025-0052), `paste` (RUSTSEC-2024-0436),
`proc-macro-error2` (RUSTSEC-2026-0173), `rustls-pemfile`
(RUSTSEC-2025-0134), `rustybuzz` (RUSTSEC-2026-0206), `ttf-parser`
(RUSTSEC-2026-0192). All unmaintained, none vulnerable, all transitive
through gpui 0.2.2, none replaceable by Corral alone. `deny.toml` carries,
per entry, the path, why Corral cannot substitute it, the exposure, and the
exit condition. Review triggers for every entry: any gpui upgrade; an
upstream replacement or removal; a change in the advisory's severity; and
the M1 public release, before which a standing entry is re-justified or the
dependency cut. The shaper pair (`rustybuzz`, `ttf-parser`) sits on the
Desktop's text path and is the one to watch.

## D4 — Duplicate versions stay a warning

No evidence that duplicate-version cleanup is worth a merge blocker.

## What this does not decide

Whether gpui is adopted at all (decided: ROADMAP, ledger §8, grill Q2),
which gpui version (`=0.2.2`, grill Q2), or anything about the Desktop
crate's own layout. One repository keeps one merge-ready dependency policy;
necessary exceptions are explicit debt, not hidden by workspace topology.

## Evidence

- `cargo deny check` on this workspace with the new policy: advisories ok,
  bans ok, licenses ok, sources ok (six `advisory-not-detected` and four
  `license-not-encountered` warnings, expected until a gpui dependency
  exists).
- The same policy over the spike's gpui 0.2.2 scratch tree: advisories ok,
  bans ok, licenses ok, sources ok.
