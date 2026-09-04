//! The snapshot contract, owned by a test: the screen a client rebuilds from
//! a snapshot is the screen the daemon holds (ADR 0003 D1/D3; grill Q6/Q11).
//!
//! Twenty-four scenarios, twenty-two distilled from the PR9 spike plus two for
//! origin mode, each fed in two halves:
//! the snapshot is minted between them, so the second half proves that
//! deltas after a snapshot land where the daemon's own deltas land. Compared
//! semantically, never as bytes. Nothing Q7 has not accepted is asserted:
//! no primary screen behind an active alternate, no palette, no geometry
//! transport.

use qwertty_term_vt::modes::Mode;
use qwertty_term_vt::snapshot::SnapshotRow;
use qwertty_term_vt::terminal::{ScreenKey, ScrollingRegion, Terminal};

use super::super::PtyGeometry;
use super::super::terminal::AuthoritativeTerminal;
use super::encode;

const GEOMETRY: PtyGeometry = PtyGeometry::expect_valid(24, 80);

/// Everything ADR 0003 D3 says a snapshot carries, read from a terminal.
#[derive(Debug, PartialEq, Eq)]
struct Fidelity {
    rows: usize,
    cols: usize,
    visible: Vec<SnapshotRow>,
    cursor: (usize, usize, bool),
    active_screen: ScreenKey,
    scrolling_region: ScrollingRegion,
    tabstops: Vec<usize>,
    title: Vec<u8>,
    cursor_keys: bool,
    bracketed_paste: bool,
}

fn fidelity(terminal: &Terminal) -> Fidelity {
    let snapshot = terminal.snapshot();
    Fidelity {
        rows: snapshot.rows,
        cols: snapshot.cols,
        visible: snapshot.visible_window(0).to_vec(),
        cursor: (
            snapshot.cursor.row,
            snapshot.cursor.col,
            snapshot.cursor.visible,
        ),
        active_screen: terminal.screens.active_key(),
        scrolling_region: terminal.scrolling_region,
        tabstops: (0..usize::from(terminal.cols))
            .filter(|col| terminal.tabstops.get(*col))
            .collect(),
        title: terminal.title.clone(),
        cursor_keys: terminal.modes.get(Mode::CursorKeys),
        bracketed_paste: terminal.modes.get(Mode::BracketedPaste),
    }
}

/// The last `n` history rows and how many there are.
fn history(terminal: &Terminal, n: usize) -> (usize, Vec<SnapshotRow>) {
    let snapshot = terminal.snapshot();
    let len = snapshot.scrollback_len();
    (len, snapshot.all_rows[len.saturating_sub(n)..len].to_vec())
}

struct Scenario {
    name: &'static str,
    first: Vec<u8>,
    second: Vec<u8>,
    /// Applied to the authoritative screen after `first`, before the snapshot.
    resize_to: Option<PtyGeometry>,
}

fn scenario(name: &'static str, first: impl Into<Vec<u8>>, second: impl Into<Vec<u8>>) -> Scenario {
    Scenario {
        name,
        first: first.into(),
        second: second.into(),
        resize_to: None,
    }
}

fn lines(prefix: &str, count: usize) -> String {
    (1..=count).map(|i| format!("{prefix} {i}\r\n")).collect()
}

fn scenarios() -> Vec<Scenario> {
    vec![
        scenario(
            "text, wrap",
            format!("hello world\r\n{}\r\n", "x".repeat(100)),
            "after\r\n",
        ),
        scenario(
            "16 colours",
            "\x1b[31mred\x1b[42mgreenbg\x1b[0m \x1b[93mbright\x1b[0m",
            "\x1b[35;44mmore\x1b[0m",
        ),
        scenario(
            "256 colours",
            "\x1b[38;5;202mo\x1b[48;5;27mb\x1b[0m",
            "\x1b[38;5;117mz\x1b[0m",
        ),
        scenario(
            "truecolour",
            "\x1b[38;2;10;20;30mT\x1b[48;2;200;100;0mU\x1b[0m",
            "\x1b[38;2;1;2;3mV\x1b[0m",
        ),
        scenario(
            "bold dim italic underline inverse",
            "\x1b[1mB\x1b[0m\x1b[2mD\x1b[0m\x1b[3mI\x1b[0m\x1b[4mU\x1b[0m\x1b[7mR\x1b[0m\x1b[1;4;7mX\x1b[0m",
            "\x1b[1;3mY\x1b[0m",
        ),
        scenario("strikethrough", "\x1b[9mS\x1b[0m", "\x1b[9mT\x1b[0m"),
        scenario("cursor position", "abc\x1b[10;20H", "\x1b[5;5Hq"),
        scenario("cursor hidden", "abc\x1b[?25l", "z"),
        // The snapshot is taken inside the alternate screen; what happens
        // after `?1049l` is Q7's (S3) and is not asserted here.
        scenario(
            "alternate screen",
            "main1\r\nmain2\r\n\x1b[?1049h\x1b[H\x1b[2Jalt content\x1b[3;3Hmore",
            "\x1b[4;1Hdelta on alt",
        ),
        scenario(
            "OSC title",
            "\x1b]0;spike title\x07text",
            "\x1b]2;second title\x07",
        ),
        scenario(
            "OSC colour (cells by index; the palette is D4's)",
            "\x1b]4;1;rgb:12/34/56\x07\x1b[31mrepaletted\x1b[0m",
            "\x1b[31mx\x1b[0m",
        ),
        scenario("wide CJK", "中文字\r\n汉字", "更多"),
        scenario("emoji", "a🦀b", "🎉"),
        scenario("combining marks", "e\u{0301}x", "n\u{0303}"),
        scenario("scrollback", lines("line", 400), "line 401\r\n"),
        scenario(
            "erase, redraw, with history and a blank tail",
            format!(
                "{}\x1b[2J\x1b[Habc\x1b[Kd\x1b[3;1Hxyz\x1b[1K",
                lines("fill", 30)
            ),
            "\x1b[5;1HQ\x1b[J",
        ),
        scenario(
            "scroll region",
            format!("\x1b[5;10r{}", lines("r", 20)),
            "\x1b[10;1H\r\nnew\r\n",
        ),
        scenario("tabs", "a\tb\tc\r\n", "\x1b[3g\x1b[1;5H\x1bHZ\r\n\tW"),
        scenario(
            "insert/delete line and char",
            format!(
                "{}\x1b[3;1H\x1b[2L\x1b[6;1H\x1b[M\x1b[1;3H\x1b[2@\x1b[2;2H\x1b[P",
                lines("l", 10)
            ),
            "\x1b[4;1H\x1b[L",
        ),
        Scenario {
            name: "resize opens an epoch",
            first: format!("{}{}\r\n", lines("row", 10), "w".repeat(150)).into_bytes(),
            second: b"post\r\n".to_vec(),
            resize_to: Some(PtyGeometry::expect_valid(30, 100)),
        },
        scenario(
            "modes: DECCKM, bracketed paste",
            "\x1b[?1h\x1b[?2004htext",
            "z",
        ),
        scenario(
            "cursor after a blank tail",
            format!("{}\x1b[2J\x1b[12;40H", lines("fill", 40)),
            "here",
        ),
        scenario(
            "origin mode inside a scroll region, with history",
            format!("{}\x1b[5;10r\x1b[?6h\x1b[2;3Hin region", lines("fill", 30)),
            "\r\nnext",
        ),
        scenario(
            "left and right margins with origin mode",
            format!(
                "{}\x1b[?69h\x1b[10;60s\x1b[5;10r\x1b[?6h\x1b[1;1Hboxed",
                lines("fill", 30)
            ),
            "\r\nmore",
        ),
    ]
}

/// What every client does with a snapshot: clear, then replay.
fn replica_of(payload: &[u8], geometry: PtyGeometry) -> AuthoritativeTerminal {
    let mut replica = AuthoritativeTerminal::new(geometry);
    let _ = replica.consume(b"\x1b[H\x1b[2J");
    let _ = replica.consume(payload);
    replica
}

#[test]
fn a_client_rebuilds_the_daemons_screen_from_its_snapshot_and_follows_its_deltas() {
    let mut failures = Vec::new();
    for scenario in scenarios() {
        let mut authoritative = AuthoritativeTerminal::new(GEOMETRY);
        let _ = authoritative.consume(&scenario.first);
        let geometry = scenario.resize_to.unwrap_or(GEOMETRY);
        if let Some(geometry) = scenario.resize_to {
            authoritative.resize(geometry);
        }
        let snapshot = encode(&authoritative).expect("the screen encodes");
        let mut replica = replica_of(snapshot.payload(), geometry);

        let expected = fidelity(authoritative.terminal().expect("not poisoned"));
        let rebuilt = fidelity(replica.terminal().expect("not poisoned"));
        if expected != rebuilt {
            failures.push(format!(
                "{}: after the snapshot\n{}",
                scenario.name,
                diff(&expected, &rebuilt)
            ));
        }

        let _ = authoritative.consume(&scenario.second);
        let _ = replica.consume(&scenario.second);
        let expected = fidelity(authoritative.terminal().expect("not poisoned"));
        let rebuilt = fidelity(replica.terminal().expect("not poisoned"));
        if expected != rebuilt {
            failures.push(format!(
                "{}: after the delta\n{}",
                scenario.name,
                diff(&expected, &rebuilt)
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}

/// History rides in the snapshot up to its row budget, and lands in history:
/// the client's scrollback holds exactly what the snapshot declared.
#[test]
fn history_carried_by_a_snapshot_lands_in_the_clients_history() {
    for (name, first) in [
        ("400 lines", lines("line", 400)),
        (
            "history behind a blank screen",
            format!("{}\x1b[2J\x1b[H", lines("fill", 30)),
        ),
        (
            "history behind a half-filled screen",
            format!("{}\x1b[2J\x1b[Hone\r\ntwo\r\n", lines("fill", 30)),
        ),
    ] {
        let mut authoritative = AuthoritativeTerminal::new(GEOMETRY);
        let _ = authoritative.consume(first.as_bytes());
        let snapshot = encode(&authoritative).expect("the screen encodes");
        let replica = replica_of(snapshot.payload(), GEOMETRY);

        let (expected_len, expected_tail) = history(authoritative.terminal().unwrap(), 5);
        let (rebuilt_len, rebuilt_tail) = history(replica.terminal().unwrap(), 5);
        assert_eq!(
            rebuilt_len,
            snapshot.included_scrollback_rows(),
            "{name}: the client holds a different amount of history than the snapshot declared"
        );
        assert_eq!(rebuilt_len, expected_len, "{name}: history length");
        assert_eq!(
            rebuilt_tail, expected_tail,
            "{name}: the last five history rows"
        );
    }
}

fn diff(expected: &Fidelity, rebuilt: &Fidelity) -> String {
    let mut out = Vec::new();
    if (expected.rows, expected.cols) != (rebuilt.rows, rebuilt.cols) {
        out.push(format!(
            "geometry {}x{} vs {}x{}",
            expected.rows, expected.cols, rebuilt.rows, rebuilt.cols
        ));
    }
    for (r, (a, b)) in expected.visible.iter().zip(&rebuilt.visible).enumerate() {
        if a != b {
            let c = a
                .cells
                .iter()
                .zip(&b.cells)
                .position(|(x, y)| x != y)
                .unwrap_or(0);
            out.push(format!(
                "row {r} col {c}: expected {:?} got {:?}",
                a.cells.get(c).map(|cell| (cell.ch, cell.width, cell.style)),
                b.cells.get(c).map(|cell| (cell.ch, cell.width, cell.style))
            ));
            break;
        }
    }
    if expected.cursor != rebuilt.cursor {
        out.push(format!(
            "cursor {:?} vs {:?}",
            expected.cursor, rebuilt.cursor
        ));
    }
    if expected.active_screen != rebuilt.active_screen {
        out.push(format!(
            "active screen {:?} vs {:?}",
            expected.active_screen, rebuilt.active_screen
        ));
    }
    if expected.scrolling_region != rebuilt.scrolling_region {
        out.push(format!(
            "scrolling region {:?} vs {:?}",
            expected.scrolling_region, rebuilt.scrolling_region
        ));
    }
    if expected.tabstops != rebuilt.tabstops {
        out.push(format!(
            "tabstops {:?} vs {:?}",
            expected.tabstops, rebuilt.tabstops
        ));
    }
    if expected.title != rebuilt.title {
        out.push(format!(
            "title {:?} vs {:?}",
            String::from_utf8_lossy(&expected.title),
            String::from_utf8_lossy(&rebuilt.title)
        ));
    }
    if (expected.cursor_keys, expected.bracketed_paste)
        != (rebuilt.cursor_keys, rebuilt.bracketed_paste)
    {
        out.push("keyboard modes differ".to_owned());
    }
    out.join("\n")
}
