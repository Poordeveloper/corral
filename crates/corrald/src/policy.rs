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
