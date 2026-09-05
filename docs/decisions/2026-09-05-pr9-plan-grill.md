# PR9 Plan Grill — structural rulings over the Desktop plan (round 1)

> Status: **structural round closed; plan conditionally accepted**
> (2026-09-05). The founder read `docs/plans/2026-09-05-pr9-desktop.md`
> in full and ruled its six decisions, adding a hard constraint to two of
> them and two plan-level corrections. The plan stays *conditionally
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

## What this leaves open

The reconciliation pass, after the two ADR 0017 PRs merge. Nothing
structural.
