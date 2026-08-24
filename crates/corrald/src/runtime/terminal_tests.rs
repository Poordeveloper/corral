use super::*;

const GEOMETRY: PtyGeometry = PtyGeometry::expect_valid(24, 80);

fn terminal() -> AuthoritativeTerminal {
    AuthoritativeTerminal::new(GEOMETRY)
}

/// An agent that asks what terminal it is talking to blocks until it is
/// answered. Nobody being attached is not an answer, so the daemon owes the
/// reply itself (`ARCHITECTURE.md` §3).
#[test]
fn a_device_attributes_query_is_answered_with_no_client_attached() {
    let mut terminal = terminal();

    let reply = terminal.consume(b"\x1b[c");

    assert!(
        !reply.is_empty(),
        "primary DA went unanswered; an unattached agent would wait forever"
    );
    assert!(
        reply.as_bytes().starts_with(b"\x1b["),
        "the reply is a control sequence, got {:?}",
        String::from_utf8_lossy(reply.as_bytes())
    );
}

/// Cursor position reports carry the position the daemon holds, which is the
/// point of the daemon holding it.
#[test]
fn a_cursor_position_report_answers_from_the_daemons_own_screen() {
    let mut terminal = terminal();

    let _ = terminal.consume(b"\x1b[5;9H");
    let reply = terminal.consume(b"\x1b[6n");

    assert_eq!(
        reply.as_bytes(),
        b"\x1b[5;9R",
        "got {:?}",
        String::from_utf8_lossy(reply.as_bytes())
    );
}

/// Ordinary output is not a reply. Echoing it into the PTY would put the
/// child's own text back into its input.
#[test]
fn ordinary_output_produces_no_reply() {
    let mut terminal = terminal();

    let reply = terminal.consume(b"hello, world\r\n");

    assert!(reply.is_empty());
}

/// The emulator tracks the title but its serializer does not re-emit it, so a
/// snapshot has to carry it deliberately (ADR 0003 D3). That starts with the
/// title being readable at all.
#[test]
fn the_window_title_the_child_set_is_readable() {
    let mut terminal = terminal();

    let _ = terminal.consume(b"\x1b]2;deploying\x07");

    assert_eq!(terminal.title(), Some(b"deploying".as_slice()));
}

#[test]
fn a_terminal_whose_child_set_no_title_reports_none() {
    let mut terminal = terminal();

    let _ = terminal.consume(b"working\r\n");

    assert_eq!(terminal.title(), None);
}

#[test]
fn resize_moves_the_authoritative_geometry() {
    let mut terminal = terminal();
    assert_eq!(terminal.geometry(), Some(GEOMETRY));

    terminal.resize(PtyGeometry::expect_valid(40, 120));

    assert_eq!(
        terminal.geometry(),
        Some(PtyGeometry::expect_valid(40, 120))
    );
}

/// Retention is a memory budget in bytes, not a row count — the emulator's
/// page model is Ghostty's. A future change that reads this number as rows
/// would silently retain a thousandth of what it meant to (spike S1's trap).
#[test]
fn retention_is_a_byte_budget_the_emulator_was_told_about() {
    let terminal = terminal();

    assert_eq!(
        terminal
            .terminal()
            .expect("a readable screen")
            .screens
            .active()
            .pages
            .explicit_max_size(),
        RETAINED_SCROLLBACK_BYTES,
        "the emulator holds the byte budget Corral set, not a row count"
    );
}

/// Scrollback beyond the viewport is retained, which is what makes a snapshot
/// able to carry history at all (ADR 0003 D7).
#[test]
fn output_past_the_viewport_becomes_retained_history() {
    let mut terminal = terminal();

    for line in 0..200 {
        let _ = terminal.consume(format!("line {line}\r\n").as_bytes());
    }

    let retained = terminal
        .terminal()
        .expect("a readable screen")
        .screens
        .active()
        .pages
        .total_rows();
    assert!(
        retained > usize::from(GEOMETRY.rows()),
        "200 lines through a {}-row screen retained only {retained} rows",
        GEOMETRY.rows()
    );
}
