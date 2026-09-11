# Corral

A user-owned control center for coding-agent sessions.

> See every session. Know what needs you. Take control.

Corral treats a Session as the unit of AI work — not a chat transcript, not
a terminal pane. It discovers the Claude Code and Codex sessions already
running on your machine, tells you which ones are blocked on you, and lets
you answer them without hunting through terminals. You keep your own
terminal, editor, and machines.

**Status: pre-release.** M1 is under construction; see `ROADMAP.md` for
what the current phase includes and what it must prove before it ships.

## Install

macOS on Apple Silicon, and Ubuntu 24.04 on x86_64:

```sh
curl -fsSL https://raw.githubusercontent.com/Poordeveloper/corral/main/install.sh | sh
```

The installer places `Corral.app` in `~/Applications` (macOS) or Corral's
executables under `~/.local/share/corral` (Linux), links `corral` into
`~/.local/bin`, and — after saying which — enables Corral's integration
with the Claude Code and Codex installs it finds. Nothing runs in the
background until you use it. `CORRAL_VERSION=vX.Y.Z` installs that release
instead of the latest; every artifact is on the
[releases page](https://github.com/Poordeveloper/corral/releases). On
Linux the CLI and the terminal session list are what is supported; the
Desktop is built and included but not yet validated there.

To remove it: `corral uninstall`. It refuses while Corral still manages a
running session, then takes Corral's entries back out of your agents'
configuration, stops `corrald`, and removes what install placed. Your
`~/.corral` stays unless you pass `--purge`.

Install is not upgrade: to move to a new release, uninstall and install.

## Documentation

| Document | Owns |
|---|---|
| [`PRODUCT.md`](PRODUCT.md) | what Corral is and is not |
| [`ARCHITECTURE.md`](ARCHITECTURE.md) | boundaries, invariants, domain glossary |
| [`ROADMAP.md`](ROADMAP.md) | what the current phase allows |
| [`AGENTS.md`](AGENTS.md) | hard rules for anyone writing code here |
| [`CONTRIBUTING.md`](CONTRIBUTING.md) | how to contribute |
| [`docs/GOVERNANCE.md`](docs/GOVERNANCE.md) | how these documents fit together |

## License

Dual-licensed under [Apache License 2.0](LICENSE-APACHE) or
[MIT](LICENSE-MIT), at your option.
