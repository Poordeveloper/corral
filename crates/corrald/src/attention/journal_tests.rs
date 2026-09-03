use std::time::{Duration, SystemTime};

use corral_core::{Assurance, AttentionItemId, CorralSessionId, EvidenceSource, MainState};

use super::*;

const DAY: Duration = Duration::from_secs(24 * 60 * 60);
/// 2026-09-02T12:00:00Z.
fn noon() -> SystemTime {
    SystemTime::UNIX_EPOCH + Duration::from_secs(1_788_350_400)
}

fn scratch() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("corral-journal-{}", CorralSessionId::mint()));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

fn transition(session: CorralSessionId, to: MainState) -> Record {
    Record::Transition(TransitionRecord {
        session,
        from: MainState::Unknown,
        to,
        source: Some(EvidenceSource::ProviderHook),
        assurance: Some(Assurance::Attested),
        sealed: Some(true),
        provider_version: Some("2.1.258".to_owned()),
        horizon: Some(Duration::from_secs(300)),
        expired_after: None,
        contradicted_first: None,
        born: Some(AttentionItemId::mint()),
        ended: None,
        item_end: None,
        notifiable: true,
    })
}

#[test]
fn records_go_to_the_file_named_for_their_day() {
    let dir = scratch();
    let mut journal = Journal::open(&dir, Budget::default(), noon()).expect("open");
    let session = CorralSessionId::mint();
    journal
        .append(noon(), transition(session, MainState::NeedsYou))
        .expect("append");
    journal
        .append(
            noon() + Duration::from_secs(60),
            transition(session, MainState::Ready),
        )
        .expect("append");
    journal
        .append(noon() + DAY, transition(session, MainState::Working))
        .expect("append");

    let today = std::fs::read_to_string(dir.join("attention-journal-2026-09-02.jsonl"))
        .expect("today's file");
    assert_eq!(today.lines().count(), 2);
    let tomorrow = std::fs::read_to_string(dir.join("attention-journal-2026-09-03.jsonl"))
        .expect("tomorrow's file");
    assert_eq!(tomorrow.lines().count(), 1);
}

/// The record's shape is closed: every key is one ADR 0015 D8 names, so a
/// raw screen, a prompt, or a payload has nowhere to go.
#[test]
fn a_record_has_only_the_keys_the_decision_names() {
    let dir = scratch();
    let mut journal = Journal::open(&dir, Budget::default(), noon()).expect("open");
    journal
        .append(
            noon(),
            transition(CorralSessionId::mint(), MainState::NeedsYou),
        )
        .expect("append");
    let line =
        std::fs::read_to_string(dir.join("attention-journal-2026-09-02.jsonl")).expect("file");
    let value: serde_json::Value = serde_json::from_str(line.trim()).expect("json");
    let mut keys: Vec<&str> = value
        .as_object()
        .expect("object")
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        [
            "assurance",
            "at_unix_ms",
            "born",
            "build",
            "contradicted_first",
            "ended",
            "expired_after_ms",
            "from",
            "horizon_ms",
            "item_end",
            "kind",
            "notifiable",
            "provider_version",
            "sealed",
            "seq",
            "session",
            "source",
            "to",
        ]
    );
}

/// A day's budget bounds the file; exhausting it stops ordinary records,
/// leaves an explicit marker, and keeps everything written before
/// (grill Q26). Never rotates early records away.
#[test]
fn exhausting_the_day_budget_marks_the_day_incomplete_and_keeps_earlier_records() {
    let dir = scratch();
    let budget = Budget {
        per_day_bytes: 600,
        retention: Duration::from_secs(30 * 24 * 60 * 60),
    };
    let mut journal = Journal::open(&dir, budget, noon()).expect("open");
    let session = CorralSessionId::mint();
    let mut outcomes = Vec::new();
    for i in 0..8 {
        outcomes.push(
            journal
                .append(
                    noon() + Duration::from_secs(i),
                    transition(session, MainState::Ready),
                )
                .expect("append"),
        );
    }
    assert!(outcomes.contains(&Appended::Written));
    assert!(outcomes.contains(&Appended::Incomplete));
    assert_eq!(outcomes.last(), Some(&Appended::Incomplete));
    let written = outcomes.iter().filter(|o| **o == Appended::Written).count();
    let file =
        std::fs::read_to_string(dir.join("attention-journal-2026-09-02.jsonl")).expect("file");
    assert_eq!(file.lines().count(), written);
    assert!(dir.join("attention-journal-2026-09-02.incomplete").exists());
}

#[test]
fn files_older_than_retention_are_pruned_at_open_and_at_rollover() {
    let dir = scratch();
    std::fs::write(dir.join("attention-journal-2026-07-01.jsonl"), "{}\n").expect("old file");
    std::fs::write(dir.join("attention-journal-2026-07-01.incomplete"), "").expect("old marker");
    std::fs::write(dir.join("attention-journal-2026-08-20.jsonl"), "{}\n").expect("recent file");
    let mut journal = Journal::open(&dir, Budget::default(), noon()).expect("open");
    assert!(!dir.join("attention-journal-2026-07-01.jsonl").exists());
    assert!(!dir.join("attention-journal-2026-07-01.incomplete").exists());
    assert!(dir.join("attention-journal-2026-08-20.jsonl").exists());

    // Thirty days later the August file is past retention; rolling to a new
    // day is what prunes it, so a daemon alive for weeks still prunes.
    journal
        .append(
            noon() + 30 * DAY,
            transition(CorralSessionId::mint(), MainState::Ready),
        )
        .expect("append");
    assert!(!dir.join("attention-journal-2026-08-20.jsonl").exists());
}

#[test]
fn a_dispute_names_the_item_it_is_about_and_whether_it_was_stale() {
    let dir = scratch();
    let mut journal = Journal::open(&dir, Budget::default(), noon()).expect("open");
    let item = AttentionItemId::mint();
    journal
        .append(
            noon(),
            Record::Dispute(DisputeRecord {
                session: CorralSessionId::mint(),
                item: Some(item),
                stale: true,
            }),
        )
        .expect("append");
    let line =
        std::fs::read_to_string(dir.join("attention-journal-2026-09-02.jsonl")).expect("file");
    let value: serde_json::Value = serde_json::from_str(line.trim()).expect("json");
    assert_eq!(value["kind"], "dispute");
    assert_eq!(value["item"], item.to_string());
    assert_eq!(value["stale"], true);
}

/// The report reads what the engine never does, and says which days are
/// incomplete rather than counting them as quiet.
#[test]
fn the_report_counts_transitions_and_names_incomplete_days() {
    let dir = scratch();
    let budget = Budget {
        per_day_bytes: 600,
        ..Budget::default()
    };
    let mut journal = Journal::open(&dir, budget, noon()).expect("open");
    let session = CorralSessionId::mint();
    for i in 0..8 {
        journal
            .append(
                noon() + Duration::from_secs(i),
                transition(session, MainState::NeedsYou),
            )
            .expect("append");
    }
    journal
        .append(noon() + DAY, transition(session, MainState::Ready))
        .expect("append");
    let report = report(&dir).expect("report");
    let today = report
        .days
        .iter()
        .find(|d| d.date == "2026-09-02")
        .expect("today");
    assert!(today.incomplete);
    assert!(today.transitions >= 1);
    assert_eq!(today.into_needs_you, today.transitions);
    let tomorrow = report
        .days
        .iter()
        .find(|d| d.date == "2026-09-03")
        .expect("tomorrow");
    assert!(!tomorrow.incomplete);
    assert_eq!(tomorrow.transitions, 1);
    assert_eq!(tomorrow.into_needs_you, 0);
}

/// A line the reader cannot parse is a record it cannot count, and a day it
/// cannot count is not a complete evidence day. Reporting the smaller number
/// as if it were the whole day is exactly the silent incompleteness the budget
/// marker exists to prevent (ADR 0015 D8, grill Q26).
#[test]
fn a_day_holding_a_record_that_will_not_parse_is_reported_incomplete() {
    let dir = scratch();
    let mut journal = Journal::open(&dir, Budget::default(), noon()).expect("open");
    journal
        .append(
            noon(),
            transition(CorralSessionId::mint(), MainState::NeedsYou),
        )
        .expect("append");
    // A write the daemon did not finish: the process died mid-line.
    let path = dir.join("attention-journal-2026-09-02.jsonl");
    let mut partial = std::fs::read_to_string(&path).expect("file");
    partial.push_str("{\"kind\":\"transi");
    std::fs::write(&path, partial).expect("truncated write");

    let report = report(&dir).expect("report");
    let day = report.days.first().expect("a day");
    assert_eq!(day.transitions, 1, "the record that parsed still counts");
    assert!(
        day.incomplete,
        "a day with a record nobody can read is not a complete evidence day"
    );
}

/// `since` exists to make a string comparison against the journal's own day
/// names mean something, so the check has to establish exactly that: a real
/// calendar day, in the one spelling those names use. A five-digit year is
/// the case that shows the difference — it parses, it is a date, and it sorts
/// *below* every four-digit year, so a report asked to start in the year
/// 10000 would answer with 2026.
#[test]
fn a_day_name_is_a_real_calendar_day_in_the_journals_own_spelling() {
    assert!(names_a_day("2026-09-03"));
    assert!(names_a_day("2024-02-29"), "2024 is a leap year");
    assert!(names_a_day("2026-12-31"));

    assert!(
        !names_a_day("10000-01-01"),
        "sorts below every 4-digit year"
    );
    assert!(!names_a_day("2026-02-31"), "February has no 31st");
    assert!(!names_a_day("2025-02-29"), "2025 is not a leap year");
    assert!(!names_a_day("2026-04-31"), "April has 30 days");
    assert!(!names_a_day("1900-02-29"), "1900 is not a leap year");
    assert!(
        names_a_day("2000-02-29"),
        "2000 is a leap year: divisible by 400"
    );
    assert!(!names_a_day("2026-9-9"), "not the spelling the names use");
    assert!(!names_a_day("2026-09"));
    assert!(!names_a_day("not-a-date"));
    assert!(!names_a_day("2026-00-01"));
    assert!(!names_a_day("2026-01-00"));
}

/// A record whose shape this build cannot place is a record it cannot count,
/// which is the same thing to the day's evidence as a line that will not
/// parse: countable or incomplete, never a quietly smaller number
/// (ADR 0015 D8).
#[test]
fn a_record_the_reader_cannot_place_makes_its_day_incomplete() {
    // A record that says it is a transition still counts as one; what it
    // cannot do is say which class the day gained.
    for (line, transitions) in [
        (r#"{"kind":"something_this_build_never_wrote"}"#, 1),
        (r#"{"kind":"transition","to":123}"#, 2),
        (
            r#"{"kind":"transition","to":"a_state_this_build_cannot_place"}"#,
            2,
        ),
        (r#"{"seq":1}"#, 1),
    ] {
        let dir = scratch();
        let mut journal = Journal::open(&dir, Budget::default(), noon()).expect("open");
        journal
            .append(
                noon(),
                transition(CorralSessionId::mint(), MainState::NeedsYou),
            )
            .expect("append");
        let path = dir.join("attention-journal-2026-09-02.jsonl");
        let mut text = std::fs::read_to_string(&path).expect("file");
        text.push_str(line);
        text.push('\n');
        std::fs::write(&path, text).expect("write");

        let report = report(&dir).expect("report");
        let day = report.days.first().expect("a day");
        assert!(day.incomplete, "{line} is a record nobody can count");
        assert_eq!(day.transitions, transitions, "{line}");
        assert_eq!(
            day.into_needs_you, 1,
            "the record that could be placed still counts"
        );
    }
}
