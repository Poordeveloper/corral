//! What the operating system will say about a process, and what it will not.
//!
//! The runtime-observation mechanism ADR 0014 D2 names, behind the platform
//! boundary: `(pid, start time, executable)` for one process, its parent, and
//! an enumeration of the whole table for the sweep.
//!
//! The failure modes are the interesting part, and they are kept apart
//! deliberately (measured 2026-09-02). "It is gone" and "I may not look" are
//! different answers, and only the first supports reporting that a Run ended
//! — reading a permission failure as an exit is exactly the
//! `unreachable == stopped` inference the runtime-truth law forbids. A zombie
//! answers gone, which is the honest answer: it is not carrying a session any
//! more.
//!
//! Start time is what makes a pid safe to remember. A reused pid necessarily
//! has a later start time, so `(pid, start time)` names one incarnation and
//! never two — measured at microsecond resolution on macOS.

use std::path::PathBuf;
use std::time::SystemTime;

/// One process, as this account is permitted to see it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProcessIdentity {
    pub pid: u32,
    pub parent: u32,
    /// The process group, which is what says whether a process descends from
    /// a Corral launch: a managed child is created as its own group leader,
    /// and what it spawns inherits the group unless it asks for one of its
    /// own. Descent by parent would not do — a launcher that exits severs it.
    pub group: u32,
    /// When the kernel says this process began.
    ///
    /// Authoritative occurrence time: it comes from the runtime rather than
    /// from when Corral happened to look, which is what lets a Run started
    /// before Corral existed still record when it started.
    pub started: SystemTime,
    /// The executable actually running, resolved by the kernel.
    ///
    /// Never `argv[0]`: measured on both platforms and both install channels,
    /// the path a provider is invoked by is a symlink or a launcher script
    /// and the real binary is somewhere else entirely.
    pub executable: PathBuf,
}

/// What asking about a pid produced.
///
/// Every variant is meaningful on every platform; what varies is which ones a
/// given build can reach. macOS reaches only `Unobservable`, by ruling, and
/// the expectation below fails the build the moment that stops being true —
/// so a later change of mind cannot leave a stale claim behind. It is scoped
/// away from test builds, where the ancestry walk's own tests construct every
/// variant against process trees they build.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(
    all(target_os = "macos", not(test)),
    expect(
        dead_code,
        reason = "macOS does not observe processes, by the ruling named below"
    )
)]
pub enum Observation {
    Identified(Box<ProcessIdentity>),
    /// No such process. The one answer that supports concluding a Run ended.
    Gone,
    /// The process exists and this account may not inspect it. Says nothing
    /// about whether it is running Corral's business — and never that it
    /// stopped.
    NotPermitted,
    /// This build cannot observe processes on this platform at all. Unknown
    /// is a first-class state and is never collapsed into `Gone`.
    Unobservable,
}

/// Every pid this account may enumerate.
///
/// `None` when enumeration is unavailable, which is not an empty machine: a
/// sweep that read failure as "nothing is running" would end every external
/// Run it had ever recorded.
pub fn all_pids() -> Option<Vec<u32>> {
    implementation::all_pids()
}

/// Observe one process.
pub fn observe(pid: u32) -> Observation {
    if pid == 0 {
        // Not a process: pid 0 is the kernel's placeholder for "no parent".
        // Answering `Gone` would say a process ended, which is the one thing
        // this must never invent about a number that never named one.
        return Observation::Unobservable;
    }
    implementation::observe(pid)
}

// macOS does not observe processes, by founder ruling of 2026-09-02
// (`docs/decisions/2026-09-01-pr7-integration-grill.md`). Not "not yet
// implemented": the decision was put and answered, and this is the answer.
//
// What it costs is stated where it is decided rather than discovered: every
// claim in ADR 0014's ladder needs a corroborating process, so on macOS no
// external session is discovered at all — not by the sweep, which has no
// table to read, and not by a delivery, whose identity nothing can
// corroborate. Corral on macOS sees the sessions it launched and no others.
//
// What it buys is that none of the three ways to reach the facts was taken:
// `libproc`'s unconditional `bindgen` build dependency, which would make
// libclang a build requirement for every developer and CI job; `sysinfo`,
// which exposes neither the microsecond start time nor the difference
// between gone and not-permitted, and would therefore have to guess at the
// one distinction that decides whether Corral may say a Run ended; and a
// named unsafe boundary crate, which the workspace lint says must be named
// in an ADR first.
//
// `Unobservable` is a first-class state and never collapses into `Gone`, so
// the effect is degraded awareness and never a false claim that a process
// ended.
#[cfg(target_os = "macos")]
mod implementation {
    use super::Observation;

    pub(super) fn observe(_pid: u32) -> Observation {
        Observation::Unobservable
    }

    pub(super) fn all_pids() -> Option<Vec<u32>> {
        None
    }
}

#[cfg(target_os = "linux")]
mod implementation {
    use super::{Observation, ProcessIdentity};
    use std::path::PathBuf;
    use std::time::{Duration, SystemTime};

    /// The kernel's tick rate, which `/proc/<pid>/stat` reports start time
    /// in. `sysconf(_SC_CLK_TCK)` is the authority and reading it needs
    /// `libc`; every Linux this runs on uses 100, and the value only scales a
    /// start time whose purpose is ordering and pid-reuse detection.
    const TICKS_PER_SECOND: u64 = 100;

    pub(super) fn observe(pid: u32) -> Observation {
        let stat = match std::fs::read_to_string(format!("/proc/{pid}/stat")) {
            Ok(stat) => stat,
            Err(error) => {
                return match error.kind() {
                    std::io::ErrorKind::NotFound => Observation::Gone,
                    _ => Observation::NotPermitted,
                };
            }
        };
        let Some(fields) = stat_fields(&stat) else {
            return Observation::NotPermitted;
        };
        // The executable is read through the kernel's own link rather than
        // from the command line, for the same reason as on macOS.
        let Ok(executable) = std::fs::read_link(format!("/proc/{pid}/exe")) else {
            return Observation::NotPermitted;
        };
        let (Some(parent), Some(group), Some(ticks)) =
            (fields.parent, fields.group, fields.started_ticks)
        else {
            return Observation::NotPermitted;
        };
        let Some(boot) = boot_time() else {
            return Observation::NotPermitted;
        };
        Observation::Identified(Box::new(ProcessIdentity {
            pid,
            parent,
            group,
            started: boot + Duration::from_millis(ticks * 1_000 / TICKS_PER_SECOND),
            executable,
        }))
    }

    struct StatFields {
        parent: Option<u32>,
        group: Option<u32>,
        started_ticks: Option<u64>,
    }

    /// Read `/proc/<pid>/stat` past the field a process name can hide in.
    ///
    /// The second field is the command in parentheses and may itself contain
    /// spaces and parentheses, so everything is counted from the last `)`
    /// rather than by splitting the whole line.
    fn stat_fields(stat: &str) -> Option<StatFields> {
        let after_name = stat.rfind(')')? + 1;
        let rest: Vec<&str> = stat.get(after_name..)?.split_whitespace().collect();
        // `rest[0]` is the state field, which `proc(5)` numbers 3.
        Some(StatFields {
            parent: rest.get(1).and_then(|raw| raw.parse().ok()),
            group: rest.get(2).and_then(|raw| raw.parse().ok()),
            started_ticks: rest.get(19).and_then(|raw| raw.parse().ok()),
        })
    }

    /// Every numeric entry under `/proc`, which is every process this account
    /// can see. Read once per sweep rather than per hop: the directory listing
    /// is the cheap half.
    pub(super) fn all_pids() -> Option<Vec<u32>> {
        let entries = std::fs::read_dir("/proc").ok()?;
        Some(
            entries
                .filter_map(|entry| entry.ok()?.file_name().to_str()?.parse().ok())
                .collect(),
        )
    }

    fn boot_time() -> Option<SystemTime> {
        let stat = std::fs::read_to_string("/proc/stat").ok()?;
        let seconds: u64 = stat
            .lines()
            .find_map(|line| line.strip_prefix("btime "))?
            .trim()
            .parse()
            .ok()?;
        Some(SystemTime::UNIX_EPOCH + Duration::from_secs(seconds))
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
mod implementation {
    use super::Observation;

    pub(super) fn observe(_pid: u32) -> Observation {
        Observation::Unobservable
    }

    pub(super) fn all_pids() -> Option<Vec<u32>> {
        None
    }
}

#[cfg(test)]
#[path = "process_tests.rs"]
mod tests;
