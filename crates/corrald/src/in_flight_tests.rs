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
        joined(waiting).await,
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
        joined(waiting).await,
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

    assert!(joined(waiting).await.is_none());
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
