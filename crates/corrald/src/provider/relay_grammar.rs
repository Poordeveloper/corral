//! Recognizing a Corral-owned entry by the invocation it carries.
//!
//! Ownership at global scope is structural: the relay invocation *is* the
//! owner identity (ADR 0013 D2). This module is the only place that decides
//! whether a command line is Corral's, so the writer and the recognizer cannot
//! drift apart into "what Corral writes" and "what Corral will delete".
//!
//! Recognition is deliberately narrow. It reads the invocation's leading
//! words — the program's own file name and the relay subcommand — and nothing
//! else. Not position in an array, not a comment, not resemblance, and not the
//! fail-open guard: a third party's `… || true` hook is not Corral's, and
//! treating the guard as evidence would let Corral delete somebody else's
//! entry (grill Q1′).

use corral_protocol::hook::{RELAY_INTEGRATION_VERSION_FLAG, RELAY_SUBCOMMAND};

use super::launch::CLIENT_BINARY;

/// Whether this shell command line invokes Corral's relay.
pub fn invokes_corral_relay(command: &str) -> bool {
    words_invoke_corral_relay(&split(command))
}

/// The integration version this shell command line declares, if any.
pub fn declared_version(command: &str) -> Option<u32> {
    version_in(&split(command))
}

/// Whether these argv words invoke Corral's relay.
///
/// The form Codex's `notify` carries: an array of words, already split by the
/// document that holds them.
pub fn words_invoke_corral_relay(words: &[String]) -> bool {
    let mut words = words.iter();
    let Some(program) = words.next() else {
        return false;
    };
    if file_name(program) != CLIENT_BINARY {
        return false;
    }
    words.next().is_some_and(|word| word == RELAY_SUBCOMMAND)
}

/// The integration version these argv words declare, if any.
///
/// Absence is meaningful and is not an error: an entry written before the
/// discriminant existed carries none, and this binary understands everything
/// it wrote back then.
pub fn version_in(words: &[String]) -> Option<u32> {
    words
        .iter()
        .position(|word| word == RELAY_INTEGRATION_VERSION_FLAG)
        .and_then(|flag| words.get(flag + 1))
        .and_then(|value| value.parse().ok())
}

/// The last path component of a program word.
fn file_name(program: &str) -> &str {
    program.rsplit('/').next().unwrap_or(program)
}

/// Split a command line into the words a shell would pass.
///
/// It reads back what `RelayInvocation::shell_command` writes — single-quoted
/// words with `'\''` for an embedded quote — and tolerates the unquoted words
/// a person's own hook is written with. It is a recognizer for Corral's own
/// output, not a shell: a command it splits differently from `/bin/sh` is a
/// command Corral does not claim, which is the safe direction.
fn split(command: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut started = false;
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for character in command.chars() {
        // Outside quotes a backslash makes the next character literal, which
        // is how a single quote survives inside a single-quoted word: the
        // writer emits `'\''` and this is the half that reads it back.
        if escaped {
            started = true;
            word.push(character);
            escaped = false;
            continue;
        }
        match (quote, character) {
            (Some(open), c) if c == open => quote = None,
            (Some(_), c) => word.push(c),
            (None, '\\') => escaped = true,
            (None, '\'' | '"') => {
                started = true;
                quote = Some(character);
            }
            (None, c) if c.is_whitespace() => {
                if started {
                    words.push(std::mem::take(&mut word));
                    started = false;
                }
            }
            (None, c) => {
                started = true;
                word.push(c);
            }
        }
    }
    if started {
        words.push(word);
    }
    words
}

#[cfg(test)]
#[path = "relay_grammar_tests.rs"]
mod tests;
