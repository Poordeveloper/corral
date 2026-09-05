# Tray Grill — rulings over the macOS tray / watchfulness surface (round 1)

> Status: **open** (2026-09-05). Round 1 ruled the root decisions of the
> M1 completion task `ROADMAP.md` §3 names first: tray. The product law
> under grill is `PRODUCT.md` §7 (badge, grouping, *watchfulness ⇔ tray
> presence*, Quit warning) and ADR 0015's placement of notification
> delivery with the tray. No ADR is accepted here; nothing in
> `AGENTS.md` §Architectural changes is touched. One earlier client-lifecycle
> ruling is superseded: PR9 plan grill round 2, Q7 ("closing the last window
> is quitting") now holds only when no tray was established (Q1, Q8).
> Governing principle, ruled in Q1:
>
> There is one Desktop client process; tray presence, not window presence,
> determines whether that process may remain watchful with no windows.

Questions asked by the grill session of 2026-09-05; rulings verbatim below.

Facts the round was grounded in (found before asking, not decided):

- gpui 0.2.2 has no working status-bar item: `platform/mac/status_item.rs`
  is not declared as a module and references retired internals. It does
  expose `on_reopen`, `set_menus` / `set_dock_menu`, `hide`, `activate`,
  `on_window_closed`, `quit`, and hard-sets the activation policy to Regular
  at launch with no API to change it.
- `objc2` 0.6.4, `objc2-app-kit` 0.3.2 and `objc2-foundation` 0.3.2 are
  already in `Cargo.lock` through blade-graphics → gpui, so tauri's
  `tray-icon` (0.24.2) + `muda` (0.19.3) most likely add no second
  Objective-C stack on macOS; on Linux they need GTK, so they are
  target-scoped to macOS.
- macOS system notifications (`UNUserNotificationCenter`) require the
  process to run inside an `.app` bundle; a bare `cargo run` binary cannot
  deliver one. Notification delivery therefore depends on packaging.
- The wire already carries what a tray projects: `attention.summary`
  (`total` / `unacknowledged` per class) and per-session `items[]`
  (`attention_item_id`, `reason`, `since_unix_ms`, `acknowledged`); the
  Desktop polls both at 1 Hz; the daemon pushes nothing.
- `session.list` already carries `origin` (`managed` / `discovered` /
  `history`, absent = unknown) and `execution_state` (`running` / `exited`
  / `unknown`) as daemon facts.
- Every Corral crate is `#![forbid(unsafe_code)]`; unsafe is permitted only
  in a named platform boundary crate (`AGENTS.md` §Rust).

# Round 1

Digest of what round 1 froze:

| Q | Ruling |
|---|---|
| Q1 | **Single process.** `corral-desktop` hosts the status item; one corrald client, one 1 Hz poll, one projection, one acknowledge/open path. Window existence and application existence are independent facts. **Failure rule:** the process may be windowless-but-running only if the tray was successfully established; if tray initialization fails, log it, fall back to the non-watchful lifecycle, and closing the last window exits. Intent to create a tray is never watchfulness. No `corral-tray`, no second connection, no tray↔Desktop IPC. |
| Q2 | **macOS only.** Linux: no tray, no background watchfulness, last-window close keeps the current exit behaviour; an explicit capability gap recorded in the product/support/benchmark records, not a fake cross-platform abstraction. macOS-only direct dependencies are target-scoped so Linux graphs acquire no GTK. *No tray means no watchfulness claim.* |
| Q3 | `tray-icon` + `muda` is the **preferred mechanism pending the compatibility probe** — not accepted before it. Thread ownership rule: tray/menu callbacks never mutate gpui state from a callback thread; native event → small Corral message → gpui foreground handling. Probe passes → adopt, new direct dependencies, human review and merge under the dependency policy. Probe fails → **stop**; no automatic fall-through to Corral-owned objc2 unsafe bindings, which would be their own explicit decision. |
| Q4 | **Probe first**, as the plan's first item, disposable, no standalone spike PR; result recorded under `docs/references/`. Six measured cases: status item creation and clean teardown (exactly one item, none duplicated across close/reopen); the event bridge for click, Open Corral, New Session, Quit, with the callback thread/context recorded; windowless lifecycle repeated more than once (process alive, item responsive, polling active, Open Corral recreates the window); Dock reopen through gpui `on_reopen` on the same path; dynamic menu update while windowless with no stale ids or duplicate handlers; idle RSS / CPU / wakeups with the real 1 Hz poll, numbers recorded rather than thresholds invented. Pass → mechanism accepted; material failure → return with evidence and reconsider. No full implementation while hoping. |
| Q5 | **Tray now, OS notifications later** — a separate plan after packaging supplies a real `.app`; no disposable mini-packager. This grill also freezes notification *product policy* (which transitions notify, acknowledgement interaction, replacement/withdrawal, duplicate suppression) so the later task owns only mechanism, authorization, delivery and packaging integration. Three evidence windows are distinct: attention inference (may already accumulate), tray watchfulness (after this task), OS notification fidelity (cannot start before bundled delivery exists — a surface that never emitted a notification contributes no false-positive evidence). Interim Quit wording describes the capability that exists ("Corral will stop watching these sessions for attention"); the notification-aware wording arrives with delivery. |
| Q6 | Menu lists attention sessions, daemon-derived, no tray ranking. Header (disabled) "Needs You N · Ready M" from `attention.summary.total`, because the groups show every projected item including acknowledged ones (rows == header totals). Needs You group then Ready group, up to 10 each, daemon/session-list order, overflow "… k more in Corral" routing to the Desktop. Working / Unknown / Exited excluded. Row click: activate/reopen Desktop → select the `CorralSessionId` → the existing Open path; the tray click itself acknowledges nothing (Ready is acknowledged only by a successful Open; Needs You only by resolution or explicit acknowledgement). Presentation wording reused from `corral-client::presentation`. |
| Q7 | Status-item title = `needs_you.unacknowledged + ready.unacknowledged`; 0 → icon only; 1…99 exact; ≥100 → `99+` (presentation bound, not wire). Menu groups keep their own totals. *Badge counts unacknowledged attention, not blocked sessions and not attention rows.* |
| Q8 | Lifecycle accepted with four corrections. (1) Last-window close with a functional tray: window closes, process/tray/poll/watchfulness remain; with no functional tray: quit, per Q1. (2) Tray "Open Corral" and Dock `on_reopen` share one ensure-main-window-visible path (reuse or create, then activate). (3) Quit warning N comes from authoritative daemon facts — Corral-owned managed runtimes currently live — never from presentation heuristics; consume `session.list` facts if sufficient, else narrow a daemon projection. (4) Every user Quit path (tray Quit, ⌘Q, app menu, any Desktop Quit action) runs one gate: N == 0 → quit; N > 0 → one confirmation per attempt, Cancel keeps watchfulness, no persisted "already warned" flag. Quit never kills managed runtimes, never forces daemon shutdown, never rewrites runtime state. Dock policy stays gpui's Regular; no AppKit unsafe boundary merely to become an Accessory app. *Closing a window stops presentation; quitting Corral stops watchfulness; neither terminates managed work.* |

## Questions as asked (abridged)

- **Q1** — process model: the status item inside `corral-desktop` (one
  process, one client, one poll) or a separate `corral-tray` helper?
- **Q2** — platform: macOS only, Linux compile-only with no tray, or both?
- **Q3** — mechanism: `tray-icon` + `muda`; a Corral-owned objc2-app-kit
  binding in a new unsafe boundary crate; or reviving gpui's dead code?
- **Q4** — a disposable compatibility probe as the plan's first item, or
  straight to implementation?
- **Q5** — scope: tray only, with OS notifications as a second plan after
  packaging, or a minimal dev bundle so notifications ship now?
- **Q6** — menu: counts only, or grouped session rows with Open / New /
  Quit?
- **Q7** — status-item title: unacknowledged total, unacknowledged Needs You
  only, or icon only?
- **Q8** — close and Quit semantics replacing PR9 grill Q7: window close
  keeps the process; Quit warns once per attempt when managed sessions
  continue; Dock stays Regular.

## Founder rulings, verbatim

这轮我基本同意你的方向，但会收紧三处：Q3 的依赖选择要等 probe 才正式落槌；Q5 要把 tray watchfulness 和 OS notification 证据窗口拆开；Q8 要定义 tray 初始化失败以及所有 Quit 入口统一语义。
当前 GPUI 0.2.2 确实已有 `on_reopen`、`hide`、`activate`、Dock menu 等生命周期接口；而 `tray-icon` 在 macOS 要求状态栏 icon 在主线程、有运行中的 event loop 下创建，所以"GPUI + tray-icon 是否真能共存"正适合用你说的小 probe 做事实门。

### M1 tray grill — round 1 rulings

#### Q1 — process model

选择 A：
single `corral-desktop` process.
Tray/menu-bar presence and Desktop windows share:

* one process
* one corrald client
* one 1 Hz session/attention polling loop
* one current client-side projection
* one acknowledgement/open path

Window existence and application existence become independent facts.
On macOS after tray support is successfully initialized:
close final Desktop window
→ destroy/hide window
→ keep `corral-desktop` process alive
→ keep tray present
→ keep daemon connection/polling alive
Tray:
Open Corral
→ create/reopen Desktop window
Dock reopen:
→ same reopen path
Do NOT create:

* `corral-tray`
* second daemon connection
* second attention poller
* IPC between tray and Desktop
* a second single-instance problem

Important failure rule
The application may enter "windowless but still running" mode only if the
tray/watchfulness surface was successfully established.
If tray initialization fails:
→ Corral must NOT become an invisible background process
→ report/log tray failure
→ fall back to non-watchful Desktop lifecycle
→ closing the last window exits the Desktop process
This preserves:
Watchfulness ⇔ tray presence
Do not interpret:
"the code intended to create a tray"
as watchfulness.
Core invariant:
There is one Desktop client process; tray presence, not window presence,
determines whether that process may remain watchful with no windows.

#### Q2 — platform scope

选择：
macOS only for this task.
macOS:
→ tray/watchfulness supported when the implementation is validated
Linux:
→ no tray in this plan
→ no background watchfulness
→ closing the last Desktop window retains the current exit behavior
This is an explicit capability gap,
not a fake cross-platform abstraction.
Update the relevant product/support/benchmark record:
macOS tray:
validated/supported according to this task's evidence
Linux tray:
not implemented / unvalidated
Do not pull GTK / StatusNotifierItem / D-Bus dependencies into PR merely to
make the API look cross-platform.
The macOS-only direct dependencies must be cfg/target scoped so Linux
dependency/build graphs do not acquire GTK tray requirements.
If M1 public support later claims Linux watchfulness:
Linux tray implementation and real desktop validation become a support
gate for that claim.
Core invariant:
No tray means no watchfulness claim.

#### Q3 — tray mechanism

结构性首选：
`tray-icon` + `muda`
但不要在 probe 之前把它记录成最终 accepted mechanism。
正确状态：
preferred mechanism pending compatibility probe.
Why it is preferred:

* safe Rust API at Corral boundary
* no Corral-owned unsafe platform crate
* native NSStatusItem / NSMenu behavior
* existing objc2 ecosystem overlap
* much smaller platform ownership burden than hand-writing AppKit bindings

`tray-icon` / `muda` expose event handlers/channels intended to bridge menu
events into another application event loop; however GPUI is not winit/tao,
so their documented event-loop examples do not prove GPUI compatibility.
That is exactly what Q4 must establish.
Thread ownership rule
Tray/menu callbacks MUST NOT directly mutate GPUI state from an arbitrary
callback thread.
The probe/implementation must establish a safe bridge:
tray/menu callback
→ small Corral event/message
→ GPUI foreground/main-thread handling
→ window/app action
No GPUI object is captured and mutated from a non-GPUI thread merely
because the callback API accepts it.
Dependency gate
If the probe passes:
→ adopt `tray-icon` + `muda`
→ new direct dependencies
→ HUMAN_REVIEW_REQUIRED
→ human merge under existing dependency policy
If the probe fails:
STOP.
Do NOT automatically fall through to custom objc2 unsafe bindings.
Option (b):
Corral-owned AppKit boundary crate
would create a new named unsafe/platform boundary and deserves its own
explicit decision/review.
Core principle:
Prefer the maintained safe platform abstraction,
but prove it composes with GPUI before making it architectural baggage.

#### Q4 — probe first

选择 A。
The implementation plan begins with a disposable macOS tray compatibility
probe.
It does not need a standalone spike PR.
Its result is recorded in:
`docs/references/...tray...spike.md`
or the repository's corresponding research-evidence location.
Minimum measured cases:
1. Status item creation
GPUI application running normally
→ tray icon appears exactly once
Also verify:
dropping/shutting down tray ownership
→ status item disappears cleanly
No duplicate item after window close/reopen.
2. Event bridge
Exercise:

* clicking tray/status item if used
* Open Corral menu item
* New Session menu item
* Quit menu item

Prove:
native event
→ Corral bridge
→ GPUI foreground action
Record callback thread/context enough to establish the safe ownership
model.
3. Windowless lifecycle
Desktop window open
→ close final window
→ GPUI process remains alive
→ status item remains responsive
→ daemon polling remains active
→ Open Corral recreates window successfully
Repeat close/reopen more than once.
A one-shot success is insufficient for lifecycle correctness.
4. Dock reopen
With zero Desktop windows:
click/reopen through Dock
→ GPUI `on_reopen`
→ same window-opening path works
GPUI explicitly provides an `on_reopen` callback for already-running macOS
applications.
5. Dynamic menu update
Change synthetic attention counts/items while windowless:
→ menu/title updates correctly
→ old menu item ids/actions do not target the wrong session
→ no duplicate handlers
This is important because Corral's tray is not a static menu.
6. Idle resource usage
Measure windowless tray process:

* RSS
* idle CPU
* wakeup behavior if practical

with the real intended 1 Hz poll running.
Record numbers rather than inventing a threshold before measurement.
The question is:
is one windowless GPUI process operationally reasonable?
not:
can we optimize it to zero CPU in the spike?
Mechanism decision
If these pass:
`tray-icon + muda` becomes the accepted mechanism.
If lifecycle/event-loop integration fails materially:
→ return with evidence
→ reconsider mechanism/process architecture
Do not implement the full tray feature while hoping the probe result will
eventually become positive.

#### Q5 — tray vs system notifications

选择 A：
this plan delivers tray/watchfulness;
OS notifications are a separate plan after packaging supplies a real
macOS `.app` execution environment.
Do NOT add a disposable mini-packager merely to make this PR larger.
This tray plan delivers:

* status item
* attention badge
* attention menu
* Open/New actions
* windowless watchfulness
* Quit semantics

It does NOT claim:
macOS system notification delivery is implemented.
But notification policy should be decided before implementation
Agree with the proposed sequencing:
this grill may also freeze notification product policy
so the later notification task owns only:

* bundled macOS mechanism
* authorization behavior
* UNUserNotificationCenter delivery/replacement mechanics
* packaging integration

Do not make that later task rediscover:

* which transitions notify
* acknowledgement interaction
* replacement/withdrawal semantics
* duplicate suppression

Those belong to product policy.
Evidence-window consequence
Separate three milestones:
attention inference evidence
→ may already accumulate
tray/watchfulness evidence
→ can start after this task
OS notification fidelity evidence
→ CANNOT start until bundled system notifications actually exist
Therefore the release claim:
zero avoidable false Needs You notifications
cannot be satisfied by:
"our attention engine had zero false Needs You states before notification
delivery existed."
A surface that never emitted an OS notification contributes no notification
false-positive evidence.
So A intentionally creates:
tray
→ packaging
→ notifications
on the notification-release critical path.
That is acceptable;
do not hide it by building a throwaway packaging path.
Interim Quit copy
Before OS notification capability exists,
do not ship/test wording that falsely claims Corral currently sends system
notifications.
The final notification-era warning may say:
"Corral will no longer notify you when they need attention."
The tray-only intermediate build should use wording describing the
capability it actually has, e.g.:
"Corral will stop watching these sessions for attention."
or equivalent approved product wording.
Once notification delivery exists,
switch to the final notification-aware wording.
Core principle:
Tray presence establishes watchfulness.
System notification delivery is a separate capability and evidence claim.

#### Q6 — tray menu contents

接受列出 attention sessions。
Menu projection is daemon-derived;
the tray must not create its own attention engine or ranking.
Structure:
Header
Disabled/non-actionable summary:
Needs You N · Ready M
Use:
`attention.summary.total`
for N/M if the following groups display all currently projected items,
including acknowledged ones.
This keeps:
group rows == header totals
Do not use unacknowledged count here if acknowledged Needs You/Ready rows
remain visible.
Needs You group
Up to 10 sessions/items.
Each row:
Ready group
Same shape,
up to 10.
Within each group:
preserve daemon/session-list ordering
rather than implementing tray-specific ranking.
Overflow:
"… k more in Corral"
or equivalent product copy
→ activates/reopens Desktop attention view
Do not silently hide overflow with no route to it.
Excluded states
Do not put in tray attention list:

* Working
* Unknown
* Exited

Tray is the attention/watchfulness surface,
not the miniature full session manager.
Row click
Click session row:

1. activate/reopen Desktop;
2. select target CorralSessionId;
3. invoke the same existing Open path.

Tray click itself does NOT acknowledge anything.
Ready:
only successful Open under the already-frozen semantics
→ acknowledges the exact Ready AttentionItem
If Desktop opens but terminal/session Open fails:
→ Ready remains unacknowledged
Needs You:
viewing/Open
→ does NOT acknowledge
Only:

* actual resolution
* explicit acknowledgement of exact item

does.
Presentation wording
Reuse daemon/client presentation vocabulary.
Do not let tray code derive provider prose from raw hook/screen payloads.
Core invariant:
The tray projects attention truth;
it does not own or reinterpret it.

#### Q7 — status item badge/title

选择 A：
display total unacknowledged AttentionItems across Needs You + Ready.
Source:
attention.summary.needs_you.unacknowledged
+
attention.summary.ready.unacknowledged
This exactly matches the accepted badge definition.
When total = 0:
→ icon only
When total > 0:
→ icon + count
The menu itself still separates Needs You / Ready and uses the appropriate
group totals.
I would also freeze a presentation-only bound:
1…99
→ exact number
=100
→ `99+`
The daemon still reports the exact count;
only status-bar width is bounded.
This is not a wire semantic.
Acknowledging Ready can therefore:
Ready row remains visible
but status-item badge decreases
which is intended.
Acknowledging Needs You similarly clears its badge contribution without
pretending the blocker disappeared.
Core invariant:
Badge counts unacknowledged attention, not blocked sessions and not total
attention rows.

#### Q8 — close / Quit semantics

接受 proposed lifecycle with four corrections.
1. Last window close
On macOS with successfully initialized tray:
⌘W / red close
→ close the window
→ do NOT quit application
→ tray remains
→ poll/daemon client remains
→ watchfulness remains
With no functional tray:
last-window close
→ quit Desktop
per Q1 failure invariant.
Never leave an invisible watchful process with no tray affordance.
2. Reopen
Tray Open Corral
or
Dock `on_reopen`
→ one common ensure-main-window-visible path
That path should:

* reuse an existing window if one exists
* otherwise create one
* activate application appropriately

Avoid two independent "tray window" and "Dock window" constructors.
3. Quit warning criterion
Warning count N must be based on an authoritative daemon projection:
Corral-owned managed runtimes that are currently live and will continue
after the Desktop client quits.
Do NOT derive N from presentation heuristics such as:

* title
* provider
* whether Open is available
* whether row says Running

If current `session.list` already exposes the exact authoritative facts
required, consume those facts.
If it does not:
add/narrow a daemon-side projection sufficient to answer this question
rather than teaching Desktop to infer "managed".
The user-facing warning does not need to expose the internal term
"managed".
Conceptually:
"N sessions will continue running."
4. Every user-initiated Quit path uses one gate
The same quit-confirmation function must own:

* tray Quit Corral
* ⌘Q
* application menu Quit
* any Desktop action that means Quit

No route may bypass the warning merely because it originated outside the
tray.
"Warn once" means:
once per user Quit attempt while N > 0.
Flow:
Quit requested
→ compute/retrieve current N
→ N == 0 → quit
→ N > 0 → one confirmation
→ Cancel: continue watchfulness
→ Quit: exit Desktop
A later independent Quit attempt may warn again.
Do not persist a:
"user has already seen this warning forever"
flag.
Quit does not own runtimes
After confirmed Quit:

* Desktop exits
* tray disappears
* watchfulness ends
* managed provider processes continue
* corrald follows its own independent lifetime rules

Desktop MUST NOT:

* kill managed runtimes
* force daemon shutdown
* rewrite runtime state merely because UI exited

Dock policy
For M1 macOS tray:
keep GPUI's Regular application behavior.
Dock icon may remain visible with all windows closed.
Do not introduce an AppKit unsafe boundary merely to become a menu-bar-only
Accessory app.
Packaging/product polish can revisit this only with a separate explicit
decision.
Core invariant:
Closing a window stops presentation.
Quitting Corral stops watchfulness.
Neither action silently terminates managed agent work.

我认为这轮最重要的不是 tray 菜单长什么样，而是 Q1/Q8 的失败状态：
只有 tray 真存在，Desktop 才有资格在零窗口时继续活着。
否则某次 `tray-icon` 初始化失败后，用户关掉最后窗口，却留下一个没有任何 UI affordance 的后台 GPUI 进程，会直接违反你已经冻结的 `Watchfulness ⇔ tray presence`。
另外 Q5 选 A 没问题，但这意味着要从现在开始明确区分三种证据窗口：attention inference、tray watchfulness、OS notification fidelity。最后一个必须等 packaging + notifications 真落地以后才能计。
下一轮我预计真正需要裁的是：probe 通过标准/失败后的 fallback、菜单动态更新策略、Quit 时 daemon 不可达怎么办，以及系统通知政策本身。

## What round 1 leaves open

Round 2's frontier, unblocked by the rulings above: the probe's execution
protocol and pass criteria; the menu's dynamic-update strategy; the Quit
gate when the daemon is unreachable and the exact counting rule for N;
notification product policy; how acknowledged rows read in the menu; how a
tray initialization failure is shown, not only logged.
