//! The attention journal: diagnostic evidence of what the engine decided,
//! and never an input to it (ADR 0015 D8, grill Q8/Q26).
//!
//! One file per day under the diagnostics directory, a closed record shape
//! with nowhere to put a screen or a payload, a per-day budget that ends in
//! an explicit `.incomplete` marker rather than in silently dropped records,
//! and a thirty-day prune that runs at open and at every day rollover, so a
//! daemon alive for weeks prunes too.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use corral_core::{Assurance, AttentionItemId, CorralSessionId, EvidenceSource, MainState};
use serde_json::{Value, json};

use super::ItemEnd;

const FILE_PREFIX: &str = "attention-journal-";
const FILE_SUFFIX: &str = ".jsonl";
const INCOMPLETE_SUFFIX: &str = ".incomplete";
/// `YYYY-MM-DD`. The journal's own day names are this wide, and the report's
/// string comparison only orders correctly while every name is.
const FILE_DATE_WIDTH: usize = 10;

/// The bound on one day's file and how long files are kept.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Budget {
    pub per_day_bytes: u64,
    pub retention: Duration,
}

impl Default for Budget {
    fn default() -> Self {
        Self {
            per_day_bytes: 16 * 1024 * 1024,
            retention: Duration::from_secs(30 * 24 * 60 * 60),
        }
    }
}

/// One main-state transition as the engine made it.
///
/// Every field is a code, a number, or a Corral-minted id: the provider
/// version is the one free-form string, and it is a version.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TransitionRecord {
    pub session: CorralSessionId,
    pub from: MainState,
    pub to: MainState,
    pub source: Option<EvidenceSource>,
    pub assurance: Option<Assurance>,
    pub sealed: Option<bool>,
    pub provider_version: Option<String>,
    pub horizon: Option<Duration>,
    /// How long past its horizon the claim was when it rotted.
    pub expired_after: Option<Duration>,
    /// Whether contradicting evidence arrived before the horizon did.
    pub contradicted_first: Option<bool>,
    /// The item born by this transition, if one was.
    pub born: Option<AttentionItemId>,
    /// The item this transition ended, and how it ended. A move straight
    /// from one actionable state to another carries both this and `born`.
    pub ended: Option<AttentionItemId>,
    pub item_end: Option<ItemEnd>,
    /// Whether this transition is one a notification may be emitted for.
    pub notifiable: bool,
}

/// A person said the current item was wrong.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DisputeRecord {
    pub session: CorralSessionId,
    pub item: Option<AttentionItemId>,
    /// The item named was no longer current when the dispute arrived.
    pub stale: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Record {
    Transition(TransitionRecord),
    Dispute(DisputeRecord),
}

/// What became of one append.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Appended {
    Written,
    /// The day's budget is exhausted; the record was not written and the
    /// day is marked incomplete.
    Incomplete,
}

/// The journal writer for this daemon.
pub struct Journal {
    dir: PathBuf,
    budget: Budget,
    seq: u64,
    day: Option<OpenDay>,
}

struct OpenDay {
    date: CivilDate,
    file: File,
    written: u64,
    incomplete: bool,
}

impl Journal {
    /// Open the journal, creating the directory and pruning what retention
    /// no longer keeps.
    pub fn open(dir: &Path, budget: Budget, now: SystemTime) -> std::io::Result<Self> {
        std::fs::create_dir_all(dir)?;
        let mut journal = Self {
            dir: dir.to_path_buf(),
            budget,
            seq: 0,
            day: None,
        };
        journal.prune(now)?;
        Ok(journal)
    }

    /// Where the journal lives, for the report reader.
    #[must_use]
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Append one record to the day's file.
    pub fn append(&mut self, now: SystemTime, record: Record) -> std::io::Result<Appended> {
        let date = CivilDate::of(now);
        if self.day.as_ref().is_none_or(|day| day.date != date) {
            self.prune(now)?;
            let path = self.dir.join(file_name(date, FILE_SUFFIX));
            let file = OpenOptions::new().append(true).create(true).open(&path)?;
            let written = file.metadata().map(|m| m.len()).unwrap_or(0);
            let incomplete = self.dir.join(file_name(date, INCOMPLETE_SUFFIX)).exists();
            self.day = Some(OpenDay {
                date,
                file,
                written,
                incomplete,
            });
        }
        let Some(day) = self.day.as_mut() else {
            return Ok(Appended::Incomplete);
        };
        if day.incomplete {
            return Ok(Appended::Incomplete);
        }
        self.seq += 1;
        let mut line = encode(&record, now, self.seq).to_string();
        line.push('\n');
        if day.written + line.len() as u64 > self.budget.per_day_bytes {
            // The budget is met by refusing, never by rotating earlier
            // records away: the marker says the day's evidence is partial.
            day.incomplete = true;
            std::fs::write(self.dir.join(file_name(date, INCOMPLETE_SUFFIX)), b"")?;
            return Ok(Appended::Incomplete);
        }
        day.file.write_all(line.as_bytes())?;
        day.written += line.len() as u64;
        Ok(Appended::Written)
    }

    /// Remove every day past retention, marker included.
    pub fn prune(&mut self, now: SystemTime) -> std::io::Result<()> {
        let oldest = now.checked_sub(self.budget.retention).map(CivilDate::of);
        for entry in std::fs::read_dir(&self.dir)? {
            let entry = entry?;
            let name = entry.file_name();
            let Some(date) = date_of_file(&name.to_string_lossy()) else {
                continue;
            };
            if oldest.is_some_and(|oldest| date < oldest) {
                std::fs::remove_file(entry.path())?;
            }
        }
        Ok(())
    }
}

fn encode(record: &Record, now: SystemTime, seq: u64) -> Value {
    let at_unix_ms = now
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0);
    match record {
        Record::Transition(t) => json!({
            "kind": "transition",
            "seq": seq,
            "at_unix_ms": at_unix_ms,
            "build": env!("CARGO_PKG_VERSION"),
            "session": t.session.to_string(),
            "from": main_state(t.from),
            "to": main_state(t.to),
            "source": t.source.map(source),
            "assurance": t.assurance.map(assurance),
            "sealed": t.sealed,
            "provider_version": t.provider_version,
            "horizon_ms": t.horizon.map(|d| d.as_millis()),
            "expired_after_ms": t.expired_after.map(|d| d.as_millis()),
            "contradicted_first": t.contradicted_first,
            "born": t.born.map(|id| id.to_string()),
            "ended": t.ended.map(|id| id.to_string()),
            "item_end": t.item_end.map(item_end),
            "notifiable": t.notifiable,
        }),
        Record::Dispute(d) => json!({
            "kind": "dispute",
            "seq": seq,
            "at_unix_ms": at_unix_ms,
            "build": env!("CARGO_PKG_VERSION"),
            "session": d.session.to_string(),
            "item": d.item.map(|id| id.to_string()),
            "stale": d.stale,
        }),
    }
}

/// The state a spelling names, or `None` for one this build cannot place.
fn main_state_named(text: &str) -> Option<MainState> {
    match text {
        "working" => Some(MainState::Working),
        "needs_you" => Some(MainState::NeedsYou),
        "ready" => Some(MainState::Ready),
        "unknown" => Some(MainState::Unknown),
        "exited" => Some(MainState::Exited),
        _ => None,
    }
}

fn main_state(state: MainState) -> &'static str {
    match state {
        MainState::Working => "working",
        MainState::NeedsYou => "needs_you",
        MainState::Ready => "ready",
        MainState::Unknown => "unknown",
        MainState::Exited => "exited",
    }
}

fn source(source: EvidenceSource) -> &'static str {
    match source {
        EvidenceSource::CorralConstructed => "corral_constructed",
        EvidenceSource::NodeRuntimeObservation => "node_runtime_observation",
        EvidenceSource::ProviderHook => "provider_hook",
        EvidenceSource::InBandSignal => "in_band_signal",
        EvidenceSource::PtyActivity => "pty_activity",
        EvidenceSource::ScreenDetection => "screen_detection",
        EvidenceSource::HistoryRecord => "history_record",
        EvidenceSource::Correlation => "correlation",
        EvidenceSource::UserAssertion => "user_assertion",
    }
}

fn assurance(assurance: Assurance) -> &'static str {
    match assurance {
        Assurance::Deterministic => "deterministic",
        Assurance::Attested => "attested",
        Assurance::Manual => "manual",
        Assurance::Heuristic => "heuristic",
    }
}

fn item_end(end: ItemEnd) -> &'static str {
    match end {
        ItemEnd::Resolved => "resolved",
        ItemEnd::Rotted => "rotted",
        ItemEnd::Exited => "exited",
    }
}

/// A calendar day, UTC, from the epoch — the one date arithmetic the journal
/// needs, done here rather than through a dependency it would use for
/// nothing else.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct CivilDate {
    year: i64,
    month: u8,
    day: u8,
}

impl CivilDate {
    fn of(at: SystemTime) -> Self {
        let days = at
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| i64::try_from(d.as_secs() / 86_400).unwrap_or(i64::MAX))
            .unwrap_or(0);
        // Howard Hinnant's civil-from-days, for a proleptic Gregorian calendar.
        let z = days + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z.rem_euclid(146_097);
        let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
        let y = yoe + era * 400;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let day = u8::try_from(doy - (153 * mp + 2) / 5 + 1).unwrap_or(1);
        let month = u8::try_from(if mp < 10 { mp + 3 } else { mp - 9 }).unwrap_or(1);
        let year = if month <= 2 { y + 1 } else { y };
        Self { year, month, day }
    }

    fn parse(text: &str) -> Option<Self> {
        let mut parts = text.splitn(3, '-');
        let year = parts.next()?.parse().ok()?;
        let month = parts.next()?.parse().ok()?;
        let day = parts.next()?.parse().ok()?;
        Some(Self { year, month, day })
    }

    /// Whether this names a day that exists. `parse` reads three numbers; a
    /// calendar decides which of them are a date.
    fn is_real(self) -> bool {
        (1..=12).contains(&self.month) && (1..=self.days_in_month()).contains(&self.day)
    }

    fn days_in_month(self) -> u8 {
        match self.month {
            1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
            4 | 6 | 9 | 11 => 30,
            2 if self.is_leap_year() => 29,
            2 => 28,
            _ => 0,
        }
    }

    fn is_leap_year(self) -> bool {
        self.year % 4 == 0 && (self.year % 100 != 0 || self.year % 400 == 0)
    }
}

impl std::fmt::Display for CivilDate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:04}-{:02}-{:02}", self.year, self.month, self.day)
    }
}

/// Whether `text` names a day in the one spelling the journal uses:
/// `YYYY-MM-DD`, zero padded, four-digit year, and a day the calendar has.
///
/// `attention.report` filters by comparing `since` against these names as
/// strings, and this is what makes that comparison mean what it says. The
/// width is load-bearing, not tidiness: `10000-01-01` is a date, and it sorts
/// below every four-digit year, so a report asked to start in the year 10000
/// would answer with 2026.
#[must_use]
pub fn names_a_day(text: &str) -> bool {
    text.len() == FILE_DATE_WIDTH
        && CivilDate::parse(text).is_some_and(|date| date.is_real() && date.to_string() == text)
}

fn file_name(date: CivilDate, suffix: &str) -> String {
    format!("{FILE_PREFIX}{date}{suffix}")
}

fn date_of_file(name: &str) -> Option<CivilDate> {
    let rest = name.strip_prefix(FILE_PREFIX)?;
    let date = rest
        .strip_suffix(FILE_SUFFIX)
        .or_else(|| rest.strip_suffix(INCOMPLETE_SUFFIX))?;
    CivilDate::parse(date)
}

/// One day as the report reads it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DayReport {
    pub date: String,
    pub transitions: u64,
    pub into_needs_you: u64,
    pub into_ready: u64,
    pub disputes: u64,
    /// The day's budget was exhausted: its counts are a floor, never a
    /// count of a quiet day.
    pub incomplete: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Report {
    pub days: Vec<DayReport>,
}

/// Read the journal back for `corral attention report`. The engine never
/// calls this; reporting is what the journal is for.
pub fn report(dir: &Path) -> std::io::Result<Report> {
    let mut days = Vec::new();
    // An entry the directory cannot yield is a day that would simply not be
    // in the answer, with nothing saying so. The failure travels instead, as
    // it already does in `prune`.
    let mut names: Vec<String> = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let name = entry?.file_name().to_string_lossy().into_owned();
        if name.starts_with(FILE_PREFIX) && name.ends_with(FILE_SUFFIX) {
            names.push(name);
        }
    }
    names.sort();
    for name in names {
        let Some(date) = date_of_file(&name) else {
            continue;
        };
        let text = std::fs::read_to_string(dir.join(&name))?;
        let mut day = DayReport {
            date: date.to_string(),
            transitions: 0,
            into_needs_you: 0,
            into_ready: 0,
            disputes: 0,
            incomplete: dir.join(file_name(date, INCOMPLETE_SUFFIX)).exists(),
        };
        for line in text.lines().filter(|l| !l.trim().is_empty()) {
            let Ok(value) = serde_json::from_str::<Value>(line) else {
                // A record nobody can read is a record nobody can count, so
                // the day stops claiming to be complete rather than reporting
                // the smaller number as the whole of it.
                day.incomplete = true;
                continue;
            };
            match value["kind"].as_str() {
                Some("transition") => {
                    day.transitions += 1;
                    match value["to"].as_str().and_then(main_state_named) {
                        Some(MainState::NeedsYou) => day.into_needs_you += 1,
                        Some(MainState::Ready) => day.into_ready += 1,
                        Some(MainState::Working | MainState::Unknown | MainState::Exited) => {}
                        // A state this build cannot place is a transition it
                        // cannot classify, so the day's class counts are a
                        // floor rather than a count.
                        None => day.incomplete = true,
                    }
                }
                Some("dispute") => day.disputes += 1,
                // Syntax this build can read and a record shape it cannot is
                // the same thing to the day's evidence as a line that will not
                // parse at all: countable, or incomplete.
                _ => day.incomplete = true,
            }
        }
        days.push(day);
    }
    Ok(Report { days })
}

#[cfg(test)]
#[path = "journal_tests.rs"]
mod tests;
