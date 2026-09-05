# PR9 Plan Grill — structural rulings over the Desktop plan (rounds 1–2)

> Status: **structural frontier cleared; plan conditionally accepted**
> (2026-09-05). Round 1: the founder read
> `docs/plans/2026-09-05-pr9-desktop.md` in full and ruled its six
> decisions, adding a hard constraint to two of them and two plan-level
> corrections. Round 2 ruled the seven questions those rulings unblocked;
> one recommendation was overturned (Q8: no raw command in New Session,
> and the launch grammar's authority stays in corrald). The plan stays *conditionally
> accepted, pending foundation reconciliation*: once `task/adr-0017-accept`
> and `task/adr-0017-impl` are on `main`, one reconciliation pass — frame
> and capability names, the replica's prefix assumptions against the final
> ADR 0017, the bridge's assumptions against the merged S6/Q6 runtime
> contract, the dependency gate against `gpui = "=0.2.2"` — flips it to
> accepted if it finds only faithful materialization, with no further
> grill.

Digest of what the round froze:

| # | Decision | Ruling |
|---|---|---|
| 1 | Session-row decoding and presentation lifted into `corral-client` | **Accepted.** Shared product-projection vocabulary for every client; `corral-client::presentation` may interpret daemon facts into the accepted presentation vocabulary and may not own view layout, widgets, or independent semantic inference. The Desktop never depends on `corral-tui`. |
| 2 | No gpui-component in PR9 | **Accepted.** PR9 has not earned that dependency; not a permanent refusal. |
| 3 | 1 Hz polling, no push channel | **Accepted**, with a consistency rule: `session_list` and `attention_summary` are two RPCs and are published together per poll generation only when both succeed; the daemon's summary is authoritative and no client recomputes counts from visible rows; no overlapping polls; immediate refresh after local state-changing actions where useful. |
| 4 | One automatic resync per failure episode | **Accepted with a re-arm constraint.** An episode begins at the first failure of a usable attachment, has a budget of exactly one `ResyncRequest`, and is cleared only by a complete fresh replica installed successfully; re-armed only by an explicit re-open or a genuinely new daemon-produced epoch or checkpoint — never by the change the client's own resync caused. Every replica-unusable cause (parser panic, malformed prefix, impossible combination) shares this one framework. |
| 5 | One main window with embedded terminal, standalone on demand | **Accepted with one-Desktop-attachment ownership.** Within one Desktop process a session has at most one Desktop-owned terminal attachment and replica; standalone is another presentation host of the same `SessionTerminalEntity`, never a second `terminal.attach`; intra-Desktop multi-viewer is a later, explicit entry into the existing contract. |
| 6 | Linux compile and non-rendering tests only; macOS validated | **Accepted.** A green Linux CI never removes *unvalidated*; a Linux Desktop claim requires real display, render and input validation first. |

Two plan-level corrections, applied: *Interrupt* is the accepted terminal
input representation of Ctrl-C (ETX) as an `Input` frame, not a function
of replica modes; a disconnected surface shows last-known facts as
historical context only — a cached row is not a current semantic claim and
caching never extends its freshness.

## Founder rulings, verbatim

```text
我把计划原文看完了。整体形状是对的：它确实没有偷偷把 tray / notifications / packaging 拉进 PR9，而且 D1–D4 基本守住了"Desktop 只是 client surface，不另造 runtime truth"的边界。

但六点里我会对 **#4、#5 各加一个硬约束**。所以现在可以裁结构，但**计划本身先记 `conditionally accepted / pending foundation reconciliation`，不要现在改成最终 accepted**。等 ADR 0017 两个 PR 都进 main，只需要做一次 reconciliation，不再完整 grill。

### 1. 把 `presentation` / session-row decoding 抬到 `corral-client`

**接受。**

这不是"把 UI 塞进 client library"，而是把**所有客户端必须一致解释的 product projection vocabulary**放到共享客户端层：

* typed `SessionListItem`
* `MainState`
* `SessionPresentation`
* secondary runtime/capability facts
* short-id/display helpers

现在 TUI 自己拥有这些，而 Desktop 又需要逐字一致；继续复制才更危险。计划里 Desktop 不依赖 `corral-tui`，这一点必须保持。

边界锁成：

> `corral-client::presentation` may interpret daemon facts into accepted product presentation vocabulary; it may not own view layout, GPUI/TUI widgets, or independent semantic inference.

也就是说：

daemon truth
   ↓
corral-client shared projection vocabulary
   ↓
TUI / Desktop rendering

而不是：

TUI logic
   ↓ copy
Desktop logic

D1 接受。

### 2. PR9 不引入 `gpui-component`

**接受。**

当前 PR9 真正需要的是：

* list
* detail view
* terminal element
* disclosure dialog
* basic controls

没有证据需要额外引入一套 component framework + 237 crates。计划用裸 GPUI `div`/Element 足够，而且依赖治理刚为了 GPUI 本身付过一次成本。

但这不是：

> Corral Desktop 永不使用 gpui-component。

只是：

> PR9 has not earned that dependency yet.

以后 Desktop 真长出 forms / menus / complex reusable widgets，再拿真实需求重开。

D2 接受。

### 3. session list 继续 1 Hz polling，不加 push

**接受。**

PR9 不应为了 Desktop 再开一个 server-push / list-subscription wire surface。

但补一个一致性要求：

`session_list` 和 `attention_summary` 是两个 RPC，所以客户端**不得假装它们来自同一个原子 snapshot**。

建议 Desktop refresh cycle：

poll generation G begins
→ fetch session_list
→ fetch attention_summary
→ if both succeed, publish UI refresh G together
→ if either fails, don't combine one fresh half with one arbitrarily stale half

这不能制造真正的 server-side atomicity，但至少避免 UI 自己把不同 refresh generation 混起来。

而且继续沿用之前 Q23：

> daemon summary is authoritative; Desktop does not recompute global counts from visible rows.

所以：

* no push
* 1 Hz initial policy
* immediate refresh after local state-changing/open-return actions where useful
* no overlapping poll storms

接受。

### 4. replica poison：每 failure episode 自动 resync 一次

**接受，但必须把 episode 边界写死。**

计划现在写：

> 第一次 poison → 自动 resync；新 replica 再失败 → 停止，直到 new epoch 或用户重新 Open。

方向对。

我会定义：

failure episode begins:
first failure while a previously usable/current attachment becomes unusable

automatic recovery budget:
exactly one ResyncRequest

episode is successfully cleared only by:
a complete fresh replica being installed successfully

第一次：

replica failure
→ destroy replica
→ Terminal unavailable
→ automatic ResyncRequest

如果恢复 snapshot 仍失败：

→ destroy failed replacement
→ NO second automatic ResyncRequest

之后可以重新尝试的外部触发只允许：

1. 用户显式重新 Open / reattach；
2. daemon **自行产生的 genuinely new authoritative epoch/checkpoint** 到来。

特别要禁止：

> client 的 resync 自己导致某个 epoch/checkpoint 变化，然后把这个变化解释成"new epoch"，重新获得一次 resync budget。

否则还是能形成：

poison
→ resync
→ poison
→ new epoch
→ resync
→ ...

所以核心不变量：

> **A recovery attempt cannot manufacture the event that re-arms its own recovery budget.**

另外，parser panic 不是唯一该走这个 bounded recovery policy 的情况。凡是会导致 replica 不可信的：

* malformed semantic prefix
* impossible geometry/palette/snapshot combination
* qwertty panic

都应该共享同一个"replica unusable → bounded recovery"框架，而不是各自造 retry loop。

#4 接受，带这个修订。

### 5. 单主窗口 + embedded terminal，standalone 按需

**接受，但这里计划漏了一个很重要的 Desktop 内部 ownership 规则。**

PR3 已经冻结：

> 同一个 terminal 可以多 viewer；geometry 是共享 runtime state；last explicit resize wins。

所以如果 PR9 Desktop 做成：

main embedded viewer
+
standalone viewer

同时 attach 同一个 session

你马上就会得到 Desktop 自己跟自己抢 resize：

embedded 120×35
standalone 180×50
→ authoritative resize
→ 两边 layout/reflow
→ resize feedback / flicker

而且凭空把一个 presentation feature 变成了第二个 terminal subscriber。

PR9 不需要这样。

我会冻结：

> **Within one Desktop process, one Session has at most one Desktop-owned active terminal attachment/replica by default.**

Standalone 应该是**同一个 Desktop terminal entity 的另一个 presentation host**，不是第二次 `terminal.attach`。

概念：

SessionTerminalEntity
├── TerminalChannel
├── Replica
├── geometry owner
└── presentation host:
      embedded
      OR standalone

当用户 "Open in standalone window"：

* 可以把现有 terminal entity 从 embedded host 移到 standalone；
* main window 留 placeholder / "Open in window"；
* 关闭 standalone 后可以重新嵌回；
* 不需要第二个 daemon terminal subscriber。

如果 GPUI 技术上 reparent 很麻烦，也可以让 standalone 成为 primary rendered host、embedded 停止 render，但还是共享一个 attachment/replica owner。

**不要为了 PR9 的窗口便利无意中实现 intra-Desktop multi-viewer semantics。**

以后真的需要：

> 同一 Desktop 同时两个 terminal view 看同一个 session

再明确进入已经存在的 multi-viewer contract。

所以 #5 接受，但 D3 要加这一条。

### 6. Linux 只 compile + non-rendering tests；macOS 为 validated Desktop

**接受。**

这和之前 Q8/Q12 一致。

PR9：

* macOS：实际 Desktop validation
* Linux：compile + platform-independent/non-rendering tests
* support matrix：Linux Desktop rendering = **unvalidated**

计划里 Ubuntu CI 安装这些开发库只是 build environment setup，也合理。

但 release/support gate继续保持：

> If M1 later claims Linux Desktop support, real Linux display/render/input validation becomes mandatory before that claim.

不能因为"CI 编译绿"把 support matrix 后面那个 `unvalidated` 自动删掉。

## 还有两个 plan-level 修改，我建议现在顺手补

第一个是 `actions.rs` 里的 **Interrupt**。计划现在写：

> interrupt = replica's own `^C` bytes.

这个描述容易误导。

`Ctrl-C` 是否应编码成什么字节，通常不是 DECCKM/bracketed-paste 这种 output-terminal mode 决定的。PR9 不要把 interrupt 过度抽象成"replica mode encoding"。

更准确：

Interrupt
→ send the already-accepted terminal input representation for Ctrl-C / ETX
→ no new RPC

普通 keyboard input 可以根据 replica mode 编码 cursor keys / bracketed paste 等。

第二个是 failure state：

> daemon unreachable 时显示 last facts with their age

可以，但必须明确：

cached row != current semantic claim

尤其不能离线以后还保留鲜亮的：

`Needs You`

让用户以为这是当前事实。

应该沿用你们已经冻结的 freshness/Unknown 法则：

> disconnected surface may show last-known facts as secondary historical context, but cannot extend their semantic freshness merely because Desktop cached them.

## 当前计划状态

我会记录为：

PR9 structural decisions:
1. shared client presentation vocabulary — ACCEPTED
2. no gpui-component — ACCEPTED
3. 1 Hz polling, no push — ACCEPTED
4. one automatic resync per failure episode — ACCEPTED WITH RE-ARM CONSTRAINT
5. one main window + standalone on demand — ACCEPTED WITH ONE-DESKTOP-ATTACHMENT OWNERSHIP
6. Linux compile/test only — ACCEPTED

PR9 plan:
CONDITIONALLY ACCEPTED
pending foundation reconciliation

等这两个进 main：

1. `adr-0017-accept`
2. `adr-0017-impl`

然后从 main 看一遍：

* actual frame/capability names 是否和 plan 一致；
* replica prefix assumptions 是否完全匹配最终 ADR 0017；
* S6/Q6 已合 runtime contract 有没有被 plan 的 bridge/replica 假设违反；
* dependency gate 最终状态与 `gpui = "=0.2.2"` 是否一致。

**如果只是忠实物化，没有新决策，直接把 PR9 plan 从 conditional → accepted，不再 grill。**
```

# Round 2

Digest of what round 2 froze:

| Q | Ruling |
|---|---|
| Q7 | Desktop activates corrald through the same `corral-client::activate` contract as the CLI and TUI; no Desktop-side discovery, endpoint choice, startup policy, or lifetime ownership; closing a window or quitting never stops corrald. Desktop is another client, not a daemon lifecycle authority. |
| Q8 | **Recommendation overturned.** New Session = Claude Code or Codex + explicit working directory + optional provider arguments under the accepted grammar. No raw command, custom executable, or shell command: `corral new -- <cmd>` was PR3's walking skeleton, never a product promise; a Custom Agent needs its own decisions first. The provider grammar's authority stays in corrald — clients may share an input model, preflight helpers, and typed error rendering, and may reject early; only corrald decides whether a managed launch is valid. |
| Q9 | Keyboard path: printable text, Enter, Backspace, Tab, Escape, arrows under DECCKM, Ctrl+letter, paste under bracketed paste, Interrupt. No terminal mouse reporting in any encoding; the Desktop's own mouse UI is unaffected. No claim of every keyboard protocol. |
| Q10 | `Resize` only from a local change of the rendered cell grid, coalesced but prompt at rest; observed daemon geometry (Geometry frames, snapshot size, epochs, other viewers) never echoes back; a host transition counts as local. |
| Q11 | No terminal selection, highlighting, copy, rectangular or link selection in PR9; paste stays; stated as a limitation. |
| Q12 | (B) No dev-dependency on `corrald`; cross-process tests launch a validated test-support corrald binary under an isolated test root, inheriting the e2e isolation contract in full (validated binaries, explicit root, fail before spawning without the test-build marker, never canonical endpoint or `~/.corral`, correct under a concurrent `cargo build` elsewhere). Unit tests own the replica; process-boundary tests cover only the public client contract. |
| Q13 | Legacy fallback under an old daemon: the attachment's last explicitly requested geometry, never labelled daemon-confirmed, no UI fact; a real local cell grid is established and sent before the first legacy snapshot, never a guessed 80×24. With `terminal.geometry.v1` advertised the fallback is forbidden: a snapshot without its Geometry is a desync. |

## Founder rulings, verbatim (round 2)

```text
这轮我会改 **Q8**，其余基本照收。最重要的是：**PR3 的 generic `corral new -- <cmd>` 当时明确只是 walking-skeleton，不能在第一个正式 Desktop 里悄悄变成"Corral 是通用终端启动器"的产品承诺。** 另外 Q12 要继承之前已经踩过的 e2e 隔离教训。

# PR9 plan grill — round 2 rulings

## Q7 — Desktop daemon activation

接受。

Desktop 与 CLI/TUI 使用同一 activation contract：

Desktop startup
→ `corral-client::activate`
→ connect to canonical corrald
→ if absent and activation is permitted, start the canonical daemon according to ADR 0001

Desktop 不建立第二套：

* daemon discovery
* endpoint choice
* startup policy
* lifetime ownership

Closing a Desktop window:

→ disconnects that client/window
→ does NOT stop corrald

Quitting the Desktop application:

→ disconnects Desktop-owned clients
→ does NOT explicitly stop corrald

daemon 是否退出仍由自己的：

* established clients
* managed runtime ownership
* idle lifecycle

决定。

PR9 没有 tray，
所以不要在 Desktop quit 上偷偷引入未来 tray/watchfulness 的生命周期语义。

核心不变量：

Desktop is another corrald client, not a daemon lifecycle authority.

## Q8 — New Session input surface

修改推荐。

接受：

* provider selection
* working directory
* provider-specific additional arguments

但 PR9 Desktop **不提供 first-class raw command mode**。

因此 PR9 New Session：

Provider:

* Claude Code
* Codex

Working directory:

* explicit directory

Advanced arguments:

* optional provider arguments
* subject to the accepted managed-launch grammar/policy

不提供：

Custom command
Raw executable
Arbitrary shell command

理由：

之前的：

`corral new -- <cmd>`

是 PR3 walking-skeleton/runtime harness。

当时已经明确：

它不定义最终 M1 "New Session" product UX。

如果现在 Desktop 把 raw command 放进 provider picker，
就会把一个测试/底层能力升级成新的产品承诺：

Corral manages arbitrary interactive terminal programs.

这超出了：

Every coding agent. One place.

以及当前 Claude/Codex supported-provider scope。

未来若真实需求证明需要：

Custom Agent / Generic Command

再单独定义它的：

* identity semantics
* attention semantics
* title/provider presentation
* continuation capability
* support boundary

### Validation ownership

另一个重要修正：

不要把 ADR 0012 的 authoritative provider grammar 从 daemon 搬进
`corral-client`。

Daemon remains the authority for:

managed launch argument acceptance.

Desktop/CLI 可以共享：

* argument input model
* UX-side preflight helpers
* typed rendering of daemon validation errors

但客户端验证不能成为唯一安全边界。

如果目前 CLI 有一份纯客户端重复 validator：

可以提取可共享的 UX/preflight部分，

但：

`session.new`
→ daemon MUST revalidate using its canonical provider grammar

不能形成：

Desktop validated it, therefore daemon trusts it.

所以 D1 可以增加：

shared launch-form / validation-error presentation

但不要写成：

"move the provider allowlist authority into corral-client."

核心不变量：

Clients may reject bad launch input early.
Only corrald decides whether a managed provider launch is valid.

## Q9 — keyboard input scope

接受 proposed PR9 set。

Supported keyboard/input behavior:

* printable text
* Enter
* Backspace
* Tab
* Escape
* arrow keys
* Ctrl+letter combinations
* paste
* interrupt / Ctrl-C behavior already accepted by the terminal-input path

Arrow encoding follows the replica's current terminal mode where required,
including DECCKM.

Paste follows bracketed-paste mode where active.

PR9 does NOT implement terminal mouse reporting.

因此暂不支持：

* mouse button reports
* motion reports
* wheel-as-terminal-mouse protocol
* SGR/X10/etc mouse encoding

普通 Desktop UI mouse interaction仍然当然可以用于：

* selecting a session row
* clicking actions
* focusing terminal
* resizing windows

这里只是不把 mouse events 转译给 PTY application。

同样不要因为键盘 scope 看起来简单就声明：

all terminal keyboard protocols supported.

未列入的特殊键可以后续按真实需求增加。

核心原则：

PR9 implements the keyboard path needed for ordinary agent TUIs;
terminal mouse protocol is a separate capability.

## Q10 — Resize policy

接受。

Desktop sends Resize only as the result of a local terminal-view geometry
change.

Geometry source:

actual rendered terminal cell grid

not raw pixel dimensions.

Flow:

view pixels/layout change
→ recompute rows/cols
→ if cell geometry changed
→ send Resize

Do not emit Resize merely because:

* daemon reports Geometry
* a Snapshot carries a different geometry
* epoch changes
* another viewer resized the authoritative terminal

Those are observations of daemon truth,
not local resize intent.

This preserves the already-accepted anti-feedback rule:

server geometry update
MUST NOT automatically cause resize echo.

Embedded ↔ standalone host transition:

counts as a local presentation geometry change

and may therefore produce one Resize to the new grid dimensions.

Coalesce normal window-resize churn sufficiently that Desktop does not send
a Resize for every pixel event,
but the final local cell geometry must be delivered promptly.

Same rows/cols:

→ no Resize

核心不变量：

Only local desired geometry produces a Desktop resize command;
observed authoritative geometry never echoes itself back.

## Q11 — terminal selection / copy

接受：

no terminal text selection/copy in PR9.

PR9 terminal element owns:

* rendering
* keyboard input
* paste
* cursor/display fidelity
* control loop

它暂不拥有：

* mouse text selection
* selection highlighting
* terminal-to-clipboard copy
* rectangular selection
* semantic link selection

Paste remains supported because it is an input capability,
not evidence that clipboard output is also implemented.

这应该作为明确的 PR9 limitation，
不要让用户以为 selection 坏了。

M1 completion 或后续 Desktop surface 可以增加 selection/copy，
不需要 terminal wire change。

核心原则：

PR9 proves graphical See → Know → Control;
it does not need to become a complete terminal emulator UX in the same PR.

## Q12 — Desktop cross-process test topology

选择 B。

`corral-desktop` MUST NOT dev-depend on `corrald` merely to make tests easy.

Pure replica/state-machine behavior：

→ unit tests inside corral-desktop

Cross-process behavior：

→ launch a real test-support corrald binary under an isolated test root

This preserves production dependency direction:

Desktop/client code consumes public client/protocol contracts,
not daemon implementation internals.

### Mandatory isolation inheritance

The earlier e2e binary-contamination incident applies here in full.

Desktop e2e harness MUST:

1. use an immutable/validated test-support `corrald` binary;

2. validate any `corral` helper binary too, if used;

3. use an explicit isolated test root;

4. fail before spawning if the expected test-build marker/isolation
   contract is missing;

5. never fall back to:

   * canonical endpoint
   * `~/.corral`
   * user's registry/log/state

6. remain correct even if another shell concurrently runs:
   `cargo build -p corral`
   or
   `cargo build -p corrald`

A wrong binary:

→ test failure

never:

→ production daemon activation

### Test split

Replica unit tests:

* prefix state machine
* epoch handling
* poison/resync budget
* input encoding

Cross-process tests:

* actual hello/capabilities
* terminal attach
* Geometry/Palette/Snapshot prefix
* resync request/response
* channel termination/reconnect
* daemon compatibility behavior

Do not duplicate the entire corrald fidelity test suite from the client side.

Only cross the process boundary where the public client contract itself is
under test.

核心原则：

Tests may run the real daemon;
client crates must not link against daemon internals.

## Q13 — old daemon without `terminal.geometry.v1`

接受 fallback，
不增加用户可见 secondary fact。

This is a compatibility mechanism,
not a product status worth occupying the PR9 UI.

Rules:

### Capability absent

If daemon hello does NOT advertise:

terminal.geometry.v1

Desktop may use:

the last geometry that this Desktop terminal attachment explicitly
requested

as the legacy replica geometry assumption.

It must NOT internally label that value:

daemon-confirmed authoritative geometry.

### First snapshot problem

There must be a defined requested geometry before the first legacy
Snapshot is installed.

Therefore on an old-daemon attachment:

Desktop waits until the terminal host has a real local cell geometry,
then establishes/sends its initial Resize,
then uses that requested size as the legacy assumption.

Do not fall back to an arbitrary:

80×24

merely because layout was not ready yet.

If no valid local geometry can yet be established:

→ delay installing the terminal replica
→ do not guess

### Capability present

If daemon advertises:

terminal.geometry.v1

then legacy fallback is forbidden.

Snapshot without required Geometry:

→ protocol desync
→ do not install
→ bounded resync policy

Do not say:

"Geometry missing, but we happen to know what size we requested,
so carry on."

That would defeat ADR 0017's capability contract.

### Multi-viewer consequence

Under an old daemon,
another viewer can resize after Desktop's request.

Therefore the fallback is intentionally only:

legacy best-effort compatibility

not an authoritative geometry guarantee.

No extra UI label is required because:

* PR9 and daemon normally ship together;
* compatibility mode is transient;
* exposing "geometry unconfirmed" is too implementation-specific for the
  session product surface.

核心不变量：

Legacy fallback may use the client's own explicit geometry assumption;
a geometry-capable daemon must satisfy the authoritative Geometry contract.

所以这轮里我唯一真正反对的是 **Q8 的"原始命令"**。

之前已经专门裁过一次：

> `corral new -- <cmd>` 是 managed-runtime walking skeleton，不定义最终 New Session 产品。

PR9 是第一个真正会被人"当产品用"的图形界面。如果现在下拉框里出现：

> Claude / Codex / Raw command

那个决策以后很难再说"只是测试 harness"。

我建议 PR9 就非常明确：

> **New Session = Claude or Codex + working directory + advanced provider args.**

另外，provider arg allowlist 也别随着 UI helper 一起从 daemon 权威层搬走。**共享客户端预检可以，最终裁决必须继续在 corrald。**

这轮落完以后，我认为真正的结构性 frontier 已经清空。接下来等 ADR 0017 两个 PR 进 `main`，然后只做一次 plan reconciliation；只要最终 wire/runtime contract 与这两轮裁决一致，就可以把 PR9 plan 正式 `accepted`，不用再 grill。
```

## What this leaves open

The reconciliation pass, after the two ADR 0017 PRs merge. Nothing
structural.
