# Tray Grill — rulings over the macOS tray / watchfulness surface (rounds 1–2)

> Status: **design frontier cleared** (2026-09-05). Round 1 ruled the root
> decisions of the M1 completion task `ROADMAP.md` §3 names first: tray;
> round 2 ruled the probe protocol, menu rebuild policy, the Quit gate's
> counts and unreachable case, notification product policy (frozen here;
> the later notification task owns mechanism only), the unacknowledged-row
> marker, and the visible tray-failure mode. What remains is evidence, not
> design: the probe's measured result, whether windowless resource use is
> acceptable, the sealing of `tray-icon` + `muda` as the mechanism, and the
> plan's move from conditional to accepted. The product law
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

# Round 2

Digest of what round 2 froze:

| Q | Ruling |
|---|---|
| Q9 | **Probe protocol accepted.** Disposable prerequisite, never a product artifact. Automated: status item creation; ≥3 programmatic close/reopen cycles; tray responsive while windowless; synthetic projection changes every ~5 s covering reorder, disappearance, replacement, same session with a changed state, and an old item id replaced by a new one; callback thread/context diagnostics; windowless RSS / CPU / wakeups. Human, in an unlocked Aqua session: click the status item, Open Corral, New Session…, Quit Corral, Dock reopen — each leaving an unambiguous log record. Pass = round 1 cases 1–5 behave per the accepted lifecycle, with no duplicate handler, no stale index targeting the wrong session, no duplicate status item, no reopen failure after repeated cycles, no windowless process without a functioning tray. Resources are measured, not thresholded; a surprising number stops the work and comes back as evidence rather than being accepted because no threshold existed. The plan is written *conditionally accepted pending Design 0 tray probe*, its mechanism section saying *preferred: tray-icon + muda; final: pending probe*; a pass plus one reconciliation seals the mechanism with no second grill; a material failure stops and reopens only the affected mechanism/process decision. |
| Q10 | **A′ — rebuild the whole menu when the structural projection changes.** The structural `TrayProjection` is a pure value: badge total, Needs You total, Ready total, ordered rows per group (session id, current item id, acknowledged, title, reason/context, attention-entered timestamp), overflow counts. Unchanged structure → no rebuild on the 1 s poll. Humanized age is presentation and must not turn the poll into a per-second rebuild: rebuild only when the displayed age bucket changes. A rebuild constructs a complete new generation — items and action bindings — published as one swap; never visuals then mappings. Every actionable row binds the `CorralSessionId` directly, never a group/row index or menu position; the item id may travel too, but Open is session-oriented. A stale generation's click resolves the session id, activates the Desktop, uses current daemon truth, and converges to current state or the no-longer-available behaviour — never applies the old action to another session. |
| Q11 | **Two counts, never merged.** R = `origin == managed ∧ execution_state == running`; U = `origin == managed ∧ execution_state == unknown`; exited, discovered, history, and unknown origin are not counted. Warn when R > 0 or U > 0. R > 0, U == 0: "R sessions will continue running. Corral will stop watching them for attention." U > 0 keeps the uncertainty: "R sessions are still running. Corral couldn't verify whether U other sessions it started have ended. Corral will stop watching for attention." — omitting the first sentence when R == 0, never saying unknown sessions continue or stopped. Facts come from the most recent successful `session.list` generation on the currently healthy connection; no new RPC; a generation from before the connection became unavailable is never treated as current. Unreachable: never N = 0 from stale or missing data; uncertainty copy "Corral can't reach its service, so it can't tell whether sessions it started are still running. Corral will stop watching for attention." [Quit] [Cancel]; the user may still quit. One gate for ⌘Q, the app menu, and the tray. *Conservative warning must not become a false Running claim.* |
| Q12 | **Notification product policy frozen** (items 1–9 accepted with precision). Only a newly observed current Needs You or Ready item notifies; Working, Unknown, Exited, expiry, resolution, and removal are silent. New = absent from the previous successful projection and present in the next, on one continuously healthy surface. Cold start and reconnect: the first successful generation is a silent baseline — existing items reach badge and menu without a burst. **Accepted cost:** an item born and still active entirely across a surface disconnect may never produce an OS notification; this is precision-first M1 behaviour and is never recovered by guessing cross-reconnect identity from session/reason fingerprints. Replacement is session-scoped (a new item for the same session replaces the prior notification and delivers on its own; identities stay distinct); withdrawal is silent on disappearance and on acknowledgement, which changes no semantic state. Click = the tray row action (activate, select, Open; Ready acknowledged only by a successful Open). No foreground suppression in M1. Needs You sounds, Ready is a silent banner, subject to the macOS API and Focus. Content: approved title, `corral-client::presentation` wording plus sanitized `NeedsInputContext`; never raw transcript, prompt, tool arguments, hook payload, or capture. No preference system. Evidence: OS-notification fidelity counts only items observed continuously, never items discovered at a baseline. |
| Q13 | **✓ rejected; mark the unacknowledged row, not the acknowledged one.** A native checkmark reads as selected/done, and an acknowledged Needs You may still be blocked. Unacknowledged rows carry a subtle unread marker (a leading `•` or equivalent native-safe affordance); acknowledged rows are plain. Glyph is presentation; the semantic is frozen: the marker means "still unacknowledged", never "resolved". No "Acknowledged" suffix prose unless usability evidence demands it; if the toolkit cannot distinguish cleanly, prefer no marker over a misleading checkmark. |
| Q14 | **Accepted.** On macOS a tray failure disables watchfulness for the process run: last-window close quits, no automatic retry, diagnostics logged, and a persistent main-window banner "Menu bar icon unavailable — closing this window quits Corral." for the process lifetime; a restart is a fresh attempt. Tray initialization is attempted before the window lifecycle policy is established, so the first window already knows which rule it lives under; a failure before window construction is carried into the window/banner model. Linux never shows this banner: no tray there is a known unsupported capability, not a failure. |

## Questions as asked (abridged, round 2)

- **Q9** — probe protocol: which cases are automated, which the founder
  performs in an unlocked Aqua session, what passes, and how the plan's
  status moves.
- **Q10** — menu updates: rebuild whole on change, diff in place, or
  rebuild every poll?
- **Q11** — Quit gate: the counting rule for N and what to do when the
  daemon is unreachable.
- **Q12** — notification product policy, nine items, frozen now so the
  later task owns mechanism only.
- **Q13** — acknowledged rows: native ✓, no distinction, or suffix text?
- **Q14** — tray initialization failure: log only, or a visible mode
  indicator?

## Founder rulings, verbatim (round 2)

这轮我会裁：

* Q9 = 接受
* Q10 = A′：整体重建，但比较的是结构投影；"时长"不能导致每秒重建
* Q11 = 修改：running 与 unknown 必须分开计，不能把 unknown 包进"will continue running"；Quit 前用最新 daemon generation，daemon 不可达走不确定文案
* Q12 = 基本全部接受，并明确"重连 baseline 静默"有意允许漏掉断线期间新生的通知
* Q13 = 不接受 ✓；改为标记未确认项，而不是把已确认 Needs You 画成"完成"
* Q14 = 接受

### Tray grill — round 2 rulings

#### Q9 — probe execution protocol

接受。
Probe remains a disposable implementation-plan prerequisite,
not a product artifact.
Required executable cases:
Automated

* create status item
* programmatically close/reopen Desktop window at least 3 cycles
* keep tray responsive while windowless
* synthetic attention projection changes every ~5 s
* include:
   * reorder
   * item disappearance
   * item replacement
   * stable session id with changed state
   * old item id replaced by new item id
* collect callback thread/context diagnostics
* collect windowless RSS / CPU / wakeup observations

Human Aqua interaction
In an unlocked real macOS Aqua session:

* click status item/menu
* Open Corral
* New Session…
* Quit Corral
* Dock reopen

Each action must leave a corresponding unambiguous log/event record.
Pass criteria
Cases 1–5 from round 1 must all behave according to the accepted
single-process lifecycle.
No:

* duplicate handler
* stale index targeting the wrong Session
* duplicate status item
* reopen failure after repeated close/reopen
* windowless process without a functioning tray

Resource case:
measurement only
No pre-invented RSS/CPU threshold.
If resource use is surprising enough to materially undermine the
single-process model:
STOP
→ bring the measurement back
rather than silently accepting it because no numerical threshold existed.
Plan status
The tray implementation plan may be written as:
conditionally accepted pending Design 0 tray probe
where the mechanism section explicitly says:
preferred: tray-icon + muda
final mechanism: pending probe
Probe passes
→ evidence recorded
→ one reconciliation confirms the measured facts match the plan
→ mechanism becomes accepted
→ no second full grill
Material probe failure
→ STOP
→ reopen only the affected mechanism/process decision.
Core principle:
The probe decides whether the preferred mechanism actually satisfies the
already-accepted lifecycle; it does not reopen the tray product semantics.

#### Q10 — dynamic menu updates

选择 A′：
rebuild the complete tray menu when the meaningful TrayProjection changes.
But distinguish:
structural projection
from:
continuously changing rendered age text.
Structural TrayProjection
Conceptually contains:

* unacknowledged badge total
* Needs You total
* Ready total
* ordered Needs You rows:
   * CorralSessionId
   * current AttentionItemId
   * acknowledged
   * title
   * reason/presentation context
   * attention entered timestamp
* ordered Ready rows
* overflow counts

This is a pure value projection of daemon truth.
If the structural projection is unchanged:
do NOT rebuild NSMenu every 1-second poll.
Age display
Human-readable age:
"2m"
"15m"
"1h"
is presentation.
It must not accidentally turn:
poll every 1 s
into:
destroy/recreate native menu every 1 s.
Choose a bounded display bucket policy,
for example rebuild only when the displayed humanized age bucket changes.
Exact formatting remains presentation policy.
Therefore steady-state tray objects should normally remain stable across
most 1-second polls.
Generation ownership
When rebuild is required:
new projection
→ construct complete new menu generation
→ construct all item action bindings for that generation
→ publish/swap as one logical generation
→ discard old generation
Do not:
replace visual items
then later replace id→session mappings
or vice versa.
Each actionable row must carry/bind:
CorralSessionId
directly.
Never resolve click by:
group index
row index
current menu position
AttentionItemId may also travel with the action when needed,
but Open remains session-oriented.
Stale click
A menu can be open while daemon truth changes.
That is allowed.
Clicking an older visible menu generation:
→ resolve stable CorralSessionId
→ activate Desktop
→ fetch/use current daemon truth
→ attempt the normal Open path
If the session/item no longer exists or is no longer actionable:
→ do not apply the old action to another session
→ simply converge to current Desktop state / appropriate no-longer-available
behavior
The menu snapshot being stale must never produce identity confusion.
Core invariant:
Rebuilding the native menu is an implementation detail.
Stable session identity, not menu position, owns the action.

#### Q11 — Quit count and daemon-unreachable behavior

修改 proposed counting rule。
Do NOT define:
N =
managed Running
+
managed Unknown
and then tell the user:
"N sessions will continue running."
Unknown explicitly means Corral cannot make that process-liveness claim.
Keep two authoritative counts:
R =
origin == managed
AND execution_state == running
U =
origin == managed
AND execution_state == unknown
Exited:
not counted
Discovered/history/unknown origin:
not counted
because Desktop quitting does not terminate Corral-owned runtime management
for those rows.
These are derived solely from daemon-provided facts:
origin
execution_state
No client heuristics.
Quit gate
Warn whenever:
R > 0
OR
U > 0
Case R > 0, U == 0
Recommended:
"R sessions will continue running. Corral will stop watching them for
attention."
[Quit] [Cancel]
After OS notifications exist, the final copy may use the already-approved
notification-oriented wording.
Case U > 0
The copy must preserve uncertainty.
For example:
"R sessions are still running. Corral couldn't verify whether U other
sessions it started have ended. Corral will stop watching for attention."
If R == 0, omit the first sentence rather than saying "0 sessions".
Do not tell the user unknown sessions:
will continue running
or:
have stopped.
Current generation
Use the most recent successful daemon `session.list` generation on the
currently healthy connection.
Given the existing 1-second poll,
a separate new RPC surface is unnecessary.
A Quit action should not use:
an old cached generation from before the connection became unavailable
as though it were current.
Daemon unreachable
If the daemon/client connection is currently unavailable:
do not calculate:
N = 0
from stale/missing data.
Use uncertainty copy:
"Corral can't reach its service, so it can't tell whether sessions it
started are still running. Corral will stop watching for attention."
[Quit] [Cancel]
User may still Quit.
Corral must never hold the UI hostage merely because daemon reachability is
lost.
One gate for all Quit sources
Still applies:

* ⌘Q
* app menu Quit
* tray Quit

all enter the same decision path.
Core invariant:
Corral may warn more conservatively when runtime truth is Unknown,
but conservative warning must not become a false Running claim.

#### Q12 — macOS system notification product policy

接受 items 1–9 with the following precision.
1. Notification-producing states
Only a newly observed current AttentionItem of:
Needs You
Ready
may produce an OS notification.
Never notify merely for:
Working
Unknown
Exited
state expiration
Item resolution/removal is silent.
2. New-item detection
For one continuously healthy notification surface:
previous successful projection did not contain AttentionItemId X
+
new successful projection contains X
→ X is newly observed by this surface
Eligible for one notification according to its reason/state.
Cold start baseline
After Desktop/notification surface starts:
first successful poll establishes baseline.
Existing items:

* appear in tray/menu/badge
* do NOT generate startup notification burst

Reconnect baseline
After the surface loses its daemon connection and later reconnects:
the first successful post-reconnect generation is also a silent baseline.
This intentionally chooses:
avoid duplicate/reconstructed notification storms
over:
guarantee notification delivery for items born while this surface was
disconnected.
Therefore accept the limitation explicitly:
An attention item that appears and remains active entirely across a
notification-surface disconnect may become visible in badge/menu after
reconnect without ever producing an OS notification.
This is honest M1 behavior.
Do not recover it by guessing cross-reconnect item identity.
A later durable notification-delivery model could change this.
3. Replacement
OS notification identity is session-scoped.
A new current AttentionItem for the same session:
→ replaces the prior notification
→ produces its own delivery behavior
Example:
Needs You item A
→ later resolves
→ Ready item B
B is a new item and may notify.
Replacement does NOT mean A and B share AttentionItem identity.
4. Withdrawal
Current item disappears/resolves/expires
with no replacement:
→ silently withdraw corresponding OS notification where mechanism permits
Current item becomes acknowledged:
→ silently withdraw
Acknowledgement does NOT change the underlying Needs You/Ready semantic
state.
5. Notification click
Notification click uses the same product action as tray row click:
activate/reopen Desktop
→ select CorralSessionId
→ normal Open
Ready:
successful Open
→ acknowledges exact Ready item
Needs You:
Open/view
→ does NOT acknowledge
6. Foreground behavior
M1 does not suppress notifications merely because:

* Desktop is frontmost
* the session is selected
* a window exists

No "probably already looking at it" heuristic.
Dogfood may later justify suppression policy.
7. Sound
Needs You:
system/default attention sound
Ready:
silent banner
This is product policy,
subject to actual macOS notification API/system Focus behavior.
8. Content
Title:
approved session display title
Body:
approved `corral-client::presentation` state/reason wording
+
sanitized display-safe NeedsInputContext where available
Never include raw:

* transcript text
* prompt body
* tool arguments
* hook payload
* terminal capture

merely because the underlying evidence contains it.
Notification presentation must consume the same privacy-safe presentation
projection used elsewhere.
9. No M1 notification preference system
No:

* per-session mute
* Corral Do Not Disturb
* notification settings page
* custom scheduling

macOS Focus/system notification controls remain the system-level mechanism.
Evidence consequence
The first baseline/reconnect suppression rules mean OS-notification
evidence must distinguish:
eligible new item observed continuously
from:
item discovered only at surface baseline.
Only the former tests notification delivery fidelity.
Core principle:
Notifications represent newly observed attention transitions,
not a replay of current daemon state whenever the UI reconnects.

#### Q13 — acknowledged rows in the tray menu

Reject A.
Do NOT put a native ✓ on an acknowledged Needs You row.
Reason:
In a native menu, a checkmark conventionally reads as:
selected
enabled
or completed
For Corral:
acknowledged Needs You
can still mean:
the agent is blocked and still needs the user.
A ✓ risks visually saying:
done
when only the alert has been acknowledged.
Instead use D:
mark the UNACKNOWLEDGED item, not the acknowledged one.
Preferred semantic presentation:
unacknowledged row
→ subtle unread/attention marker, e.g. leading `•` or equivalent native-safe
affordance
acknowledged row
→ normal row, no marker
Exact glyph/rendering may be decided in presentation implementation,
but the semantic direction is frozen:
marker means "this attention item remains unacknowledged"
never:
marker means "this blocker is resolved".
This directly explains:
Needs You total = 3
badge = 1
because one row still carries the unread/unacknowledged marker.
Do not introduce suffix prose such as:
"Acknowledged"
unless usability evidence later shows the marker insufficient.
If the chosen menu toolkit cannot provide a clean distinction without
misleading semantics:
prefer no per-row acknowledgment marker
over a misleading checkmark.
Core invariant:
Acknowledgement clears alert attention.
It does not visually imply resolution of the underlying state.

#### Q14 — tray initialization failure

接受。
On macOS, if tray initialization fails:

* tray/watchfulness disabled for this process
* last-window-close falls back to quit
* no automatic retry
* log diagnostic details
* main-window persistent banner appears

Approved concept copy:
"Menu bar icon unavailable — closing this window quits Corral."
Keep visible for the process lifetime.
Do not silently retry and later switch lifecycle semantics underneath the
user.
If the user restarts Corral:
→ a fresh initialization attempt is fine
This is a new application run,
not hidden runtime recovery.
Initialization ordering
Accept the assumption:
attempt tray initialization before establishing the normal window lifecycle
policy
so the initial main window already knows whether:
last-window close = watchful background
or:
last-window close = process exit
If tray setup fails before main window construction:
carry the failure state into the window/banner model.
Linux
Do not show this failure banner on Linux merely because Linux has no tray.
Linux tray absence is:
known unsupported platform capability
not:
macOS tray initialization failure.
PRODUCT/support copy owns that distinction.
Core invariant:
A tray failure changes lifecycle visibly and deterministically for the
entire application run.

你的 Q11 里"unknown 也加进 N"这个保守方向是对的，错的是把两者压成同一句 `N sessions will continue running`。应该保守地触发警告，但不能保守地制造 Running 事实。
Q12 我也赞成整体政策，尤其"首次 baseline 不响"。但要把代价写死：daemon/transport 断线期间诞生的 item，重连后可能永远没有 OS 通知，只会出现在 badge/menu。 这是 precision-first 的有意取舍，不要以后有人看到 missed notification 就偷偷用 session/reason 指纹补跨重连推断。
这一轮后，剩下我预计只有：probe 实测结果、无窗资源是否可接受、`tray-icon/muda` 最终机制封印，以及计划从 conditional → accepted。 系统通知政策已经可以视为冻结，不需要 packaging 阶段重新 grill。

## What this grill leaves open

Evidence only. The probe's measured result against the six cases; whether
the windowless process's resource use is operationally reasonable; the
sealing of `tray-icon` + `muda` as the accepted mechanism, or a stop with
evidence if it fails to compose with gpui; the plan's move from
*conditionally accepted pending Design 0 tray probe* to accepted after one
reconciliation. Notification product policy is frozen here and is not
re-grilled by the packaging or notification tasks; those own mechanism.
