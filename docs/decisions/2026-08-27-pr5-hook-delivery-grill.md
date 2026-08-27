# Founder Decision Record — PR5 hook delivery and Claude managed sessions

> Status: founder-accepted, 2026-08-27. Class C. Materializes into
> `docs/adr/0004-hook-delivery.md` and is implemented under
> `docs/plans/2026-08-27-pr5-claude-managed-sessions.md`.

Two grill rounds over the drafted ADR 0004 and PR5 plan. Round one ruled
ten questions (Q1–Q10); its Q3 ordered a durable-vocabulary capability
check, which found the accepted event vocabulary cannot express a contested
binding; round two ruled that Class C durable gap (R2 Q1–Q3). The founder's
rulings are carried verbatim below.

## Capability check evidence (2026-08-27, this tree)

- `corral-state/src/event.rs::SessionEvent` holds exactly nine kinds:
  `session-created`, `binding-added`, `binding-confirmed`, `run-started`,
  `run-ended`, `run-attached`, `run-detached`, `session-forked-from`,
  `command-accepted`. None weakens, ends, or contests a binding.
- `BindingConfirmed` strengthens only. Carrying contrary evidence in it
  would alter an accepted event's meaning — the path AGENTS §Durable state
  forbids.
- `ControlEligibility` is `Eligible | AssuranceTooWeak`, derived from
  assurance alone (`corral-core/src/binding.rs`); no durable input can push
  an Attested binding out of eligibility.
- `STORAGE_EPOCH` = `dev`: adding an event kind carries no migration
  obligation. The decision itself still requires explicit human acceptance,
  which this record is.

## Round one rulings (verbatim)

```text
第一轮裁决


Q1 — 50 ms hook relay budget

采纳单一 50 ms overall deadline。

但精确定义：

50 ms 是每一次 hook relay invocation 的
maximum synchronous interference budget。

它覆盖：

connect
→ write/frame delivery
→ minimum acknowledgement if the protocol requires one

使用单一 monotonic deadline，
每个阶段消费剩余预算。

禁止：

10 ms connect
+ 50 ms send
+ 50 ms ack

这种阶段重置预算。


同样禁止 relay-side retry-loop。

明确错误例如：

ENOENT
ECONNREFUSED
EACCES

应立即 fail open，
不为了“花满 50 ms”继续等待。


timeout / temporary backlog：

→ event may be dropped
→ hook returns control to provider
→ never widen timeout to mask daemon slowness


250 ms 不应写成某个“五事件事务”的系统保证。

正确说法是：

Each individual hook invocation may delay its caller by at most 50 ms.

如果 provider 在一个操作过程中同步触发 5 次 hook，
理论累计干扰可达到约 250 ms；
这是 provider 调用模式的组合结果，不是 relay 自己获得 250 ms budget。


核心不变量：

Hook delivery is best-effort within a hard interference budget.
Daemon slowness degrades Corral awareness, never provider progress.


Q2 — local hook trust floor

采纳 (a)，但必须把 threat-model 文字写准。

冻结：

M1 Local Mode hook-ingress authenticity floor
= same host OS user boundary.

0600 Unix endpoint protects against other OS users.

It does NOT authenticate one process belonging to the same OS user
against another process belonging to that same user.


launch token：

是：

- correlation evidence
- accidental cross-session confusion protection
- proof that a hook event matches a Corral-created launch under the
  non-malicious-same-user threat model

不是：

- cryptographic authorization against a malicious same-user process
- privilege boundary


所以不要在 ADR 写：

“0600 + token proves an authorized actor.”

写：

“A malicious process already executing as the same OS user is outside
the M1 local evidence-authenticity boundary.”


但是 daemon 仍必须验证：

- token 是它签发的/认识的；
- token 对应正确 launch/run；
- provider/session facts 满足绑定规则。

不能因为 threat floor 是 OS user
就接受任意同用户发来的 session_id。


这和 AGENTS：

“same OS user is insufficient for privileged control”

不冲突。

Hook evidence ingestion 本身不是 privileged control authorization。

后续 control eligibility 仍然走 Corral binding/assurance/control rules。


如果以后 M3 remote/local adversarial model 要抵抗同用户 process，
那是新的 trust architecture，不在 PR5 做假安全。


Q3 — session_id mutation / contested identity

选择 (ii)：

identity anomaly
→ provider identity becomes contested
→ session.resume fails closed
→ Open/current live attachment remains available where safe


但我要加一条硬要求：

CONTESTED 不能只是 process-memory warning。

原因：

今天发生：

session A
→ hook reports provider id X
→ same launch/token later reports Y
→ mark contested

daemon restart

如果 contested 状态消失，
下次 session.resume 又拿 X 去 resume，

我们只是把错误推迟到了重启之后。


所以冻结的是：

“control eligibility revocation caused by an identity conflict must
survive long enough to prevent later silent resume.”


实现前先 capability-check PR2 durable binding model：

如果已有 accepted durable vocabulary 能表达：

- binding no longer safe for control
- identity conflict / invalidated binding
- without merging X and Y

→ 使用既有模型
→ 无新 durable decision。


如果不能：

STOP
→ this is a Class C durable semantic gap
→ founder decision before implementation crosses it。


不要为了守住“PR5 zero schema diff”把 contested 只存在内存。


另外：

后续重复收到 Y
不能自动把 contested 清掉。

PR5 不发明 automatic conflict resolution。

当前原则：

ambiguity discovered
→ revoke resume eligibility
→ future accepted identity/correction mechanism resolves it


Q4 — awaiting_input 是否上屏

选择改版 (a′)。

允许显示，但它只能是：

provider-reported historical/secondary fact

绝不能成为：

- main Needs You state
- attention item
- tray badge
- notification
- current authoritative wait claim


推荐语义：

“Claude reported waiting for input · 5m ago”

必须：

- 明确 provider/reporting provenance；
- 过去式；
- 始终带 age。


但不要“只要曾经出现过就永远显示”。

应该显示：

latest still-relevant provider-reported semantic fact。


例如：

10:00 awaiting_input
10:01 turn_started / stop / session_end

则 10:05 不再把：

“Reported waiting for input · 5m ago”

挂在主列表 secondary line 上。

因为已有更新事实 supersede 它。


PR5 不设置 freshness threshold。

所以：

latest awaiting_input
+
没有后续 superseding evidence

即使几小时后：

仍可以显示 age，
为 dogfood 提供数据。


但“没有固定 stale threshold”
≠
“忽略后来明确相反的事实”。


这正好允许 PR8 dogfood 对比：

provider reported waiting
vs
future authoritative attention derivation

而不会让 PR5 自己假装已经实现 Needs You。


Q5 — Provider trait

采纳推荐：

PR5 不建立 Provider trait。

建立具体：

provider::claude

并守住明确模块边界，例如：

- launch construction
- resume construction
- hook ingress interpretation
- provider-specific validation


PR6 第二个真实 provider 出现以后，
再根据两个实现提炼真正共同的 abstraction。

原则：

Module boundary now.
Trait only after the second implementation provides evidence.


不要为了让 PR6 “验证 trait”
而先让 PR5 猜一个 trait。


Q6 — CLI provider shape

采纳 provider-first positional form。

M1/PR5：

corral new claude [-- <provider args>]

raw runtime harness：

corral new -- <cmd> [args...]


因此：

corral new bash

如果 bash 不是已知 provider：

→ explicit unknown-provider error
→ “For a raw command, use: corral new -- bash”


不做：

unknown first argument
→ 猜它是不是 executable

这样 provider namespace 和 raw command namespace 不会模糊。


PRODUCT 的正常路径就是 provider-first，
所以 normal CLI 不需要：

--provider claude


注意：

这是 public CLI surface change，
所以按治理属于 compatibility-facing/human-reviewed surface。

当前未 external ship，可以修改；
但不能声称它只是内部 refactor。


Q7 — Unverifiable run 的 continue

采纳硬拒绝。

条件：

previous Run execution is confirmed Exited
→ native resume may be eligible

previous Run is Unverifiable / execution Unknown
→ session.resume refused


M1：

- no override
- no --force
- no “I know it is dead”
- no PID heuristic bypass


理由：

我们没有足够证据证明旧 provider execution 已停止。

启动第二个 native resume
可能造成两个 live executions 操作同一 provider session。


错误应陈述事实，例如：

“Corral cannot verify that the previous run has exited, so it will not
resume this provider session automatically.”

可以给 provider session/session id 做诊断，
但不要把它包装成 Corral 内的越权按钮。


后续如果 provider matrix 证明 concurrent resume 有安全行为，
再拿证据重新裁。

PR5 不预支那个答案。


Q8 — provider event wire

这里我不接受原来的二选一。

你现在把两条不同 wire 混在一起了：

1. provider hook → corrald ingress
2. corrald → Corral clients semantic IPC


正确是三层。


Layer 1 — provider ingress

Claude hook relay sends provider-specific facts.

例如概念上：

ProviderIngress {
    provider = Claude
    hook_event = Notification / Stop / SessionStart / ...
    provider-specific structured payload
}

relay/shim 不负责把：

Claude Notification
→ Corral AwaitingInput

这种 semantic interpretation 做掉。


Layer 2 — corrald

provider::claude owns interpretation:

raw/provider-specific event
        ↓
normalized Evidence / semantic event

例如：

SessionStarted
TurnStarted
AwaitingInputReported
...


这里才是 provider knowledge 的 owner。


Layer 3 — client-facing Corral IPC

Corral clients only see normalized Corral semantics where needed。

Desktop/TUI/CLI 不认识：

Claude “Stop”
Codex event-name-X


因此：

provider-specific ingress
→ daemon-owned normalization
→ provider-neutral client semantics


这同时满足两条既有原则：

- provider knowledge belongs in provider adapter / daemon;
- clients render Corral's semantic model.


所以如果当前 `agent_event.kind` 是：

hook relay → corrald

的 wire：

不要把它伪装成 provider-neutral enum。


如果它是：

corrald → client

的 wire：

则 normalized enum 正确。


方法名：

采纳：

session.resume

wire/domain operation 对齐 NativeResume。

UI/CLI product wording：

Continue in Corral

继续保持两层 vocabulary separation。


Q9 — provider/version matrix evidence

采纳：

PR5 先做 dated first-party evidence record，
不在这个 PR 强行搭完整 automated release matrix。


记录必须至少包含：

- exact Claude Code version
- installation/update channel if relevant
- OS
- exact scenario
- command/config
- expected behavior
- observed behavior
- Corral commit SHA
- date
- pass/fail/limitations


位置：

docs/references/

是合理的，因为这是：

external provider behavior / compatibility research evidence

不是 Corral 自己算法的 fuzz/soak verification artifact。


PR5 不把：

provider investigation
+
verify-release matrix runner infrastructure

硬塞成一个 semantic scope。


但计划必须留明确 follow-up：

supported provider/version matrix automation
→ verify-release owning task

在 M1 release 之前必须落地，
不能把“一次文档证据”当永久 release gate。


Q10 — injected settings cleanup

拒绝：

“corrald startup 时清扫所有无主文件”

这种宽泛规则。


Run 正常、可确认结束：

→ best-effort delete Corral-owned injected settings file

采纳。


文件：

- Corral-owned state directory
- unique per launch/run
- 0600
- provenance/owner naming sufficient to avoid deleting arbitrary files


但 daemon restart 后必须区分：

A. owner Run confirmed Exited
→ safe to GC according to provider-file lifetime contract

B. owner Run ended Unverifiable / process fate Unknown
→ DO NOT blindly delete merely because current daemon does not own it


因为这正是 Q7/Q5 一直守的原则：

loss of Corral ownership
≠
proof provider process is dead。


即使我们“觉得 Claude 启动时只读取一次 settings”，
除非 PR5 provider matrix 明确验证并把它变成 supported assumption，
不要靠猜测删除一个可能仍被 surviving provider process referenced 的文件。


所以 PR5 startup cleanup：

可以自动删除：

- malformed files clearly owned by Corral and never successfully published;
- files whose owning Run is durably confirmed Exited;
- temporary creation remnants for which no launch was committed.

对于 Unverifiable owner：

→ retain
→ optionally log stale Corral-owned provider artifact
→ no destructive cleanup in PR5


这确实可能留下少量 0600 文件。

但这是比：

“为了目录干净而破坏一个可能还活着的 provider process”

更正确的 tradeoff。


Token exposure 也不能作为删除理由强行覆盖 lifecycle truth。

如果 launch token 在 daemon restart 后已经无效，
旧文件里的 token 只是 stale secret-like material；
0600 + eventual confirmed cleanup 足够。

如果 token 在 restart 后仍然有 authority，
那反而说明 token lifecycle 本身必须先设计清楚，
更不能盲删 settings。


Q10 核心不变量：

Cleanup requires ownership evidence strong enough for the artifact being
destroyed.

An Unverifiable Run does not provide that evidence.
```

## Round two rulings (verbatim)

```text
Q3 durable gap — Founder Decision

Class C boundary confirmed.

The accepted durable vocabulary cannot represent a binding that was once
Attested but is now known to be identity-conflicted.

Add one narrow durable fact.
Do not generalize assurance reevaluation in PR5.


Q1 — Durable representation

裁决：接受 `binding-contested`。

新增：

BindingContested {
    session,
    binding,
    conflicting_external_id,
    evidence,
}

语义：

A previously accepted provider binding has received contradictory
provider-identity evidence through the managed launch channel.

`conflicting_external_id` means only:

“this conflicting identifier was reported”

It does NOT:

- create a binding to that identifier;
- merge sessions;
- replace the existing external id;
- assert that the conflicting id is the correct one.


`binding-contested` is monotonic for PR5.

Once emitted:

- the binding remains contested;
- later reports of the original id do not automatically restore it;
- later reports of the conflicting id do not automatically replace it;
- later third ids do not create more bindings;
- PR5 has no automatic conflict-resolution mechanism.

Subsequent reports may be retained as diagnostics/evidence if useful,
but they do not emit repeated state-transition events once the binding is
already contested.

Clearing contested requires a future explicitly accepted correction /
re-identification mechanism.


Do NOT implement:

- generic assurance-reassessment event;
- assurance downgrade to Heuristic;
- projection-only contested state without a durable event.

Reason:

This is not weaker evidence about the same claim.
It is positive evidence that two incompatible identity claims have been
observed.


Q1 accepted durable semantics:

BindingConfirmed
    ↓
later contradictory provider identity evidence
    ↓
BindingContested

There is no PR5 transition back.


Q2 — What does contested revoke?

裁决：接受“只撤销依赖 provider identity 的动作”，
但拒绝把 `IdentityContested` 直接塞进一个泛化的
`ControlEligibility` unless that type is already explicitly scoped to
provider-identity control.

The durable projection should expose a distinct binding fact:

binding.identity_status =
    Confirmed
    | Contested

(or an equivalent explicit representation)

Assurance remains orthogonal.

Therefore:

Attested + Confirmed
is different from
Attested + Contested.

Do NOT mutate:

Attested → Heuristic.


For PR5, derive an operation-specific eligibility:

NativeResumeEligibility =
    Eligible
    | AssuranceTooWeak
    | IdentityContested


`session.resume` requires:

- sufficient assurance;
- identity_status == Confirmed;
- all other existing resume preconditions.

If contested:

→ IdentityContested
→ fail closed
→ no provider external id is placed into resume argv.


Runtime operations that do NOT depend on provider conversation identity
remain unaffected, including where otherwise valid:

- Open / terminal attach
- observing the current managed runtime
- runtime-local interruption/control owned by the deterministic runtime
  binding

So do NOT let a generic `ControlEligibility::IdentityContested`
accidentally mean:

“nothing about this session can be controlled anymore.”


Core invariant:

Identity contest revokes authority derived from that identity claim;
it does not revoke unrelated authority derived from a deterministic
runtime binding.


If the existing `ControlEligibility` type is already narrowly documented
as “provider-identity-based control eligibility”, it may be renamed/
extended consistently.

If it is generic session control eligibility, keep it generic and add the
operation-specific eligibility above.

Do not introduce a generalized action-policy framework in PR5.


Q3 — What remains visible after contest?

裁决：基本接受推荐。

After `binding-contested`:

provider name:
→ retained

Reason:
Claude/provider kind came from the managed launch and is not what became
ambiguous.


current provider external_id claim:
→ withdrawn from normal client-facing current-identity representation

If `SessionListItem.provider.external_id` means:

“the provider identity Corral currently stands behind for this session”

then:

→ omit / None after contest.

Absence means unknown/not currently assertable.
It does not mean no provider id ever existed.


Do not silently replace X with Y.


Agent/turn facts:

→ continue to ingest and normalize facts belonging to the current managed
runtime.

Reason:

The runtime itself is deterministically Corral-owned;
the disputed fact is which provider conversation identity that runtime
currently represents.


But normalized facts must not regain identity authority indirectly.

For example:

a later SessionStart(Y)
after contest

may remain diagnostic/runtime evidence,
but cannot automatically:

- publish Y as current external_id;
- clear contested;
- enable NativeResume.


Preserve historical evidence separately.

The durable binding/event history may still show:

- original confirmed id X;
- conflicting report Y;
- evidence/timestamps.

That is historical provenance.

The normal client field is a current claim.

Do not overload the same field to mean both.


Optional UI secondary presentation may say something neutral such as:

“Provider identity conflict”

but presentation wording is not part of this durable decision.


Q3 core invariant:

Withdraw exactly the claim that became unsafe.

Do not erase the provider/runtime facts that remain known,
and do not promote the conflicting claim into a replacement identity.
```

## Founder emphasis on R2 Q2, recorded

A generic `ControlEligibility` gaining an `IdentityContested` arm invites
`if eligibility != Eligible { disable_all_control() }`, which would disable
Open/attach — operations the Deterministic runtime binding still honestly
supports. Revocation must track the source of the authority it revokes:
the durable projection keeps `Confirmed | Contested` as its own fact, and
`NativeResumeEligibility` consumes it. `ControlEligibility` in
`corral-core` today is generic binding-control eligibility, so it stays
generic.
