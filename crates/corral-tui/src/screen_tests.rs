use super::*;

const SMALL: Geometry = Geometry { rows: 4, cols: 10 };

fn drawn(frame: Frame) -> String {
    String::from_utf8(frame.bytes).expect("the frame is text and escape bytes")
}

/// A frame never draws past the last row. One extra line scrolls the whole
/// screen up by one, which on a list repainted every second is a screen that
/// crawls.
#[test]
fn a_frame_stops_at_the_last_row() {
    let mut frame = Frame::new(SMALL);

    for _ in 0..10 {
        frame.line(Emphasis::Plain, "a row");
    }

    assert_eq!(frame.drawn, SMALL.rows);
    assert_eq!(frame.remaining(), 0);
    assert_eq!(drawn(frame).matches("a row").count(), 4);
}

/// Truncation cuts on a character boundary. Cutting a multi-byte character in
/// half would put a replacement character on the screen — or, for a terminal
/// reading bytes, nothing recognisable at all.
#[test]
fn a_line_wider_than_the_terminal_is_cut_between_characters() {
    let mut frame = Frame::new(SMALL);

    frame.line(Emphasis::Plain, "ééééééééééééé");

    let text = drawn(frame);
    assert!(text.contains(&"é".repeat(10)), "{text:?}");
    assert!(!text.contains(&"é".repeat(11)), "{text:?}");
}

/// Raw mode is on while the list is up, so a newline alone moves down without
/// returning to the first column and every row would start further right than
/// the last.
#[test]
fn every_line_returns_to_the_first_column() {
    let mut frame = Frame::new(SMALL);

    frame.line(Emphasis::Plain, "one");
    frame.line(Emphasis::Plain, "two");

    let text = drawn(frame);
    assert_eq!(
        text.matches('\n').count(),
        text.matches("\r\n").count(),
        "{text:?}"
    );
}

/// A frame that fills the screen ends on the last row and not one line past
/// it.
///
/// The newline after a last line lands on the bottom margin, which scrolls the
/// frame up by one: the heading leaves the screen a moment after it is drawn,
/// and it is redrawn and lost again every second. Every full frame this list
/// draws has that shape — `draw` pads to exactly the last row — so the rule
/// belongs here, where no caller can get it wrong.
#[test]
fn a_full_frame_does_not_end_in_a_newline() {
    let mut frame = Frame::new(SMALL);

    for _ in 0..SMALL.rows {
        frame.line(Emphasis::Plain, "a row");
    }

    let text = drawn(frame);
    assert!(!text.ends_with("\r\n"), "{text:?}");
    assert_eq!(
        text.matches("\r\n").count(),
        usize::from(SMALL.rows) - 1,
        "{text:?}"
    );
}

/// The prompt is the last thing drawn and leaves the cursor in it, so a person
/// can see where what they type is going.
#[test]
fn the_prompt_shows_the_cursor_and_does_not_move_off_its_line() {
    let mut frame = Frame::new(SMALL);

    frame.prompt("run: sh");

    let text = drawn(frame);
    assert!(text.ends_with(SHOW_CURSOR), "{text:?}");
    assert!(!text.ends_with("\r\n"), "{text:?}");
}

/// Every redraw starts from a clean screen, and the cursor is hidden while the
/// list is being painted rather than skittering across it.
#[test]
fn a_frame_clears_before_it_draws() {
    let text = drawn(Frame::new(SMALL));

    assert!(text.starts_with(HIDE_CURSOR), "{text:?}");
    assert!(text.contains(HOME_AND_CLEAR), "{text:?}");
}

/// Rows spoken for stay spoken for. A body bigger than the screen takes the
/// blank rows above the footer, never the footer's own — and never the
/// prompt's, which is the only line that shows the cursor.
#[test]
fn a_reserved_row_is_not_taken_by_the_body() {
    let mut frame = Frame::new(SMALL);
    frame.reserve(1);

    for _ in 0..10 {
        frame.line(Emphasis::Plain, "a row");
    }
    assert_eq!(frame.remaining(), 0);
    assert_eq!(frame.drawn, SMALL.rows - 1);

    frame.reserve(0);
    frame.prompt("new session: ");

    let text = drawn(frame);
    assert!(text.ends_with(SHOW_CURSOR), "{text:?}");
}

/// A title is the file name of a program somebody chose, and a file name may
/// carry a newline or an escape byte. Drawn as it arrived it would move the
/// cursor rows this frame is not counting, or leave a colour behind for the
/// next line to inherit.
#[test]
fn text_from_elsewhere_cannot_move_the_cursor() {
    let mut frame = Frame::new(SMALL);

    frame.line(Emphasis::Plain, "a\r\n\x1b[2Jb");

    let lines = frame.drawn;
    let text = drawn(frame);
    assert_eq!(lines, 1);
    assert_eq!(text.matches("\r\n").count(), 0, "{text:?}");
    // The frame clears once, at the top. A second one came out of the text.
    assert_eq!(text.matches(HOME_AND_CLEAR).count(), 1, "{text:?}");
}
