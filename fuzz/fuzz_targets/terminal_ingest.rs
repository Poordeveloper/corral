//! The path every byte of untrusted provider output takes first.
//!
//! ADR 0003 D1 chose an emulator with 936 `unsafe` blocks concentrated in its
//! page memory layer, and named the cost rather than discovering it later. A
//! `catch_unwind` cannot contain undefined behaviour, so the only evidence
//! that the parser survives hostile input is having tried to break it.
//!
//! The target exercises what a real session does with those bytes: consume
//! them in kernel-shaped chunks, reflow, and serialize — because a crash
//! reachable only through a snapshot of a poisoned screen is still a crash a
//! person meets on resync.

#![no_main]

use std::cell::Cell;

use corrald::runtime::{AuthoritativeTerminal, PtyGeometry, encode};
use libfuzzer_sys::fuzz_target;

thread_local! {
    /// Set only while inside the call Corral contains panics for.
    static INSIDE_CONTAINED_CALL: Cell<bool> = const { Cell::new(false) };
}

/// Silence the panic report *only* for panics Corral contains.
///
/// Without this the target rediscovers a known, contained upstream defect on
/// every run (`docs/evidence/pr3-terminal-fuzz-2026-08-24.md`), and a nightly
/// job that fails every night on the same known thing trains people to ignore
/// it.
///
/// The previous version replaced libfuzzer-sys's hook outright, which was
/// worse than the problem: that hook is what prints the message and backtrace
/// and aborts, so an *uncontained* panic — exactly what this gate exists to
/// catch — would have aborted with no diagnostic at all. The original is kept
/// and delegated to for everything outside the contained call.
fn install_hook() {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let original = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            if INSIDE_CONTAINED_CALL.with(Cell::get) {
                return;
            }
            original(info);
        }));
    });
}

/// Run `work` with contained panics reported quietly.
fn contained<T>(work: impl FnOnce() -> T) -> T {
    INSIDE_CONTAINED_CALL.with(|inside| inside.set(true));
    let outcome = work();
    INSIDE_CONTAINED_CALL.with(|inside| inside.set(false));
    outcome
}

fuzz_target!(|data: &[u8]| {
    install_hook();

    if data.len() < 3 {
        return;
    }

    // The first bytes steer the shape rather than the content: geometry and
    // chunking are part of what a parser meets, and fixing them would leave
    // whole classes of splitting unexercised.
    let rows = 1 + u16::from(data[0] % 60);
    let cols = 1 + u16::from(data[1] % 200);
    let chunk = 1 + usize::from(data[2] % 64);
    let payload = &data[3..];

    // Sizes Corral would refuse are not what this target is about: the daemon
    // never builds one, so a screen that only exists here would be testing an
    // input the product cannot produce.
    let Ok(geometry) = PtyGeometry::new(rows, cols) else {
        return;
    };
    let mut terminal = AuthoritativeTerminal::new(geometry);
    contained(|| {
        for piece in payload.chunks(chunk) {
            // Device replies are dropped here: what they are is the daemon's
            // business, that producing them does not crash is this target's.
            let _reply = terminal.consume(piece);
        }
    });

    // A reflow touches every retained row, which is where a pathological
    // screen becomes unbounded work rather than a wrong screen.
    if let Ok(reflowed) = PtyGeometry::new(
        rows.saturating_add(7).min(500),
        cols.saturating_add(13).min(1000),
    ) {
        terminal.resize(reflowed);
    }

    let snapshot = encode(&terminal);

    // The property Corral owns: a screen whose parser failed is never read
    // from again. Reading a structure a panic left half-modified is unsound,
    // so a poisoned screen that still answered anything would be a real
    // finding — one this target must fail on.
    if terminal.poisoned().is_some() {
        assert!(snapshot.is_err(), "a poisoned screen produced a snapshot");
        assert!(terminal.geometry().is_none(), "a poisoned screen stated a size");
        assert!(terminal.title().is_none(), "a poisoned screen stated a title");
    }
});
