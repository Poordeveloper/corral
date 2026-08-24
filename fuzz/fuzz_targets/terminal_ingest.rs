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

use corrald::runtime::{AuthoritativeTerminal, PtyGeometry, encode};
use libfuzzer_sys::fuzz_target;

/// libfuzzer-sys prints and aborts on every panic, which would make this
/// target rediscover a contained upstream defect forever
/// (`docs/evidence/pr3-terminal-fuzz-2026-08-24.md`). Silencing the hook does
/// not weaken the target: libfuzzer-sys still wraps the body in its own
/// `catch_unwind` and aborts on anything that escapes, so a panic Corral does
/// *not* contain is still a crash. What changes is that a panic Corral *does*
/// contain becomes a property to check rather than a run to throw away.
fn quiet_panics() {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    ONCE.call_once(|| std::panic::set_hook(Box::new(|_| {})));
}

fuzz_target!(|data: &[u8]| {
    quiet_panics();

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
    for piece in payload.chunks(chunk) {
        // Device replies are dropped here: what they are is the daemon's
        // business, that producing them does not crash is this target's.
        let _reply = terminal.consume(piece);
    }

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
