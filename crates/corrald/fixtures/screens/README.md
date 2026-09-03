# Provider screen and event captures

Real provider sessions driven on a real PTY, recorded first-party for the
PR8 attention matrix (`docs/references/2026-09-02-pr8-attention-matrix.md`).
They exist so a screen rule is proven against what the provider actually
drew, rendered by the emulator `corrald` owns, rather than against a
screenshot or a shape a test author imagined (AGENTS.md §Tests, ADR 0015
D6).

Layout: `<provider>/<provider version>/<scenario>/`, each holding

| File | Content |
|---|---|
| `stream.bin` | every byte the provider wrote, framed `<u64 wall-clock ns><u32 len><bytes>` |
| `input.bin` | every byte the driver wrote to the PTY, same framing |
| `marks.jsonl` | the driver's checkpoints: `name`, `t_ns`, byte `offset`, and the fields a checkpoint measured |
| `hooks.jsonl` | Claude hook deliveries: `t_ns`, `argv`, verbatim `stdin` payload |
| `notify.jsonl` | Codex notify deliveries: `t_ns`, `argv` (payload last) |
| `meta.json` | the command line, working directory, geometry, start time, environment |
| `driver.log` | what the driver waited for and when it gave up |

Render the screen at every mark and event with

```text
cargo run -p corrald --example replay_capture -- <scenario dir>
```

Paths, session ids, and prompts inside are the container's, not a
person's; the payloads are exactly what the providers wrote. A capture is
evidence about the version in its path and nothing later (grill Q13).

`claude/2.1.258/` is the first run, 2026-09-02. `claude/2.1.259/` is the
second, 2026-09-03: the provider updated itself between them and removed
the 2.1.258 binary, so those scenarios cannot be re-measured on the version
that produced them, and neither directory speaks for the other. The second
run's driver sets `DISABLE_AUTOUPDATER=1` so a capture's version is a fact
rather than a guess.
