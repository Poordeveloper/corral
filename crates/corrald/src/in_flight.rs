//! One mutating command executes once, however many times it arrives.
//!
//! Two different windows are open between a client sending a command and
//! learning what it did, and they need different answers.
//!
//! The **crash window** — the daemon dies after spawning and before its
//! receipt commits — is closed by ADR 0007 L6 rather than by anything here: a
//! managed runtime does not survive its owning daemon, so a retry against a
//! new daemon is a legitimate retry and not a second live runtime.
//!
//! > The safety of retrying an unreceipted `session.new` after daemon loss
//! > depends on ADR 0007 L6. Any future change that lets managed runtimes
//! > survive daemon loss MUST reopen the command crash-window design before
//! > that behaviour ships.
//!
//! The **concurrency window** — two retries arriving at one live daemon, both
//! reading "no receipt" before either commits — L6 does not touch, and this
//! table closes. The order is what makes it work, and the obvious arrangement
//! is the broken one: consulting the receipt first and inserting afterwards
//! lets both requests see "not found". So a claim is taken before the receipt
//! is consulted, and only the claim's owner ever consults it (grill Q8).

use std::collections::HashMap;
use std::sync::Mutex;

use corral_core::{Command, CommandFingerprint, CommandId, CorralSessionId, RunId};
use corral_protocol::ErrorCode;

/// The commands this daemon is executing right now.
///
/// Daemon-local and deliberately not durable: it answers a question about this
/// process's own concurrency, and a durable "in progress" claim would be a
/// receipt state the accepted vocabulary does not have.
#[derive(Default)]
pub struct InFlightCommands {
    claimed: Mutex<HashMap<CommandId, Slot>>,
}

struct Slot {
    /// What this command id already means. A different one is a conflict, not
    /// a second execution.
    fingerprint: CommandFingerprint,
    /// Where the owner's answer appears. A receiver rather than the sender:
    /// the sender lives with the owner, so an owner that dies without
    /// publishing closes this and tells every waiter to send the command
    /// again.
    concluded: tokio::sync::watch::Receiver<Option<Concluded>>,
}

/// What one execution of a command concluded, as its waiters need it.
///
/// `Accepted` rather than `Started`, because that is what the answer means: the
/// command was accepted and a managed Run was created. Whether that Run is
/// still running belongs to the Run's own lifecycle, and naming this variant
/// after execution would have invited every reader of this code to collapse
/// the two layers (ADR 0002 D6).
#[derive(Clone, Debug)]
pub enum Concluded {
    Accepted {
        session: CorralSessionId,
        run: RunId,
    },
    /// The command did not execute, and this is why. Carried so a waiter is
    /// told what the first caller was told, rather than a different story
    /// about the same command.
    Refused { code: ErrorCode, message: String },
}

/// What a request found when it claimed its command id.
pub enum Claim<'a> {
    /// This request executes the command.
    Owner(OwnedCommand<'a>),
    /// Another request is executing the same semantic command. Its answer is
    /// this one's answer.
    Waiting(tokio::sync::watch::Receiver<Option<Concluded>>),
    /// The id already names a different semantic command. Nothing is executed
    /// and nothing is waited for.
    Conflict,
}

/// The right to execute one command, and the duty to publish what it did.
///
/// The claim is released when this is dropped, whichever way the execution
/// ends. An owner that returns without publishing has completed nothing, so a
/// later retry may execute — and its waiters are told to send the command
/// again rather than handed an outcome nobody produced.
pub struct OwnedCommand<'a> {
    commands: &'a InFlightCommands,
    id: CommandId,
    concluded: tokio::sync::watch::Sender<Option<Concluded>>,
}

impl InFlightCommands {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Claim a command id, before anything about it is looked up.
    ///
    /// Atomic against every other claim, which is the whole point: the
    /// durable receipt is consulted by the owner and by nobody else, so two
    /// concurrent retries cannot both conclude that this command has not run.
    pub fn claim(&self, command: &Command) -> Claim<'_> {
        let mut claimed = self.lock();
        if let Some(slot) = claimed.get(command.id()) {
            if &slot.fingerprint != command.fingerprint() {
                return Claim::Conflict;
            }
            return Claim::Waiting(slot.concluded.clone());
        }

        let (sender, receiver) = tokio::sync::watch::channel(None);
        claimed.insert(
            command.id().clone(),
            Slot {
                fingerprint: command.fingerprint().clone(),
                concluded: receiver,
            },
        );
        Claim::Owner(OwnedCommand {
            commands: self,
            id: command.id().clone(),
            concluded: sender,
        })
    }

    fn release(&self, id: &CommandId) {
        self.lock().remove(id);
    }

    /// A poisoned lock means a holder panicked mid-insert. The map is a map of
    /// clonable values either way, and refusing to look would leave every
    /// later command unable to claim anything.
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<CommandId, Slot>> {
        self.claimed
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

impl OwnedCommand<'_> {
    /// Say what this command did, to everyone waiting on it.
    ///
    /// Published before the claim is released, so a waiter that arrived while
    /// this was executing reads the answer rather than racing to execute it
    /// again. One that arrives afterwards finds no claim and consults the
    /// durable receipt, which is the replay authority across a lost response.
    pub fn publish(&self, concluded: Concluded) {
        self.concluded.send_replace(Some(concluded));
    }
}

impl Drop for OwnedCommand<'_> {
    fn drop(&mut self) {
        self.commands.release(&self.id);
    }
}

/// Wait for the execution that already owns this command id.
///
/// `None` means its owner ended without publishing: nothing was completed, and
/// the honest answer is that the command may be sent again.
pub async fn joined(
    mut concluded: tokio::sync::watch::Receiver<Option<Concluded>>,
) -> Option<Concluded> {
    // Checks the value it already holds before waiting, so an owner that
    // published and released before this ran is not waited on forever.
    match concluded.wait_for(Option::is_some).await {
        Ok(answer) => (*answer).clone(),
        Err(_) => None,
    }
}

#[cfg(test)]
#[path = "in_flight_tests.rs"]
mod tests;
