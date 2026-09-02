use std::time::Duration;

use corral_core::RepairBudget;

/// A daemon with no established client for this long may exit.
///
/// Runtime tuning, not a wire contract and not a public configuration surface:
/// the daemon is user-wide, so a spawning shell's environment must not decide
/// when it goes away (ADR 0001 D5).
pub const DEFAULT_IDLE_GRACE: Duration = Duration::from_secs(60);

/// How long a connection may stay pending before it must have said hello.
pub const DEFAULT_PRE_HELLO_DEADLINE: Duration = Duration::from_secs(10);

/// How long a starting daemon waits for a departing one to release the claim.
pub const SINGLETON_CLAIM_WAIT: Duration = Duration::from_secs(5);

/// How long an accept loop waits after a failed accept.
///
/// One number for both listeners. The canonical socket and the hook endpoint
/// have the same loop and the same reason for it — keeping a failing accept
/// from spinning the CPU while the cause persists — and two copies of one
/// decision is how they come to differ.
pub const ACCEPT_BACKOFF: Duration = Duration::from_millis(50);

/// How many accepted connections one listener may be serving at once.
///
/// One number for both, for the same reason as the backoff above. Generous
/// next to what an account's own surfaces open — a desktop, a TUI, a tray, and
/// a hook shim per event, each of which is one connection that ends — and a
/// bound at all because a task spawned per accept with no cap turns a process
/// that opens connections faster than it closes them into a daemon with no
/// memory left for the sessions it is watching.
///
/// Reaching it stops the loop accepting rather than refusing what it accepted:
/// the kernel holds the pending connections, and a shim that waits a moment is
/// a shim inside its budget where one handed a closed socket is a lost event.
///
/// `handshake::slots_come_back_when_connections_end` opens more connections
/// than this to prove a served connection frees its slot. It cannot read this
/// number — a client crate depending on the daemon would hand every surface a
/// path to `corral-core` — so raising this past the count that test opens
/// leaves it asserting nothing. Move them together.
pub const CONCURRENT_CONNECTIONS: usize = 128;

/// How much automatic repair one integration drift fingerprint may spend
/// before Corral stops rather than joining a configuration tug-of-war.
///
/// A dogfood-tunable policy default, never a wire constant (grill Q4′): the
/// numbers may go stricter or looser on dogfood evidence, but no code path may
/// silently exceed the authority accepted here. Sized against the measured
/// benign case — a provider's own whole-file rewrite can drop Corral's entry
/// in an ordinary race — so three in a day is already far past what that race
/// produces, while an authority replaying its own configuration crosses it on
/// the first day.
pub const REPAIR_BUDGET: RepairBudget = RepairBudget::new(3, Duration::from_secs(24 * 60 * 60));

/// How often the process table is swept for provider runtimes.
///
/// Chosen from how stale a row may be, not from what a sweep costs. The cost
/// was measured and is about a millisecond for a whole desktop's process
/// table (2026-09-02: p50 0.79 ms over 535 processes), so it buys nothing to
/// economize on — what this number actually decides is how long a session
/// that has been idle since before Corral started stays invisible, and how
/// long one that exited keeps a row.
pub const SWEEP_CADENCE: Duration = Duration::from_secs(30);

/// Timing the daemon runs under.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DaemonPolicy {
    pub idle_grace: Duration,
    pub pre_hello_deadline: Duration,
}

impl Default for DaemonPolicy {
    fn default() -> Self {
        Self {
            idle_grace: DEFAULT_IDLE_GRACE,
            pre_hello_deadline: DEFAULT_PRE_HELLO_DEADLINE,
        }
    }
}

impl DaemonPolicy {
    /// The policy this process runs under.
    pub fn resolve() -> Self {
        test_policy::resolve().unwrap_or_default()
    }
}

/// Test-support only (ADR 0001, "Test injection").
///
/// Lifecycle behaviour has to be proven against real processes, and no test
/// can wait a production idle grace to watch an idle exit. Each knob is a
/// single typed scalar rather than an open configuration map.
///
/// Normal production binaries do not recognize these variables at all; only
/// explicit `test-support` builds do, and `test-support` is not a default
/// feature. This is daemon runtime policy, distinct from the rendezvous
/// namespace seam in `corral-rendezvous`, which substitutes a whole Corral
/// root rather than tuning behaviour.
mod test_policy {
    use super::DaemonPolicy;

    #[cfg(feature = "test-support")]
    pub(super) fn resolve() -> Option<DaemonPolicy> {
        let defaults = DaemonPolicy::default();
        let idle_grace = duration("CORRAL_TEST_IDLE_GRACE_MS");
        let pre_hello_deadline = duration("CORRAL_TEST_PRE_HELLO_DEADLINE_MS");
        if idle_grace.is_none() && pre_hello_deadline.is_none() {
            return None;
        }
        Some(DaemonPolicy {
            idle_grace: idle_grace.unwrap_or(defaults.idle_grace),
            pre_hello_deadline: pre_hello_deadline.unwrap_or(defaults.pre_hello_deadline),
        })
    }

    #[cfg(feature = "test-support")]
    fn duration(variable: &str) -> Option<std::time::Duration> {
        std::env::var(variable)
            .ok()?
            .parse()
            .ok()
            .map(std::time::Duration::from_millis)
    }

    #[cfg(not(feature = "test-support"))]
    pub(super) fn resolve() -> Option<DaemonPolicy> {
        None
    }
}
