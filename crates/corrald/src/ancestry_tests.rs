use super::*;

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::SystemTime;

/// A process tree a test builds, so the shapes under test are the measured
/// ones rather than whatever this machine is running.
struct Tree {
    processes: HashMap<u32, Observation>,
}

impl Tree {
    fn new() -> Self {
        Self {
            processes: HashMap::new(),
        }
    }

    fn with(mut self, pid: u32, parent: u32, executable: &str) -> Self {
        self.processes.insert(
            pid,
            Observation::Identified(Box::new(ProcessIdentity {
                pid,
                parent,
                started: SystemTime::UNIX_EPOCH,
                executable: PathBuf::from(executable),
            })),
        );
        self
    }

    fn hidden(mut self, pid: u32) -> Self {
        self.processes.insert(pid, Observation::NotPermitted);
        self
    }

    fn walk(&self, from: u32, claimed: KnownProvider) -> Corroboration {
        walk(from, claimed, &|pid| {
            self.processes
                .get(&pid)
                .cloned()
                .unwrap_or(Observation::Gone)
        })
    }
}

fn reached(corroboration: &Corroboration) -> Option<&ProcessIdentity> {
    match corroboration {
        Corroboration::Reached { process, .. } => Some(process),
        _ => None,
    }
}

/// The measured Claude shape: the hook process, the `/bin/sh -c` Claude runs
/// it through, then the provider.
#[test]
fn a_claude_hook_reaches_its_provider_through_the_shell() {
    let tree = Tree::new()
        .with(300, 200, "/bin/dash")
        .with(200, 100, "/root/.local/share/claude/versions/2.1.252")
        .with(100, 1, "/bin/bash");

    let corroboration = tree.walk(300, KnownProvider::Claude);

    assert_eq!(
        reached(&corroboration).map(|process| process.pid),
        Some(200)
    );
}

/// The measured Codex shape: the notify program is spawned by the native
/// binary directly, and that binary sits one hop below its `node` wrapper.
#[test]
fn a_codex_notify_reaches_the_native_binary_beneath_its_wrapper() {
    let tree = Tree::new()
        .with(
            400,
            300,
            "/usr/local/lib/node_modules/@openai/codex/.../bin/codex",
        )
        .with(300, 200, "/usr/local/bin/node")
        .with(200, 1, "/bin/bash");

    let corroboration = tree.walk(400, KnownProvider::Codex);

    assert_eq!(
        reached(&corroboration).map(|process| process.pid),
        Some(400)
    );
}

/// A delivery names which provider's ingress it is. A walk that returned
/// whatever it found would let one provider's event corroborate itself
/// against another provider that happened to be an ancestor.
#[test]
fn a_chain_carrying_the_other_provider_does_not_corroborate() {
    let tree = Tree::new()
        .with(300, 200, "/bin/dash")
        .with(200, 1, "/usr/local/bin/codex");

    assert_eq!(
        tree.walk(300, KnownProvider::Claude),
        Corroboration::NotFound
    );
}

/// Providers spawn children that are not the agent — `git`, above all — so a
/// process being a provider's descendant is not recognition. Here the walk
/// starts at such a child and still has to reach the provider itself to say
/// anything.
#[test]
fn a_providers_unrelated_child_still_has_to_reach_the_provider() {
    let tree = Tree::new()
        .with(500, 400, "/usr/bin/git")
        .with(400, 1, "/usr/local/bin/codex");

    let corroboration = tree.walk(500, KnownProvider::Codex);

    assert_eq!(
        reached(&corroboration).map(|process| process.pid),
        Some(400)
    );
}

/// A hook is a short-lived child and the chain can be gone before it is read.
/// Nothing readable at all is unknown, which is not the same as looking and
/// finding no provider.
#[test]
fn a_chain_that_is_already_gone_is_unknown_rather_than_absent() {
    let tree = Tree::new();

    assert_eq!(
        tree.walk(999, KnownProvider::Claude),
        Corroboration::Unreadable
    );
}

#[test]
fn a_chain_this_account_may_not_inspect_is_unknown() {
    let tree = Tree::new().hidden(700);

    assert_eq!(
        tree.walk(700, KnownProvider::Claude),
        Corroboration::Unreadable
    );
}

/// A chain that was partly readable and ran out has been looked at. It says
/// no provider was found, which a later sweep may still contradict.
#[test]
fn a_chain_that_runs_out_partway_reports_nothing_found() {
    let tree = Tree::new().with(300, 250, "/bin/dash");

    assert_eq!(
        tree.walk(300, KnownProvider::Claude),
        Corroboration::NotFound
    );
}

/// The process tree is somebody else's data structure. A cycle in it must
/// cost a bounded walk, never the daemon.
#[test]
fn a_cycle_in_the_process_tree_terminates() {
    let tree = Tree::new()
        .with(10, 20, "/bin/dash")
        .with(20, 10, "/bin/dash");

    assert_eq!(
        tree.walk(10, KnownProvider::Claude),
        Corroboration::NotFound
    );
}

/// The bound is generous next to both measured chains and is still a bound.
#[test]
fn a_chain_longer_than_the_bound_stops_rather_than_walking_forever() {
    let mut tree = Tree::new();
    for pid in 1..100_u32 {
        tree = tree.with(pid, pid + 1, "/bin/dash");
    }
    tree = tree.with(100, 0, "/usr/local/bin/claude");

    assert_eq!(tree.walk(1, KnownProvider::Claude), Corroboration::NotFound);
}

/// A process whose parent is the reaper is the top of what can be walked.
#[test]
fn a_chain_that_reaches_the_top_stops_there() {
    let tree = Tree::new().with(50, 0, "/bin/dash");

    assert_eq!(
        tree.walk(50, KnownProvider::Claude),
        Corroboration::NotFound
    );
}
