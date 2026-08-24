# PR3 pre-merge fuzz campaign — the terminal ingest path

> Required by ADR 0003 D1 and D9 before PR3 ships. Founder-approved layering:
> `docs/decisions/2026-08-24-pr3-plan-grill.md` (Q3).

## What was fuzzed

| | |
|---|---|
| Target | `terminal_ingest` (`fuzz/fuzz_targets/terminal_ingest.rs`) |
| Under test | `AuthoritativeTerminal::consume` → `resize` → `encode`, i.e. the path every byte of untrusted provider output takes |
| Commit | `4294d3a9d0b5`, branch `task/pr3-terminal-runtime` |
| Emulator | `qwertty-term-vt` 0.4.0 |
| Tool | `cargo-fuzz` 0.13.1 (libFuzzer), driven by `./scripts/fuzz-terminal` |
| Toolchain | `cargo 1.100.0-nightly (e8cb624d5 2026-08-22)`, release + debuginfo |
| Instrumentation | AddressSanitizer (cargo-fuzz default), `-rss_limit_mb=4096`, `-timeout=30` |
| Platform | macOS 15.5, arm64 (Apple silicon) |
| Seeds | the twenty curated cases in `crates/corrald/tests/corpus/terminal` |
| Duration | three runs: ~15 min, ~10 min, and ~10 min wall clock |

The target steers geometry and chunk size from the input rather than fixing
them, because a sequence torn across a read is the ordinary case on a PTY and
a parser only tested on whole messages has not met what it will actually get.

## Result

**One crash, found in the first run.** Not memory unsafety: a Rust panic,
reached through ordinary — not exotic — provider output.

```
thread panicked at qwertty-term-vt-0.4.0/src/stream.rs:2432:19:
end byte index 1024 is not a char boundary; it is inside '漢' (bytes 1022..1025)
```

### The defect

`TerminalHandler::window_title` truncates a title at 1024 bytes with a raw
string slice:

```rust
fn window_title(&mut self, title: &str) {
    const MAX: usize = 1024;
    let t = if title.len() > MAX { &title[..MAX] } else { title };
    self.terminal.set_title(t.as_bytes());
}
```

`&title[..1024]` panics whenever byte 1024 falls inside a multi-byte
character. Any child that sets a window title longer than 1024 bytes
containing anything but ASCII reaches it — an agent reporting a long path, a
prompt with an emoji, a CJK filename. This is not a hostile-input-only
defect; it is a defect a normal session can hit.

`report_pwd` (same file, `MAX = 4096`) has the identical shape. **It was not
reproduced**: OSC payloads are bounded at `MAX_BUF = 2048` before dispatch,
and no input reached the slice. Recorded as a source observation, not a
finding — the difference matters, and a later change to that bound would make
it real.

### Minimized reproducer

Machine minimization left a 500-byte input still carrying unrelated noise, so
the reproducer in the corpus is hand-written from the understood root cause
and is exactly the shape of the defect:

`crates/corrald/tests/corpus/terminal/osc-title-truncation-splits-a-character.bin`
— `OSC 2 ;` followed by 1022 ASCII bytes, a three-byte character straddling
byte 1024, and a terminator.

A second file, `osc-pwd-truncation-splits-a-character.bin`, exercises the
unreproduced pwd path and is kept as an ordinary corpus case with no
poisoning assertion attached to it.

### Disposition

Two things, deliberately separate.

**Upstream owns the fix.** The producer is `qwertty-term-vt`; the repair is
`floor_char_boundary` (or `char_indices`) instead of a raw slice, in both
`window_title` and `report_pwd`. 0.4.0 is the current published release, so
there is nothing to pin. Vendoring this crate the way `portable-pty` was
vendored is **not** taken as an agent decision: that precedent was explicitly
scoped to one small crate and one reviewable hunk, and this crate is 2.6 MB
across 96 files. It is a founder call, recorded as an open question below.

**Corral contains it, and says so.** `AuthoritativeTerminal::consume` catches
the panic and marks the screen poisoned; every reader — snapshot, geometry,
title — then refuses. This is the fail-closed containment AGENTS.md §Scope
discipline permits, with the root-cause follow-up named: Corral never guesses
what the parser meant to do with the bytes that broke it, and never serves a
plausible-looking screen from a structure a panic left half-modified. A
poisoned screen never recovers; later input does not repair it.

What the user sees: the session's process keeps running and its execution
state is unaffected, but its terminal cannot be attached or snapshotted. That
is a real degradation, and it is the honest one.

### What the target asserts now

The first two runs rediscovered the same panic, because `libfuzzer-sys`
prints and aborts on every panic before Corral's `catch_unwind` can see it.
Left alone, the nightly job would fail every night on a defect that is
already known, contained, and recorded — which trains people to ignore a
red job, the worst outcome available.

So the target checks **Corral's** contract rather than upstream's: it
silences the panic hook, and asserts that a screen marked poisoned answers
nothing — no snapshot, no geometry, no title. Silencing does not weaken it.
`libfuzzer-sys` still wraps the body in its own `catch_unwind` and aborts on
anything that escapes, so a panic Corral does *not* contain is still a crash.
What changed is that a panic Corral *does* contain became a property under
test instead of a run thrown away.

The third run, after that change, executed 51,956 inputs in 601 seconds and found nothing.

## Open for the founder

1. **Vendor `qwertty-term-vt` and patch the two slices, or wait for
   upstream?** Vendoring makes sessions survive long non-ASCII titles now, at
   the cost of carrying 2.6 MB across 96 files — a different proposition from
   the one-hunk `portable-pty` precedent. Waiting keeps the tree small and
   leaves a reachable session-losing defect behind the containment.

   A third option was examined and is **not** recommended: Corral could
   implement the `Handler` trait itself, delegating all 83 methods to the
   upstream handler and overriding only the two broken ones. Every method has
   a default no-op body, so a delegation left out — by a mistake now, or by
   upstream adding a method later — would silently drop terminal behaviour
   rather than fail to compile. That is a worse failure mode than either
   carrying the source or waiting.
2. **Does the containment need a user-visible surface?** Today a poisoned
   terminal simply refuses. Whether a person is told *why* their session's
   screen went away belongs to the phase that owns attention and surfacing.

## The containment's boundary moved after this campaign

The campaign reached the emulator through one door — feeding it bytes — and
the containment was written around that call. The session-lifetime design pass
found that reflow and snapshot serialization walk the same packed pages with
the same consequence, and drew the boundary around the screen instead
(ADR 0007 L5): one `contain` that every entrance goes through, poisoning on a
panic from any of them.

**Verification gap, stated rather than papered over.** The two entrances added
to the containment have no deterministic test that they *become* poisoned,
because the only known reproducer — the OSC title truncation above — is
reachable through the parser alone. Fault injection would mean test-only
behaviour in the daemon, which `AGENTS.md` §Scope discipline rules out. What
is covered: every entrance refuses an already-poisoned screen, the corpus
reflows and serializes every case, and the nightly campaign continues to feed
the parser. A reproducer that panics reflow or serialization is a corpus entry
the moment one exists.

## What this campaign did not cover

Linux (the corpus suite runs there per PR; this campaign ran on macOS only),
sustained multi-hour runs, MemorySanitizer or ThreadSanitizer, concurrent
sessions sharing a daemon, and the PTY layer itself — the target exercises the
emulator, not `portable-pty`, whose own compatibility evidence is in
`docs/decisions/2026-08-24-pr3-spawn-gate.md`. Nightly deep runs continue
under `scripts/fuzz-terminal`; findings there are distilled into the corpus
that every PR clears.
