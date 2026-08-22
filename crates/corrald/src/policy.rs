use std::time::Duration;

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
        let defaults = Self::default();
        Self {
            idle_grace: test_duration("CORRAL_TEST_IDLE_GRACE_MS").unwrap_or(defaults.idle_grace),
            pre_hello_deadline: test_duration("CORRAL_TEST_PRE_HELLO_DEADLINE_MS")
                .unwrap_or(defaults.pre_hello_deadline),
        }
    }
}

/// Test-support only (ADR 0001, "Test injection").
///
/// Lifecycle behaviour has to be proven against real processes, and no test can
/// wait a production idle grace to watch an idle exit. Each knob is a single
/// typed scalar rather than an open configuration map, and the whole mechanism
/// is compiled out of release builds, so production packaging cannot reach it.
/// It is not a user configuration surface and never participates in
/// auto-activation.
#[cfg(debug_assertions)]
fn test_duration(variable: &str) -> Option<Duration> {
    std::env::var(variable)
        .ok()?
        .parse()
        .ok()
        .map(Duration::from_millis)
}

#[cfg(not(debug_assertions))]
fn test_duration(_variable: &str) -> Option<Duration> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn production_defaults_are_the_documented_ones() {
        let policy = DaemonPolicy::default();

        assert_eq!(policy.idle_grace, Duration::from_secs(60));
        assert_eq!(policy.pre_hello_deadline, Duration::from_secs(10));
    }
}
