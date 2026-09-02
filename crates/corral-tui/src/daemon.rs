//! Which corrald this surface is talking to, and what it may ask it.
//!
//! Split from the list because it has an invariant of its own and the list has
//! enough: which failures leave a connection that can be asked again, when
//! starting a daemon is allowed, and how a wait that ran out is described to
//! someone. The surface renders what this reports and decides none of it.

use std::time::{Duration, Instant};

use corral_client::{ClientActivationPolicy, Connection, RequestError, activate};
use corral_protocol::method::SessionListResult;

use crate::ANSWER;

/// Why there is no list to show.
///
/// Three different claims about the daemon, and the surface must not make the
/// wrong one: one that refused answered, and is on the other end of a
/// connection that is fine; one whose answer this build could not read also
/// answered; only the third may not be there at all (`AGENTS.md` §Runtime
/// truth).
pub enum Unanswered {
    /// It answered, and the answer was a refusal.
    Refused(String),
    /// It answered, and the answer was not one this build can read.
    Unreadable(String),
    /// Nothing answered.
    Silent(String),
}

impl Unanswered {
    pub fn line(&self) -> String {
        match self {
            Self::Refused(detail) => format!("corrald would not list its sessions: {detail}"),
            Self::Unreadable(detail) => {
                format!("corrald answered with something this build cannot read: {detail}")
            }
            Self::Silent(detail) => format!("corrald did not answer: {detail}"),
        }
    }
}

/// What one failed request says about the daemon behind it.
///
/// The claim and the disposal are two decisions, and only one of them is the
/// same for both: a protocol fault and a lost daemon both leave a connection
/// this client cannot ask again, but only one of them means nothing is there
/// (`AGENTS.md` §Runtime truth).
pub fn about(error: &RequestError) -> Unanswered {
    match error {
        RequestError::Refused(_) => Unanswered::Refused(error.to_string()),
        RequestError::Protocol { .. } => Unanswered::Unreadable(error.to_string()),
        RequestError::DaemonConnectionLost { .. } => Unanswered::Silent(error.to_string()),
    }
}

/// The connection to `corrald`, and the ability to get another one.
///
/// A lost daemon is not a dead list: the list says it cannot be read, keeps
/// asking, and picks up again when one answers — a person who restarted
/// `corrald` should not have to restart this too. Activation is the client
/// library's, exactly as the CLI does it, so this surface can never start a
/// daemon on terms of its own (ADR 0001).
pub struct Daemon<'a> {
    pub policy: &'a ClientActivationPolicy,
    pub connection: Option<Connection>,
    /// When activation may be attempted again, and how many have failed in a
    /// row.
    ///
    /// Activation may start a daemon. A `corrald` that dies on startup leaves
    /// no owner behind, so a poll that activated every second would start one
    /// every second forever — a retry loop that is indistinguishable, from the
    /// outside, from a fork bomb with a one-second fuse. The poll keeps its
    /// cadence; only starting one backs off.
    pub retry: Option<Backoff>,
}

/// How long to wait before trying to activate again.
pub struct Backoff {
    failures: u32,
    until: Instant,
}

impl Backoff {
    /// Doubling, to a ceiling: long enough that a daemon which cannot start is
    /// not started repeatedly, short enough that a person who fixes whatever
    /// stopped it does not wait long to see the list come back.
    const CEILING: Duration = Duration::from_secs(30);

    fn after(failures: u32) -> Self {
        let seconds = 1_u64 << failures.min(5);
        Self {
            failures: failures.saturating_add(1),
            until: Instant::now() + Duration::from_secs(seconds).min(Self::CEILING),
        }
    }

    fn waiting(&self) -> Option<Duration> {
        self.until.checked_duration_since(Instant::now())
    }
}

impl Daemon<'_> {
    /// The connection this daemon already has, for something the person just
    /// did.
    ///
    /// Never activates. Activation may spawn a daemon and wait out an
    /// activation deadline that is not this crate's to bound, and this runs
    /// with the terminal handed over and nothing reading the keyboard. The
    /// poll owns starting a daemon, because the poll is the wait a person can
    /// interrupt.
    pub fn connection(&mut self) -> Result<&mut Connection, String> {
        self.connection
            .as_mut()
            .ok_or_else(|| "corrald has not answered yet; the list is still asking".to_owned())
    }

    /// The connection, taken out of this daemon for exactly one question.
    ///
    /// Taken rather than borrowed, because waiting for the answer is
    /// interruptible: a question this surface stopped waiting for — a
    /// keystroke arrived, or `ANSWER` ran out — leaves a socket whose next
    /// read is an answer nobody is holding any more. Putting it back only once
    /// one arrived means an abandoned question costs a reconnect and never a
    /// mismatched answer.
    async fn borrow_for_one_question(&mut self) -> Result<Connection, Unanswered> {
        match self.connection.take() {
            Some(connection) => Ok(connection),
            None => self.activated().await,
        }
    }

    /// A connection to a running daemon, starting one if there is none and the
    /// last attempt is far enough behind.
    ///
    /// The one place this surface activates. Both the poll and the person's
    /// own actions come through here, so a daemon that cannot start is not
    /// started once per second by one and once per keystroke by the other.
    async fn activated(&mut self) -> Result<Connection, Unanswered> {
        if let Some(waiting) = self.retry.as_ref().and_then(Backoff::waiting) {
            return Err(Unanswered::Silent(format!(
                "no corrald is running; trying again in {} seconds",
                waiting.as_secs().max(1)
            )));
        }

        // Armed before the wait, not after it. This future is dropped whole
        // when a key arrives, and an attempt recorded only on the way out
        // would not be recorded at all — leaving a person who types once a
        // second starting a daemon once a second, which is the loop the
        // backoff exists to stop.
        let failures = self.retry.as_ref().map_or(0, |backoff| backoff.failures);
        self.retry = Some(Backoff::after(failures));

        match activate(self.policy).await {
            Ok(connection) => {
                self.retry = None;
                Ok(connection)
            }
            Err(error) => Err(Unanswered::Silent(error.to_string())),
        }
    }

    /// What the daemon holds, or why it did not say.
    pub async fn sessions(&mut self) -> Result<SessionListResult, Unanswered> {
        let mut connection = self.borrow_for_one_question().await?;

        // The connection is not put back when this runs out: its next read
        // would be the answer to a question nobody is holding any more.
        let Ok(answered) = tokio::time::timeout(ANSWER, connection.session_list()).await else {
            return Err(Unanswered::Silent(format!(
                "nothing within {} seconds",
                ANSWER.as_secs()
            )));
        };

        match answered {
            Ok(listed) => {
                self.connection = Some(connection);
                Ok(listed)
            }
            Err(error) => {
                // Put back only what can be asked again. A refusal came on a
                // connection that is fine; the others left one with nobody on
                // it, or at a place in the stream this client cannot find.
                if matches!(error, RequestError::Refused(_)) {
                    self.connection = Some(connection);
                }
                Err(about(&error))
            }
        }
    }

    /// How many sessions need the user, as the daemon counts them, or why it
    /// did not say. The same connection rules as `sessions`.
    pub async fn summary(
        &mut self,
    ) -> Result<corral_protocol::method::AttentionSummaryResult, Unanswered> {
        let mut connection = self.borrow_for_one_question().await?;
        let Ok(answered) = tokio::time::timeout(ANSWER, connection.attention_summary()).await
        else {
            return Err(Unanswered::Silent(format!(
                "nothing within {} seconds",
                ANSWER.as_secs()
            )));
        };
        match answered {
            Ok(summary) => {
                self.connection = Some(connection);
                Ok(summary)
            }
            Err(error) => {
                if matches!(error, RequestError::Refused(_)) {
                    self.connection = Some(connection);
                }
                Err(about(&error))
            }
        }
    }

    /// Acknowledge one item by the id this surface saw. What the daemon said
    /// back, in words a person reads.
    pub async fn acknowledge(&mut self, session: &str, item: &str) -> Result<(), Unanswered> {
        let mut connection = self.borrow_for_one_question().await?;
        let Ok(answered) =
            tokio::time::timeout(ANSWER, connection.attention_acknowledge(session, item)).await
        else {
            return Err(Unanswered::Silent(format!(
                "nothing within {} seconds",
                ANSWER.as_secs()
            )));
        };
        match answered {
            Ok(()) => {
                self.connection = Some(connection);
                Ok(())
            }
            Err(error) => {
                if matches!(error, RequestError::Refused(_)) {
                    self.connection = Some(connection);
                }
                Err(about(&error))
            }
        }
    }

    /// Drop a connection that cannot be asked another question.
    ///
    /// A daemon that went away has nobody on the other end, and one that broke
    /// the protocol may have left the stream at a place this client cannot
    /// find — asking again on either would answer the wrong question. A
    /// refusal is neither: the daemon answered, on a connection that is fine.
    ///
    /// The same rule `sessions` applies, because it is the same connection.
    pub fn forget_if_unusable(&mut self, error: &RequestError) {
        if !matches!(error, RequestError::Refused(_)) {
            self.forget();
        }
    }

    /// Drop the connection, whatever it was.
    pub fn forget(&mut self) {
        self.connection = None;
    }
}

#[cfg(test)]
#[path = "daemon_tests.rs"]
mod tests;
