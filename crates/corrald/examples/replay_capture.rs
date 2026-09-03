//! Render a PR8 matrix capture through the emulator `corrald` owns.
//!
//! A capture is one provider session driven on a real PTY: `stream.bin` is
//! every byte the provider wrote, framed as `<u64 wall-clock ns><u32 len>`
//! then the bytes; `marks.jsonl` names the moments the driver marked; and
//! `hooks.jsonl` / `notify.jsonl` hold the provider's own events, verbatim.
//! Replaying the stream into the same emulator the daemon uses is what makes
//! a screen fixture the daemon's screen rather than a screenshot
//! (`docs/references/2026-09-02-pr8-attention-matrix.md`).
//!
//! ```text
//! cargo run -p corrald --example replay_capture -- <capture dir> [mark filter]
//! REPLAY_AT=12.5,40 …   also render at those seconds from the driver's start
//! ```
//!
//! Screens land in `<capture dir>/screens/NN-<mark>.txt`, headed by the mark,
//! its time, the OSC title, and the geometry.
#![forbid(unsafe_code)]

use std::io::Read;

use qwertty_term_vt::stream::{Stream, TerminalHandler};
use qwertty_term_vt::terminal::{Options, Terminal};
use serde_json::{Value, json};

struct Frame {
    t_ns: u64,
    bytes: Vec<u8>,
}

type Error = Box<dyn std::error::Error>;

fn frames(path: &str) -> Result<Vec<Frame>, Error> {
    let mut raw = Vec::new();
    std::fs::File::open(path)?.read_to_end(&mut raw)?;
    let mut out = Vec::new();
    let mut i = 0;
    while i + 12 <= raw.len() {
        let t_ns = u64::from_le_bytes(raw[i..i + 8].try_into()?);
        let len = u32::from_le_bytes(raw[i + 8..i + 12].try_into()?) as usize;
        i += 12;
        let end = (i + len).min(raw.len());
        out.push(Frame {
            t_ns,
            bytes: raw[i..end].to_vec(),
        });
        i = end;
    }
    Ok(out)
}

/// Absent files are empty: a Claude capture has no `notify.jsonl` and a Codex
/// capture no `hooks.jsonl`.
fn json_lines(path: &str) -> Result<Vec<Value>, Error> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Ok(Vec::new());
    };
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).map_err(|e| format!("{path}: {e}").into()))
        .collect()
}

fn dimension(meta: &Value, key: &str) -> Result<u16, Error> {
    let value = meta[key]
        .as_u64()
        .ok_or_else(|| format!("meta.json: {key}"))?;
    Ok(u16::try_from(value)?)
}

fn main() -> Result<(), Error> {
    let dir = std::env::args()
        .nth(1)
        .ok_or("usage: replay_capture <capture dir> [mark filter]")?;
    let filter = std::env::args().nth(2);
    let meta: Value = serde_json::from_str(&std::fs::read_to_string(format!("{dir}/meta.json"))?)?;
    let t0 = meta["t0_ns"].as_u64().ok_or("meta.json: t0_ns")?;
    let (mut rows, mut cols) = (dimension(&meta, "rows")?, dimension(&meta, "cols")?);

    let mut marks: Vec<Value> = json_lines(&format!("{dir}/marks.jsonl"))?
        .into_iter()
        .filter(|m| m["t_ns"].is_u64())
        .collect();
    // The provider's own events are marks too: a keyboard-driving regex can
    // misfire, and the hook or notify delivery still dates the screen.
    for rec in json_lines(&format!("{dir}/hooks.jsonl"))? {
        let name = rec["stdin"]
            .as_str()
            .and_then(|s| serde_json::from_str::<Value>(s).ok())
            .and_then(|d| d["hook_event_name"].as_str().map(str::to_owned))
            .unwrap_or_else(|| "unknown".into());
        marks.push(json!({"name": format!("hook-{name}"), "t_ns": rec["t_ns"]}));
    }
    for rec in json_lines(&format!("{dir}/notify.jsonl"))? {
        let name = rec["argv"]
            .as_array()
            .and_then(|a| a.last())
            .and_then(Value::as_str)
            .and_then(|s| serde_json::from_str::<Value>(s).ok())
            .and_then(|d| d["type"].as_str().map(str::to_owned))
            .unwrap_or_else(|| "notify".into());
        marks.push(json!({"name": format!("hook-{name}"), "t_ns": rec["t_ns"]}));
    }
    if let Ok(spec) = std::env::var("REPLAY_AT") {
        for s in spec.split(',').filter(|s| !s.is_empty()) {
            let secs: f64 = s.parse()?;
            marks.push(json!({"name": format!("at-{s}s"), "t_ns": t0 + (secs * 1e9) as u64}));
        }
    }
    marks.sort_by_key(|m| m["t_ns"].as_u64().unwrap_or(0));

    let frames = frames(&format!("{dir}/stream.bin"))?;
    let mut term = Stream::new(TerminalHandler::new(Terminal::new(Options {
        cols,
        rows,
        max_scrollback: 4 * 1024 * 1024,
        ..Options::default()
    })));
    let out_dir = format!("{dir}/screens");
    std::fs::create_dir_all(&out_dir)?;
    let mut next_frame = 0;
    let mut seq = 0;
    for mark in &marks {
        let t = mark["t_ns"].as_u64().unwrap_or(0);
        let name = mark["name"].as_str().unwrap_or("mark");
        while next_frame < frames.len() && frames[next_frame].t_ns <= t {
            for byte in &frames[next_frame].bytes {
                term.next(*byte);
            }
            let _device_reply = term.handler.take_output();
            next_frame += 1;
        }
        if name == "resize" {
            rows = dimension(mark, "rows").unwrap_or(rows);
            cols = dimension(mark, "cols").unwrap_or(cols);
            term.handler.terminal.resize(cols, rows);
            continue;
        }
        if filter.as_ref().is_some_and(|f| !name.contains(f.as_str())) {
            continue;
        }
        let screen = term.handler.terminal.plain_string();
        let title = String::from_utf8_lossy(&term.handler.terminal.title).to_string();
        let file = format!("{out_dir}/{seq:02}-{name}.txt");
        std::fs::write(
            &file,
            format!("# mark {name} t_ns={t} title={title:?} rows={rows} cols={cols}\n{screen}"),
        )?;
        println!("{seq:02} {name} title={title:?}");
        seq += 1;
    }
    Ok(())
}
