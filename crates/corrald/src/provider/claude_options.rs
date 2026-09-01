//! The verified command line of one Claude Code version family.
//!
//! **Not "Claude's grammar".** It is what one measured build's parser does,
//! bound to `VERIFIED_AGAINST` and to the row this provider holds in
//! `PRODUCT.md` §10's supported provider/version matrix. A newer executable
//! does not extend it: an option it adds is unknown here, and unknown refuses
//! the launch (ADR 0012 D1). The way that is repaired is the pipeline below,
//! not a build that happens to be newer.
//!
//! Evidence, not policy. What each entry *means* — which options are refused
//! and why — belongs to `super::claude`; this module only says what the
//! provider's parser does with a word, for one version family, because that
//! was measured. ADR 0012 D4 fixes the direction it may be produced in:
//!
//! ```text
//! spike → verified inventory → this file → tests → the version matrix
//! ```
//!
//! Never the other way round. Corral does not inspect a provider binary, probe
//! its parser, or infer arity from error wording while starting a session; a
//! provider's internals are not a runtime dependency of the control plane.
//!
//! Measured against **2.1.251** by asking the root parser itself, with standard
//! input closed so nothing reached a model: a required value with none supplied
//! answers `argument missing`, a name it does not know answers `unknown
//! option`, and anything else is a root option that takes no word from after
//! it. Candidates came from `strings` over the installed binary rather than
//! from `--help`, which lists neither the 40 required-value options below that
//! it omits nor the eight valueless ones
//! (`docs/references/2026-09-01-claude-2.1.251-attachment-matrix.md`,
//! scenario 10).
//!
//! A gap here is a false rejection, never a wrong attach: an option in neither
//! list is unknown, and an unknown option refuses the launch (ADR 0012 D1).

/// The build these lists were measured against.
///
/// Held as a value rather than only as prose so that the binding to a version
/// is a fact in the code, and so a person reading a refusal in a log can see
/// what Corral was holding the command line against. Corral does **not** ask
/// the installed executable for its version and does not gate a launch on it:
/// a newer Claude runs, and only an option this file has never seen is
/// refused.
pub const VERIFIED_AGAINST: &str = "2.1.251";

/// The options that take the next word as their value, whatever it looks like.
///
/// Commander is greedy for a required value where clap is not, which is what
/// makes this list load-bearing for reading a command line rather than merely
/// for tidiness: `--name -- --continue` hands the terminator to `--name`, and
/// `--continue` behind it parses as an option.
pub const REQUIRED_VALUE_FLAGS: [&str; 69] = [
    "--add-dir",
    "--advisor",
    "--agent",
    "--agent-color",
    "--agent-id",
    "--agent-name",
    "--agent-type",
    "--agents",
    "--allowed-tools",
    "--append-subagent-system-prompt",
    "--append-system-prompt",
    "--append-system-prompt-file",
    "--autocompact",
    "--betas",
    "--channels",
    "--correlation-id",
    "--dangerously-load-development-channels",
    "--debug-file",
    "--deep-link-cwd-b64",
    "--deep-link-last-fetch",
    "--deep-link-repo",
    "--disallowed-tools",
    "--effort",
    "--environment",
    "--fallback-model",
    "--file",
    "--forward-home-settings",
    "--input-format",
    "--json-schema",
    "--managed-settings",
    "--max-budget-usd",
    "--max-thinking-tokens",
    "--max-turns",
    "--mcp-config",
    "--messaging-socket-path",
    "--model",
    "--name",
    "--on-branch",
    "--output-format",
    "--parent-session-id",
    "--permission-mode",
    "--permission-prompt-tool",
    "--plan-mode-instructions",
    "--plugin-dir",
    "--plugin-dir-no-mcp",
    "--plugin-url",
    "--pool",
    "--prefill",
    "--prefill-b64",
    "--ref",
    "--remote-control-session-name-prefix",
    "--resume-drops-turn",
    "--resume-session-at",
    "--rewind-files",
    "--sdk-url",
    "--session-id",
    "--setting-sources",
    "--settings",
    "--system-prompt",
    "--system-prompt-file",
    "--task-budget",
    "--team-name",
    "--teammate-mode",
    "--thinking",
    "--thinking-display",
    "--tools",
    "--watch-artifact",
    "--watch-artifact-no-autoreact",
    "--workload",
];

/// The root options that take no word from after them.
///
/// Booleans and optional-value options together, because for reading a command
/// line they behave the same way: measured, an optional value never takes a
/// dash-leading word — `-d -c` continues, and `--debug -- --continue` leaves
/// the terminator standing — and a following ordinary word is a positional
/// this grammar already allows.
pub const VALUELESS_FLAGS: [&str; 43] = [
    "--allow-dangerously-skip-permissions",
    "--ax-screen-reader",
    "--background",
    "--bare",
    "--bg",
    "--brief",
    "--chrome",
    "--cloud",
    "--continue",
    "--dangerously-skip-permissions",
    "--debug",
    "--debug-to-stderr",
    "--deep-link-origin",
    "--disable-slash-commands",
    "--enable-auto-mode",
    "--exclude-dynamic-system-prompt-sections",
    "--fork-session",
    "--forward-subagent-text",
    "--from-pr",
    "--help",
    "--ide",
    "--include-hook-events",
    "--include-partial-messages",
    "--init",
    "--init-only",
    "--maintenance",
    "--no-chrome",
    "--no-session-persistence",
    "--print",
    "--prompt-suggestions",
    "--remote",
    "--remote-control",
    "--replay-user-messages",
    "--reply-on-resume",
    "--restricted",
    "--resume",
    "--safe-mode",
    "--strict-mcp-config",
    "--teleport",
    "--tmux",
    "--verbose",
    "--version",
    "--worktree",
];

/// Every short flag this version defines.
///
/// A cluster holding a letter outside this set is one this build cannot read.
pub const KNOWN_SHORTS: [char; 8] = ['c', 'd', 'h', 'n', 'p', 'r', 'v', 'w'];

/// The short flags that take a value, which is the rest of their cluster when
/// there is one.
pub const VALUE_SHORTS: [char; 4] = ['n', 'd', 'w', 'r'];

/// The short flag whose value is required. `-n` is `--name`; `-d`, `-r`, and
/// `-w` take theirs optionally.
pub const REQUIRED_VALUE_SHORTS: [char; 1] = ['n'];
