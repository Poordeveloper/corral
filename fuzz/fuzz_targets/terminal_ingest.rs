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

fuzz_target!(|data: &[u8]| {
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

    let mut terminal = AuthoritativeTerminal::new(PtyGeometry { rows, cols });
    for piece in payload.chunks(chunk) {
        // Device replies are dropped here: what they are is the daemon's
        // business, that producing them does not crash is this target's.
        let _reply = terminal.consume(piece);
    }

    // A reflow touches every retained row, which is where a pathological
    // screen becomes unbounded work rather than a wrong screen.
    terminal.resize(PtyGeometry {
        rows: rows.saturating_add(7).min(500),
        cols: cols.saturating_add(13).min(1000),
    });

    let _ = encode(&terminal);
});
