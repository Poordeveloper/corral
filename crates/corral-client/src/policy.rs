use std::time::Duration;

/// The default overall activation budget.
///
/// Comfortably longer than a daemon's own bounded wait for the singleton lock,
/// so a client that legitimately started one does not give up while that daemon
/// is still winning its claim.
pub const DEFAULT_ACTIVATION_DEADLINE: Duration = Duration::from_secs(15);

/// Timing the activation state machine runs under.
///
/// One overall deadline covers probe, spawn, connect and handshake together.
/// Per-stage budgets would sum into a wait nobody chose, and a person waiting
/// for `corral list` experiences the total, not the stages.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClientActivationPolicy {
    pub activation_deadline: Duration,
}

impl Default for ClientActivationPolicy {
    fn default() -> Self {
        Self {
            activation_deadline: DEFAULT_ACTIVATION_DEADLINE,
        }
    }
}

impl ClientActivationPolicy {
    /// The policy a surface runs under.
    pub fn resolve() -> Self {
        Self {
            activation_deadline: test_activation_deadline().unwrap_or(DEFAULT_ACTIVATION_DEADLINE),
        }
    }
}

/// Test-support only (ADR 0001, "Test injection").
///
/// A wedged rendezvous is only observable after the activation budget expires,
/// and the merge gate cannot spend the production budget proving it. One typed
/// scalar, compiled out of release builds, never a user configuration surface.
#[cfg(debug_assertions)]
fn test_activation_deadline() -> Option<Duration> {
    std::env::var("CORRAL_TEST_ACTIVATION_DEADLINE_MS")
        .ok()?
        .parse()
        .ok()
        .map(Duration::from_millis)
}

#[cfg(not(debug_assertions))]
fn test_activation_deadline() -> Option<Duration> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_production_default_is_the_documented_one() {
        assert_eq!(
            ClientActivationPolicy::default().activation_deadline,
            Duration::from_secs(15)
        );
    }
}
