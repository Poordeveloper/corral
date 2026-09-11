---
status: blocked  # D0's probe gates everything after it; three decisions in §Decisions needed first
class: B         # implements accepted policy (PRODUCT §7, ADR 0015 D7) on a mechanism grill Q12 accepted conditionally
writes: [crates/corral-macos, crates/corral-desktop, Cargo.toml, ARCHITECTURE.md, ROADMAP.md, docs/references]
reads: [docs/decisions/2026-09-06-m1-completion-grill.md, docs/decisions/2026-09-05-tray-grill.md, docs/adr/0015-attention-derivation.md, docs/plans/done/2026-09-05-tray.md, docs/plans/2026-09-11-m1-packaging.md, PRODUCT.md]
---

# M1 notifications — the macOS delivery of an attention item, and the fidelity window it opens

## Status

**Policy accepted** and not reopened here: PRODUCT §7 fixes what notifies
and what never does; ADR 0015 D7 fixes items, acknowledgement, and
eligibility. **Mechanism accepted conditionally** by completion grill Q12:
`UNUserNotificationCenter` via `objc2-user-notifications`, sealed only by a
Design-0 probe inside a real packaged `.app` under the real bundle
identity. This plan materializes those and decides how delivery is driven,
where the Objective-C boundary lives, and what the fidelity window counts.

Blocked on two things: the packaging PRs, which supply the `.app` the probe
needs, and the three rulings in §Decisions.

## Goal

A person who is not looking at Corral learns that a session needs them,
once, from the operating system, and one click puts them in that session.
The third ROADMAP evidence window — OS notification fidelity — can then
start.

## Non-goals

Linux notifications: M1 has no Linux tray (grill Q10), and a notification
without a persistent surface has nowhere to come from. Recurring reminders,
snooze, per-session or per-class preferences, quiet hours, notification
history, and Focus-mode integration (PRODUCT §7 excludes the first two;
the rest are M2 ergonomics). Notification *content* beyond the shared
projection's words — no transcript excerpt, no answer-from-the-banner,
which is M2's `NeedsInputRequest`. Any change to what an attention item is,
when one is born, or what may decide a main state: that is ADR 0015's, and
a notification plan that widened it would be deciding attention policy in a
delivery task. `AttentionReason::RuntimeEnded` stays reserved — whether an
exit ever notifies is not this plan's to answer.

## Existing owner / architecture involved

- **Eligibility is already structural.** `Claim::entitlement`
  (`corral-core`) refuses Heuristic association (`AssociationTooWeak`) and
  unsealed interpretation (`Unsealed`) before a claim can reach a main
  state, and an attention item is minted only on entry to Needs You or
  Ready (`corrald::attention::session`). So every item that appears on the
  wire is one PRODUCT §7 permits a notification for. The Desktop needs no
  eligibility rule, no new wire field, and derives nothing: it decides
  *whether it has already shown this item*, which is presentation state.
- **The item id is already on the wire.** `AttentionItemFacts` carries
  `attention_item_id` and `acknowledged`; `present_at` exposes them through
  `SessionPresentation::current_item`. A new id becoming current is exactly
  the daemon's `ItemBorn`/`ItemReplaced`, seen from outside.
- **The journal's `notifiable` is diagnostics, not a channel.** It records
  that a transition was one a notification may be emitted for (ADR 0015 D8:
  the journal is never product truth, and nothing reads it back). Delivery
  is not driven from it, and the cold-start suppression the grill describes
  is the *deliverer's* rule, not the journal's — which is why grill Q2
  forbids filtering the release count by it.
- **The Watch already polls and publishes generations.** `Bridge::poll` →
  `Polled` → `SessionList::take` → `Watch::publish`, at 1 Hz, with the
  status item rebuilt only when the projection changed (tray plan D3). The
  notification decision belongs on that same path: one generation in, a set
  of instructions out.
- **Watchfulness equals tray presence** (PRODUCT §7). Notifications exist
  while the Desktop process does, and stop when the person quits the tray —
  which the Quit gate already warns about in those words.
- **`corral-desktop` forbids unsafe** and every other crate does too.
  `objc2` 0.6.4 and the 0.3.2 framework crates are already in the lockfile
  through gpui and `tray-icon`; `objc2-user-notifications` 0.3.2 is the
  same generation. A `UNUserNotificationCenterDelegate` is a custom
  Objective-C class, so it needs `unsafe`, and grill Q12 says that stops at
  a named macOS platform-boundary crate rather than entering
  `corral-desktop`.

## Design

### D0 — Probe, before anything is sealed (Q12)

A throwaway binary inside a real signed `Corral.app`, under
`com.carriez.corral`, recorded in
`docs/references/2026-09-1X-notification-probe.md`. It proves, or returns
with evidence and no fallback to `NSUserNotification` or `osascript`:

1. settings readable; authorization requestable; granted, denied, and
   previously-denied distinguishable; no second prompt after a denial;
2. Needs You with sound, Ready silent, replacement on a session key,
   explicit withdrawal, withdrawal of an acknowledged item;
3. the click path end to end — callback → main context →
   activate/reopen → the right `CorralSessionId` → the normal Open path —
   with the window visible **and** windowless;
4. delegate ownership, lifetime, and main-thread bridging stated, with the
   unsafe confined to the boundary crate.

Nothing below merges before this passes. The probe is the mechanism's
acceptance evidence, and it is throwaway code.

### D1 — `corral-macos`: the one crate allowed unsafe

A new crate, the macOS platform boundary the Desktop talks to. Charter, in
its own `AGENTS.md`: Objective-C framework code that a safe Rust API cannot
otherwise reach, one module per framework, Corral vocabulary on the
boundary and no Objective-C type crossing it. It is not a utilities crate:
each module names the framework it wraps and the product capability it
serves. Today it holds `user_notifications` alone; the tray's `tray-icon`
and `muda` stay where they are, because a safe wrapper already exists for
them and moving working code is not this task.

The API is deliberately small and synchronous-looking:

```text
Authorization::current() -> Settings          read, never prompts
Authorization::request() -> Granted | Denied  prompts at most once ever
Notifications::post(Delivery)                 session-keyed identifier
Notifications::withdraw(&SessionKey)
Notifications::clicks() -> Receiver<SessionKey>
```

`#![forbid(unsafe_code)]` is lifted here and only here, per `AGENTS.md`
§Rust; every block carries a `// SAFETY:` comment, which
`undocumented_unsafe_blocks = "deny"` already enforces workspace-wide.
`check-dependency-direction` gains the rule that nothing depends on this
crate except `corral-desktop`.

### D2 — `notify::Decision`: a pure function of two generations

The whole delivery policy is a value computed in `corral-desktop`, with no
Objective-C anywhere near it:

```text
Decision::between(previous: &Shown, current: &SessionList) -> Vec<Instruction>
Instruction = Post { session, class, title, body } | Withdraw { session }
```

Rules, each testable on its own:

- **A new item id for a session posts once.** The same id in the next
  generation does nothing, however many generations it survives. A
  different id for the same session replaces (§D5).
- **Baseline, not backlog.** The first generation after the process starts,
  and the first after a reconnect or a refused/silent gap, establishes what
  is current and posts nothing. A daemon restart mints new ids for every
  session; without this rule reconnecting would ring for the whole list, and
  the grill's "discovered at a cold-start or reconnect baseline" is exactly
  this suppression seen from the journal's side.
- **Invalidation never rings** (PRODUCT §7): an item that ended, rotted, was
  superseded, exited, or was acknowledged produces `Withdraw`, never `Post`.
- **Acknowledgement withdraws.** `acknowledged` arriving true for the
  current item withdraws the banner and posts nothing.
- **A row Corral cannot read, or a session that left the list,** withdraws.
  A list that did not arrive at all changes nothing: silence is not
  resolution, and the next answered generation is a baseline.

### D3 — Authorization, asked once

`Authorization::current()` at Desktop start. Not determined → request once,
at first launch rather than at the first event, because a prompt that
arrives with the banner delays exactly the thing it is asking about.
Denied or previously denied → Corral never asks again; the tray menu
carries one line — *Notifications are off · open System Settings* — and
the badge and menu keep working, because the tray is the surface that
does not need permission. Granted is silent. The state is read at every
start, so turning notifications back on in System Settings needs no
reinstall and no reset.

### D4 — What a banner says

From the shared projection, never a second vocabulary (grill Q2, the rule
that made `corral list` and the TUI agree):

```text
title   the session's title
body    the state line the tray row shows — "Needs you · permission"
sound   Needs You only; Ready is silent (Q12)
```

No count, no aggregate banner: the badge counts, a banner names one
session. `AttentionReason` decides the class and nothing else; a reason
this build has no word for does not notify, the same way an unknown wire
value renders as unknown rather than guessed.

### D5 — One live banner per session

The notification identifier is the `CorralSessionId`, so a new item for a
session replaces the banner in place instead of stacking — the behaviour
Q12 requires proving. The `AttentionItemId` rides in the payload as the
thing the click and the withdrawal name, so a withdrawal that races a
replacement cannot remove the newer banner: a `Withdraw` carrying a stale
item id is dropped.

### D6 — The click path

Delegate callback (any thread AppKit chooses) → an unbounded channel,
the same shape `tray_macos` already uses for menu clicks → the Watch
reads it on the foreground → `ensure_main_window` → select the
`CorralSessionId` → the normal Open path. The banner does nothing the
menu cannot: no new control surface, no answering from the notification. A
click on a session that is gone opens the window and says so, rather than
failing silently.

### D7 — Evidence: the fidelity window

The third ROADMAP window. The Desktop's diagnostic log (grill Q20, already
the Desktop's own and never the daemon's) gains one line per instruction —
timestamp, pid, generation, session, class, post/withdraw, result — and
carries no attention content. From it the window reports: banners posted,
withdrawn, clicked; posts that failed; and the one number that matters,
banners the person judged wrong, which they record with
`corral attention dispute` exactly as they do for the attention window.
There is no separate dispute channel for notifications: a false banner is a
false item, and the journal already has the word for it.

### D8 — Records

`ARCHITECTURE.md` §11 gains **Delivered notification**: the OS banner for
one session's current attention item, keyed by `CorralSessionId`, replaced
rather than stacked, withdrawn on invalidation, and never itself a state —
a projection of an item, exactly as the badge is a projection of counts.
`ROADMAP.md` §1's "notifications" line becomes what shipped. PRODUCT §7
already states the policy and does not change.

## Interfaces or persistence changed

No wire change: eligibility and item identity are already carried, which is
what makes this a client-only task. No durable state. One new crate, one
new dependency (`objc2-user-notifications` 0.3.2 — the same objc2
generation already in the lockfile; the alternatives were `notify-rust`,
which on macOS shells out to a deprecated API, and hand-rolled bindings,
which is the unsafe this crate exists to bound). New user-visible
behaviour: a system prompt at first Desktop launch.

## Failure / unknown states

Authorization denied → no banners, tray unchanged, one line saying so, and
never a second prompt. Post fails → logged with its error, the badge and
menu are unaffected, and the next generation does not retry the same item:
a banner is a projection of a current fact, and a retried one would arrive
after the fact changed. Delegate never installed → clicks are lost rather
than misrouted, and the log says the delegate is absent. The daemon goes
away → no generations, so nothing posts; the existing banners stay until
the next answered generation withdraws what is gone, because a lost
connection is not evidence that anything resolved. The Desktop quits → the
OS clears its banners; watchfulness ended, which the Quit gate already
says. macOS older than the deployment target, or a build that is not a
bundle → `Authorization::current()` reports unavailable, the Desktop logs
it once and runs without notifications rather than refusing to start.

## Tests

- `Decision::between` unit tests over constructed generations, one per
  rule: new id posts; repeated id is silent; replaced id replaces; first
  generation is a baseline; first generation after a gap is a baseline;
  acknowledged withdraws; ended/rotted/exited withdraws; a session leaving
  the list withdraws; an unanswered generation changes nothing.
- A regression test for the reconnect burst: two generations whose item ids
  all differ, with a gap between them, post nothing.
- `corral-macos` is exercised by the probe and by the dogfood window, not
  by CI: a banner needs a bundle, a logged-in session, and a person to see
  it. The crate's surface is kept narrow precisely so that what cannot be
  tested mechanically is small and the policy above is not inside it.
- No test asserts that a real banner appeared; that claim belongs to the
  probe record and the evidence window.

## Definition of done

- D0's probe passes and is recorded, or the mechanism returns for a ruling.
- `corral-macos` exists with its charter, its `SAFETY` comments, and the
  dependency-direction rule.
- `Decision` and its tests are merged; the Watch drives delivery from its
  existing poll path.
- Authorization, content, replacement, withdrawal, and the click path work
  in a signed `.app` on the founder's machine, windowed and windowless.
- The diagnostic log carries the instruction lines the window reads.
- `./scripts/verify` green on the final tree; the notification-fidelity
  window can start.

## Decisions needed before implementation

1. **The crate name and charter.** `corral-macos` as D1 describes it, or a
   narrower `corral-user-notifications` that would be joined by a sibling
   crate the next time a framework is needed. Recommendation: `corral-macos`
   — the boundary is "Objective-C the Desktop needs", and one bounded crate
   with a written charter is easier to police than a growing family.
2. **When authorization is requested.** At first Desktop launch (D3), or
   lazily at the first eligible item. Recommendation: first launch. Lazy is
   politer in the abstract and worse here — the first banner is the one
   that proves the product, and it would be replaced by a permission
   dialog.
3. **Whether a banner appears while Corral is frontmost.** The convention is
   to suppress; Corral's own list may be open on a different session.
   Recommendation: always present. A rule that depends on focus makes the
   fidelity window measure two different behaviours, and the person who is
   looking at Corral is not harmed by a banner naming a session they are
   not looking at.

## Plan Size Justification

Over the ~150-line target, and one scope: everything here serves a single
invariant — *an eligible item the person has not seen produces exactly one
banner, and anything that invalidates it takes that banner away*. The parts
cannot be split without breaking it. A crate with no policy is untestable
and unreviewable on its own; a decision function with nothing to drive
proves nothing; a click path without withdrawal delivers banners that
outlive their facts, which is the failure PRODUCT §7 names first. The probe
(D0) is separable in time but not in review: it is what makes the mechanism
acceptable, and merging the implementation before it would seal by
accident. The only genuinely separate piece is the evidence window (D7),
which continues after this task ends and belongs to `m1-release-gate`.
