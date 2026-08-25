use std::time::Duration;

use corral_core::{CommandFingerprint, CommandId, CommandKind};

use super::*;

fn command(id: &str, cwd: &str) -> Command {
    Command::new(
        CommandId::new(id).expect("usable"),
        CommandFingerprint::builder(CommandKind::new("session.new").expect("usable"))
            .input("cwd", cwd)
            .build(),
    )
}

/// The order that makes this work: the claim is taken before anything about
/// the command is looked up, so a second arrival cannot also conclude that
/// nothing has run.
#[test]
fn a_second_arrival_waits_rather_than_executing() {
    let commands = InFlightCommands::new();
    let command = command("cmd-1", "/work");

    let Claim::Owner(_owner) = commands.claim(&command) else {
        panic!("the first arrival executes");
    };

    assert!(matches!(commands.claim(&command), Claim::Waiting(_)));
}

/// One command id names one semantic command. A different one is refused
/// without executing and without waiting for something that will never mean
/// this.
#[test]
fn the_same_id_with_a_different_command_conflicts() {
    let commands = InFlightCommands::new();
    let _owner = commands.claim(&command("cmd-1", "/work"));

    assert!(matches!(
        commands.claim(&command("cmd-1", "/elsewhere")),
        Claim::Conflict
    ));
}

#[tokio::test]
async fn a_waiter_is_answered_with_what_the_owner_published() {
    let commands = InFlightCommands::new();
    let command = command("cmd-1", "/work");
    let Claim::Owner(owner) = commands.claim(&command) else {
        panic!("the first arrival executes");
    };
    let Claim::Waiting(waiting) = commands.claim(&command) else {
        panic!("the second arrival waits");
    };

    let session = corral_core::CorralSessionId::mint();
    let run = corral_core::RunId::mint();
    owner.publish(Concluded::Accepted { session, run });

    assert!(matches!(
        joined(waiting, Duration::from_secs(5)).await,
        Some(Concluded::Accepted { session: answered, run: ran })
            if answered == session && ran == run
    ));
}

/// Published before the claim is released, so a waiter that arrived during the
/// execution reads the answer instead of racing to execute it again.
#[tokio::test]
async fn an_answer_published_before_the_claim_is_released_still_reaches_a_waiter() {
    let commands = InFlightCommands::new();
    let command = command("cmd-1", "/work");
    let Claim::Owner(owner) = commands.claim(&command) else {
        panic!("the first arrival executes");
    };
    let Claim::Waiting(waiting) = commands.claim(&command) else {
        panic!("the second arrival waits");
    };

    let session = corral_core::CorralSessionId::mint();
    owner.publish(Concluded::Accepted {
        session,
        run: corral_core::RunId::mint(),
    });
    drop(owner);

    assert!(matches!(
        joined(waiting, Duration::from_secs(5)).await,
        Some(Concluded::Accepted { session: answered, .. }) if answered == session
    ));
}

/// An owner that ends without publishing completed nothing. Its waiters are
/// told to send the command again rather than handed an outcome nobody made.
#[tokio::test]
async fn a_waiter_whose_owner_vanished_is_told_nothing_completed() {
    let commands = InFlightCommands::new();
    let command = command("cmd-1", "/work");
    let Claim::Owner(owner) = commands.claim(&command) else {
        panic!("the first arrival executes");
    };
    let Claim::Waiting(waiting) = commands.claim(&command) else {
        panic!("the second arrival waits");
    };

    drop(owner);

    assert!(joined(waiting, Duration::from_secs(5)).await.is_none());
}

/// Releasing the claim is what lets a later retry consult the durable receipt,
/// which is the replay authority once this daemon no longer remembers.
#[test]
fn a_released_claim_leaves_the_id_available_again() {
    let commands = InFlightCommands::new();
    let command = command("cmd-1", "/work");
    {
        let Claim::Owner(owner) = commands.claim(&command) else {
            panic!("the first arrival executes");
        };
        owner.publish(Concluded::Refused {
            code: ErrorCode::Busy,
            message: "not now".to_owned(),
        });
    }

    assert!(matches!(commands.claim(&command), Claim::Owner(_)));
}

/// A waiter is bounded. An owner that never finishes hands its client no
/// answer at all, and "send it again" is a true statement about an idempotent
/// command — where waiting forever is not a statement about anything.
#[tokio::test]
async fn a_waiter_gives_up_on_an_owner_that_never_finishes() {
    let commands = InFlightCommands::new();
    let command = command("cmd-1", "/work");
    let _owner = commands.claim(&command);
    let Claim::Waiting(waiting) = commands.claim(&command) else {
        panic!("the second arrival waits");
    };

    // A short deadline rather than the production one: what is under test is
    // that waiting is bounded at all, and `JOIN_DEADLINE` is the policy for
    // how long, not the behaviour.
    assert!(joined(waiting, Duration::from_millis(50)).await.is_none());
}

/// Giving up on the wait must not give a second request the right to execute.
///
/// This is where a bounded join could go wrong: the waiter is told to send the
/// command again, and if its retry found the id unclaimed it would spawn a
/// second runtime while the first is still going — the duplicate the whole
/// mechanism exists to prevent. The claim belongs to the execution, not to
/// whoever is waiting on it, so it outlives every waiter.
///
/// The durable receipt is the other guard, and it is not enough on its own: an
/// owner that has not committed yet has no receipt for a retry to find.
#[tokio::test]
async fn a_join_that_timed_out_does_not_let_the_command_execute_twice() {
    let commands = InFlightCommands::new();
    let command = command("cmd-1", "/work");
    let _owner = commands.claim(&command);
    let Claim::Waiting(waiting) = commands.claim(&command) else {
        panic!("the second arrival waits");
    };

    assert!(joined(waiting, Duration::from_millis(50)).await.is_none());

    assert!(
        matches!(commands.claim(&command), Claim::Waiting(_)),
        "the execution still owns its command id, so a retry waits rather than \
         starting a second runtime"
    );
}
