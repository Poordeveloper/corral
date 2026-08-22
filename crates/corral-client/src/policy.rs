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
        test_policy::resolve().unwrap_or_default()
    }
}

/// Test-support only (ADR 0001, "Test injection").
///
/// A wedged rendezvous is only observable once the activation budget expires,
/// and the merge gate cannot spend the production budget proving it. One typed
/// scalar.
///
/// Normal production binaries do not recognize the variable at all; only
/// explicit `test-support` builds do, and `test-support` is not a default
/// feature.
mod test_policy {
    use super::ClientActivationPolicy;

    #[cfg(feature = "test-support")]
    pub(super) fn resolve() -> Option<ClientActivationPolicy> {
        let activation_deadline = std::env::var("CORRAL_TEST_ACTIVATION_DEADLINE_MS")
            .ok()?
            .parse()
            .ok()
            .map(std::time::Duration::from_millis)?;
        Some(ClientActivationPolicy {
            activation_deadline,
        })
    }

    #[cfg(not(feature = "test-support"))]
    pub(super) fn resolve() -> Option<ClientActivationPolicy> {
        None
    }
}
