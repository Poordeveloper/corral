# PR7 Integration Grill — structural rulings and acceptance (rounds 1–4)

> Status: **closed** (2026-09-02). Rounds 1–2 froze the structural
> rulings over the proposed ADR 0013 (global hook integration) and
> ADR 0014 (external session evidence); the spike
> (`docs/references/2026-09-02-pr7-global-integration-spike.md`) measured
> the load-bearing facts; round 3 ruled the fact-sensitive remainder;
> the final round ruled acceptance: **both ADRs `proposed → accepted`**,
> with the three remaining evidence items placed on named gates (Q7′).
> A spike or matrix result that contradicts a load-bearing accepted
> assumption explicitly reopens the ruling with the reason recorded —
> never a silent edit; ordinary matrix expansion reopens nothing.
> Governing principle, ruled in Q1:
>
> Decision authority may precede empirical completion;
> ADR acceptance may not precede its load-bearing evidence.

Questions asked by the grill session of 2026-09-01; rulings verbatim below.

# Round 1

Digest of what round 1 froze:

| Q | Ruling |
|---|---|
| Q1 | (b) — structural rulings bind now; ADRs stay proposed; post-spike round rules fact-sensitive items only; no full re-grill |
| Q2 | ADR 0013 D1 amended: during PR7 dogfood, install runs only on explicit `corral integration enable`; the normal installer owns default install (ADR 0006); first daemon activation is never the installation trigger. Post-enable drift auto-repair NOT ruled — round 2 |
| Q3 | (a) — occupied Codex `notify`: preserve, degrade, explain, human resolution only; no takeover/force/backup-and-replace/chaining in PR7; takeover UX deferred pending dogfood evidence. Invariant: Corral never overwrites a non-Corral Codex notifier merely to obtain awareness |
| Q4 | (a′) — Class C durable expansion accepted in concept: a Run may end because the runtime continues but ceases to carry that Session; neither `Exited` nor `Unverifiable` may be reused for it; A ends, B starts, never concurrently open on sameness of process alone. Discriminant name and event/transaction shape: round 2 (founder leans a fact-shaped name, e.g. `SessionChanged`; `Departed` not frozen) |
| Q5 | (a′) — three evidence tiers: weak candidate evidence is internal-only; an approved high-precision provider-runtime recognition is immediately visible as a provisional row; identity evidence binds. Invariant: discovery can collect weak evidence freely; user-visible rows require enough evidence to support the row's literal claim. If no sufficiently precise process-only recognizer exists, the evidence threshold wins over PRODUCT §9 |
| Q6 | (a′) — installer/uninstaller must run integration uninstall before removing Corral-owned artifacts, AND a default-installed integration must fail open when Corral is absent: stale integration must not repeatedly disrupt the user's provider sessions. Missing-command behavior is a load-bearing spike fact; visible per-event disruption is a STOP on the default-install shape, and no residual-failure mechanism (wrapper/shim/other) is preselected before measurement |

## Founder rulings, verbatim

```text
第一轮裁决。


Q1 — 裁决时序

选择 (b)。

本轮产生 binding structural rulings，
但 ADR 0013 仍保持：

status: proposed

直到承重 spike 完成。


本轮可以冻结：

- integration mutation 的唯一写者/owner
- intent model
- read-only / mutation boundary
- claim ladder
- succession 的 durable ownership semantics
- provisional visibility policy
- uninstall/fail-open responsibility


后续 spike 只负责裁：

- provider-specific trigger inventory
- exact recognition grammar
- missing-hook behavior
- repair/reconciliation mechanics whose safety depends on measured facts


不要 spike 后重新完整 grill。

否则已经裁过的结构性问题会因为“重新 grill”
无意义地重新开放。


但是如果 spike 直接反证某个 structural ruling 的前提：

→ explicitly reopen that ruling
→ record why

不能静默改。


本轮 rulings：

docs/decisions/2026-09-01-pr7-integration-grill.md

它记录：

accepted founder rulings for the proposed ADR

而不是伪装成：

ADR 0013 accepted。


核心原则：

Decision authority may precede empirical completion;
ADR acceptance may not precede its load-bearing evidence.


Q2 — 默认安装触发点

修改 D1。

PR7 / dogfood 阶段：

installation is triggered only by an explicit user action:

corral integration enable

daemon activation MUST NOT turn an untouched developer machine into
a configuration-writing event merely because intent happens to resolve
to Enabled/default-enabled.


正常产品安装阶段：

the installer/package installation flow owns default integration install

以兑现 ADR 0006：

integration is installed with the normal installation.


因此区分：

product default:
Enabled

和：

PR7 development trigger:
explicit enable


这两个不矛盾。


PR7 daemon activation：

- may inspect integration state
- may detect/report drift
- does not perform first-time silent installation


已经明确 Enabled 后发生 drift 是否自动 repair：

本轮不裁。

那属于你下一轮的 repair cadence / ownership mechanics。


所以不要把本裁决扩大成：

“daemon 永远不得修 integration”。

现在只冻结：

First activation is not the installation trigger during PR7 dogfood.


这也避免：

corral list

这种逻辑上的 read operation
第一次运行却突然改 ~/.claude/settings.json。


Q3 — Codex notify 已被用户占用

选择 (a)。

PR7 的 resolution path 写实为：

- detect that Codex notify is already owned by another configuration
- preserve it
- report Limited awareness
- explain exactly what blocks Corral integration
- tell the user how to remove/change that conflicting notify
- after the user has done so, allow them to run integration enable again


不要写模糊的：

“resolution path offered”

如果实际上没有 Corral-owned resolution operation。


PR7 不提供：

--force
take over
backup-and-replace
chain notifier


原因不是这些永远错误。

而是当前没有 dogfood evidence 证明：

“已经使用 custom notify 的 Codex 用户”
是值得我们新增 takeover lifecycle 的真实 M1问题。


尤其 (b) 一旦做了就要定义：

- backup durable location
- user changes notify after backup 的 conflict semantics
- uninstall restore 的 compare-and-swap 条件
- backup stale/corrupt
- multiple Corral versions
- user intentionally replacing Corral while enabled

这绝不是一个小 flag。


因此 deferred：

explicit takeover UX

只有真实冲突数据证明需要时再开。


核心不变量：

Corral never overwrites a non-Corral Codex notifier merely to obtain
awareness.


Q4 — succession 时 Run 怎么结束

接受 (a) 的语义方向，
但先不冻结名字 `Departed`。

这是一个真实 Class C durable semantic expansion。


已接受事实：

Run =
one concrete runtime occurrence of one Session


因此外部进程：

process P carrying Session A
        ↓
provider succession
        ↓
same process P now carrying Session B

意味着：

A's Run occurrence has ended

即使：

process P has NOT exited.


现有两个 RunEnd 都不诚实：

Exited
→ false: process did not exit

Unverifiable
→ false: Corral actually has affirmative evidence explaining why
  A stopped being the session carried by that runtime


所以不能复用任意一个。


冻结新的 durable semantic concept：

A Run may end because the underlying runtime continues but ceases to
carry that Session identity.


succession transition 必须逻辑上表现为：

Run(A) open
        ↓ strong succession evidence
Run(A) ends with NEW_REASON
Run(B) starts on the same continuing runtime occurrence/process context


不允许：

A and B remain concurrently open merely because OS process is the same.


否则 durable log 会说：

一个已经不被 runtime 承载的 Session A
仍然有 live Run

这是错误 projection。


也不允许：

B 不落 durable Run

因为那会使 observed runtime truth 在 succession 后凭空退出 durable model。


所以：

Q4 creates a Class C durable decision boundary.

本轮 founder 已接受“需要第三种 end semantic”这个决定。

下一轮只需裁：

- exact discriminant name
- exact event ordering / transaction shape
- whether existing projection columns suffice or require schema migration


我目前更偏向一个事实型名字，例如：

SessionChanged

而不是 `Departed`。

但名字下一轮再封，
不要现在因为示例词生成永久 wire/storage vocabulary。


核心不变量：

A Run ends when that runtime stops carrying its Session,
even when the underlying OS process continues.


Q5 — provisional 行显示门槛

修改成 (a′)。

PR7 可以立即显示 sweep-discovered provisional rows，
但只有当：

the provider-specific recognizer has reached the explicitly approved
high-precision recognition threshold.

不能：

看到进程名里有 "claude"
→ 就造一行


要把三层分开。


1. Weak candidate evidence

例如：

- loose process-name match
- ambiguous wrapper
- IDE child where role cannot be established

→ internal discovery evidence only
→ NOT user-visible session row


2. Approved provisional runtime recognition

经过 spike 封印的：

- argv shape
- executable identity
- process relationship
- mode exclusions
- other provider-specific facts

足以高精度声称：

“there is a supported provider runtime here”

但 provider session identity 尚未知

→ immediately visible provisional row


呈现可类似：

Running outside Corral · Status unknown


但它仍然：

- no semantic Working/Needs You/Ready
- no provider identity claim beyond evidence
- no heuristic merge
- no durable binding fabricated merely for display


3. Strong provider event / identity evidence

→ bind/collapse according to accepted identity rules


所以 PRODUCT §9 的正确解释是：

Supported pre-existing live sessions should become visible as soon as
Corral has sufficient evidence to make that claim.

而不是：

Every discovery heuristic hit must be visible.


这仍然保持 precision-first：

false row 比 delayed row 更伤 Corral 的可信度。


同时接受你的兜底：

strict sweep may miss
+
global hook first event may independently surface/bind it


这两个路径互补。


dogfood noise catalog 重点记录：

- false provisional rows
- supported live sessions missed until hook
- wrapper/IDE/MCP false candidates


如果最终 spike 证明根本没有足够高精度的 process-only recognizer，

那 Q5 不强迫你显示：

evidence threshold wins over roadmap wishful thinking.


Q6 — binary/package 被删后的残留 integration

选择 (a′)。

第一条义务：

normal installer/uninstaller MUST run integration uninstall
before removing the Corral-owned executable/integration artifacts.


但这还不够。

因为现实里总会有：

- manual binary deletion
- broken package removal
- downgrade
- filesystem corruption
- partially removed install


默认安装的 integration 不能把：

“Corral binary happens to exist forever”

当成 provider usability 的前提。


所以 ADR 0013 还应冻结更强的一条：

A default-installed Corral integration must fail open when Corral is
absent or unavailable; stale integration must not repeatedly disrupt the
user's provider session.


这正是 missing-command spike 要验证的承重事实。


新增 spike：

Claude:
configured Corral hook command path does not exist
→ observe exact provider behavior

Codex:
configured Corral notifier/entry command path does not exist
→ observe exact provider behavior


记录：

- user-visible warning/error
- per-event/per-turn repetition
- whether agent continues
- latency/blocking
- exit status interpretation
- stderr/stdout behavior
- whether provider disables/retries the integration


如果结果是：

missing command is silent/fail-open

→ current default-install shape remains viable。


如果结果是：

every turn/event shows visible error
or meaningfully disrupts progress

→ STOP

不能仅靠：

“our package manager normally runs uninstall”

来接受 default-installed integration。


那时必须重新设计 residual-hook failure shape，
例如 provider grammar 若允许的话使用一个自身 fail-open 的 invocation
mechanism。

但本轮不预选 shell wrapper / stable shim / 其他机制；
先量事实。


所以 Q6 不只是 packaging hygiene。

它是 default-install safety invariant。


核心不变量：

Removing or losing Corral must not turn a previously installed integration
into persistent interference with the user's agent.
```

Additional founder emphasis, same session: Q4 must not be dodged by abusing
`Unverifiable` to preserve a "PR7 zero durable diff" — PR3's `Unverifiable`
was honest because the process outcome was genuinely unknown; succession is
the opposite case. Q5's precision-first is a **display gate**, not a
recognizer implementation wish: "Discovery can collect weak evidence
freely; user-visible rows require enough evidence to support the row's
literal claim."

# Round 2 (same session, 2026-09-01)

Digest of what round 2 froze:

| Q | Ruling |
|---|---|
| Q7 | `RunEnd::SessionChanged`, encoding `"session-changed"`; no successor reference in A's end event — B's side of the transition is expressed by B's own events in the same transaction; doc comment locks "runtime continued, ceased to carry this Session, never claims the OS process exited" |
| Q8 | One accepted succession observation commits as one atomic store operation (one SQLite transaction, projections in-tx, all or nothing); canonical order A `RunEnded(SessionChanged)` → B `SessionCreated` (if new) → B binding events → B `RunStarted`; invariant A-end seq < B-start seq; atomicity claimed for succession only; idempotency rides existing machinery |
| Q9 | (a′) — product goal accepted (old client keeps "Run has ended", may lose the reason) but the implementation claim corrected: adding `Unknown(raw)` today cannot retro-teach already-shipped decoders. Three layers: durable truth precise (`SessionChanged`); protocol adapter projects a lossy-but-true representation for old peers; new decoders are born open for the future. Invariant: a newer daemon must never make an older compatible client lose the fact that a Run ended merely because the daemon knows a newer end reason. A wire capability check (read the PR6 encoding, not a spike) decides where the projection lands |
| Q10 | (a′) — Enabled authorizes maintaining what Corral owns, only at daemon startup and named operations; missing Corral-owned entry ≠ modified/conflicting slot: the former may auto-repair, the latter is a conflict that is never overwritten (Limited awareness + explicit resolution); repairs observable and counted; frozen principle: repeated undoing by another authority must eventually stop automatic repair rather than create a tug-of-war; the recurrence threshold/window/fingerprints deliberately unfrozen until spike + dogfood data |

Founder rulings, verbatim:

```text
第二轮裁决


Q7 — succession RunEnd discriminant

选择：

RunEnd::SessionChanged

storage encoding:
"session-changed"


语义：

The Run ended because the continuing runtime stopped carrying this
Session identity and began carrying another Session identity.

它明确不意味着：

- OS process exited
- runtime disappeared
- execution became unverifiable


不带 successor SessionId / RunId。


理由：

RunEnd(A) 应只陈述：

为什么 A 的 Run 不再成立。

B 的出现已经由同一事务中的：

- SessionCreated（若需要）
- BindingAdded / BindingConfirmed
- RunStarted

表达。


不要在 A 的结束事件中再嵌：

successor_session_id
successor_run_id

否则会产生：

- cross-session durable coupling
- successor creation ordering dependency
- replay 时双向关系一致性义务
- future succession/fork semantics 被当前模型绑死


如果 projection/UI 以后需要：

“这个 runtime 从 A 去了 B”

应该从原子 transition 中投影出来，
而不是把 durable A event 变成 navigation link。


doc comment 可以直接锁：

`SessionChanged` means that the runtime continued, but ceased to carry
this Session. It never claims that the underlying OS process exited.


这次 durable expansion 是本 grill 已接受的 Class C decision。


Q8 — succession transaction

选择 (a)。

一个被强证据确认的 succession：

runtime carrying A
→ same runtime carrying B

作为一个 atomic store operation 落盘。


若 B 是新 session，canonical event order：

1. A: RunEnded(SessionChanged)
2. B: SessionCreated
3. B: BindingAdded / BindingConfirmed as applicable
4. B: RunStarted


若 B 已存在：

1. A: RunEnded(SessionChanged)
2. B: BindingAdded / BindingConfirmed as applicable
3. B: RunStarted


关键不变量：

A-end seq < B-start seq.


整个 transition：

- one SQLite transaction
- projections updated in same transaction
- commit all or commit nothing


不能出现 durable 可见中间态：

A 已结束
但已知 succession 的 B 尚未开始

仅仅因为实现把同一事实拆成两个 commits。


这不是要求所有 provider events 都跨 session 原子化。

只针对：

one accepted succession observation
which simultaneously proves both sides of the transition.


retry/idempotency 继续沿用既有 command/event idempotency machinery，
不要为 succession 再发明第二套 transaction identity。


Q9 — old-client compatibility

接受 (a) 的产品目标：

旧客户端必须至少保留：

Run has ended

可以损失：

exact new end reason.


但是修改实现结论。


不能说：

“本 PR 给 RunEnd 加 `Unknown(raw)`，
所以 PR6 旧客户端就能安全收到 `SessionChanged`。”

这是时间倒流。

已经存在的旧 binary decoder
不会因为新版本今天增加了 Unknown(raw)
突然学会容忍未来 discriminant。


因此要先区分两件事。


A. backward delivery compatibility

新 daemon 向旧 negotiated protocol peer 发送事件时：

必须投影成那个 peer 已经能解码的表示。

也就是说：

durable:
RunEnded(SessionChanged)

不要求：

old wire:
RunEnded(SessionChanged)


如果旧协议已有一个不会撒谎的 lossy 表示，
daemon 应针对旧 peer 做 compatibility projection。


当前最可能可用的已有语义是：

RunEnd::Unverifiable

因为它至少仍表达：

- managed Run 已结束
- process exit was not established

对于 SessionChanged 来说，
这两个陈述仍然都是真的，
只是旧客户端不知道真正原因是 succession。


所以如果 PR6 wire 已能表示：

RunEnded(Unverifiable)

则旧协议 projection 可以是：

SessionChanged durable fact
        ↓ old peer
RunEnded(Unverifiable)


这是一种有意的 information loss，
不是 durable reinterpretation。


如果旧 wire 有更准确的：

ended / unknown-reason

则优先用那个。

但不能现在凭空假定它存在。


B. future-input compatibility

从本版本开始，
可以把 RunEnd wire decoder 改造成：

Known(...)
Unknown(raw)

或者其他明确的 open-enum encoding，

使“今天的新客户端”将来遇到新 reason 时：

- 保留 RunEnded
- 降级 reason
- 不让整个事件 decode fail


这个 future-input fixture 值得做。

但它只保护：

new client against future server values

不保护：

already-shipped old client against today's new SessionChanged.


因此 Q9 的真正 invariant：

A newer daemon must never make an older compatible client lose the
fact that a Run ended merely because the daemon knows a newer end reason.


如果当前 negotiated protocol 根本没有任何合法方法
把 SessionChanged 降级成旧客户端可解码且不撒谎的 RunEnded：

→ protocol compatibility boundary
→ bump minimum compatible version / capability as required

不能发送未知 enum 然后期待 serde 自动善后。


所以在实现前做一个很小的 capability check：

“PR6 wire 对 RunEnd 的实际编码和 decoder 是什么？”

这不是重新 grill；
只是确定 compatibility projection 具体落在哪。


Q10 — Enabled 后 integration drift repair

方向接受 (a)，但改成 (a′)。

持续 intent：

integration_intent = Enabled

确实授予 Corral 维护其 integration 的权利。

因此：

首次安装不得由 daemon activation 偷偷触发

和：

已经明确 enable 后允许维护 Corral-owned integration

完全不冲突。


但“发现被改 → 自动修”必须再切一刀。


自动 repair 只能作用于：

bytes / fields / entries that Corral can prove it owns.


例如：

1. Corral-owned entry missing
   → may auto-repair

2. Corral-owned entry contains an old Corral executable path/version-owned
   representation that ownership rules prove is ours
   → may auto-repair

3. unrelated user-owned configuration changed
   → never touch

4. expected Corral slot now contains non-Corral/user-authored content
   → conflict
   → do NOT overwrite
   → Limited awareness + explicit resolution


因此：

Enabled
≠
permission to continuously normalize the user's provider configuration.


repair 时机采纳：

- daemon startup
- explicit named integration operation

不做：

- periodic polling writer
- mid-run rewrite
- per-hook rewrite
- background normalization loop


每次 successful repair：

→ observable/logged
→ delivery-health accounting


对 repeated drift：

我不建议现在冻结具体 N。

这是你已经正确留给 spike/dogfood 的参数。

但结构原则现在冻结：

Repeated evidence that another authority keeps undoing Corral's
integration must eventually stop automatic repair rather than create a
configuration tug-of-war.


N / recurrence window 后续按 spike + dogfood 数据定。


直到该参数封印前，
不要把 implementation 写成：

while Enabled:
    if drift:
        rewrite forever


另一个重要区别：

missing Corral-owned entry

和：

modified/conflicting entry

不应共享同一个 repair policy。


前者可以是正常 repair。

后者只有在 ownership model 能证明变动部分仍完全属于 Corral 时才能修；
否则必须当 conflict。


核心不变量：

Enabled authorizes Corral to maintain what Corral owns.
It does not authorize Corral to overwrite configuration whose ownership
has become ambiguous.
```

## Q9 capability check — result (2026-09-01, working tree at PR6)

Measured, not assumed: the client protocol carries **no `RunEnd`
representation at all**. `crates/corral-protocol/src/method.rs`'s
`SessionListItem.execution_state` is an open string — `running` /
`exited` / `unknown` — and its documented decoder rule already treats an
unrecognized value as `unknown`. No durable-event stream has a wire
surface yet (PR1's no-ghost-wire rule). Landing, per the ruling's own
priority ("prefer the more accurate existing *ended* representation"): a
succession-ended Session projects `execution_state: "exited"` for every
peer — projecting a new string would downgrade old clients to `unknown`
and violate the Q9 invariant; no minimum-version bump is required; any
richer succession fact for newer clients is a later additive optional
field, absent meaning unknown. Recorded in ADR 0014 D7.

## What the post-spike round rules

Provider-specific trigger inventory (D4 seal), exact recognition grammar
(Q5's approved threshold), missing-hook behavior (Q6's STOP check),
repair recurrence parameters (Q10's N / window / drift fingerprints), and
repair/reconciliation mechanics whose safety depends on measured facts.
ADRs 0013/0014 move to `accepted` only there. The structural frontier is
otherwise empty.

# Round 3 — post-spike (2026-09-02)

Evidence: `docs/references/2026-09-02-pr7-global-integration-spike.md`.
Questions Q1′–Q6′ put to the founder over that record's findings 1–11;
rulings verbatim below.

Digest of what round 3 froze:

| Q | Ruling |
|---|---|
| Q1′ | (a′) — Claude default-installed hook entry MUST use a fail-open guarded invocation (conceptually `<corral relay invocation> \|\| true`); D8 freezes the semantic shape "provider-visible hook result is fail-open", not a quoting string. Hard boundary: the shell guard is not a new provider-data parser — only static Corral-owned invocation structure plus safely represented Corral-owned path/arguments; never interpolate payload, prompt text, event content, arbitrary-text identifiers, or user shell fragments into shell syntax. The integration grammar recognizes exactly the Corral-owned guarded form it writes; arbitrary `\|\| true` proves nothing. Tradeoff accepted: relay crashes are hidden from Claude's UI and belong to Corral delivery-health/Limited-awareness reporting. Exit-status-only judgment is a version-matrix retest fact. Codex keeps its native argv invocation |
| Q2′ | Trigger inventory sealed for the measured versions (common + Claude `disableAllHooks` at any measured effective layer including layers Corral never mutates but must inspect + Codex occupied/ill-typed `notify`), plus a mandatory pre-replacement validation gate: the complete candidate content is re-parsed by **Corral's provider-specific strict validation parser** (never described as the provider's own parser) before atomic replacement; a failed re-parse is a D4 refusal leaving original bytes untouched. The gate is necessary but not sufficient; the supported-version matrix remains the empirical authority |
| Q3′ | Provider-specific representation policy accepted; D3's "preserved original" split into **semantic preservation (universal)** and **byte/format preservation (provider-specific, required where the measured provider format/workflow makes it meaningful)**. Claude `settings.json`: strict JSON, whole-document parse, structured merge, semantic preservation of unknown keys, canonical re-serialization, no byte-layout/key-order preservation, and Corral MUST NOT introduce comments. Codex `config.toml`: format-preserving TOML editing — comments, unrelated key order, spacing retained; only the owned `notify` surface mutated |
| Q4′ | Circuit breaker accepted: 3 automatic repairs per rolling 24h window per fingerprint `(provider, config target, drift class)`; drift classes: entry missing / older Corral-owned representation / ownership conflict — but ownership conflict never consumes the repair budget (it is already non-auto-repairable and goes straight to Limited awareness). **The breaker does not close when the window slides**: once open it stays open until an explicit user-controlled reconciliation action (`corral integration repair` or an equivalent explicit re-enable) rechecks ownership and, on success, clears the breaker and history for that fingerprint. Daemon restart never clears it. Breaker state and repair history are Corral-owned durable operational state — a durable-state expansion under the storage law, with its own schema/migration review; this ruling covers the policy, not the schema. 3/24h is a dogfood-tunable policy default, never a wire constant; no implementation may silently exceed currently accepted repair authority. Invariant: *Enabled authorizes bounded self-repair, not an endless configuration tug-of-war* |
| Q5′ | Split sealing accepted: seal only matrix-established primitives (resolved executable identity is evidence, argv[0] never sufficient; provider one runtime hop below launcher/wrapper per measured shape; truncated comm never primary; Claude's measured lower-chain hook shape; Codex notify's parent relationship used only for the exact claim it supports; descendant-of-provider insufficient — providers spawn unrelated children such as git). Unsealed: tmux/screen/nohup/terminal-host chains, unmeasured macOS host shapes, Homebrew shapes. Unsealed upper-chain facts may be collected diagnostically but MUST NOT contribute the evidence required for a user-visible provisional session claim; an independent sealed path (e.g. integration delivery) may still suffice. The recognizer may know more candidates than the UI may claim; matrix expansion is additive and does not reopen the recognizer model |
| Q6′ | Proposed "second occurrence of the same thread-id promotes to user-visible Session" **rejected as a frozen rule** — repetition raises confidence in persistence but does not change the semantic type of what is observed. Split two concepts: a **runtime/provisional row** is justified by approved runtime-recognition evidence alone ("a supported provider runtime appears to be running here", status Unknown, no thread id required); **provider identity candidates** — unknown Codex notify thread-ids — are recorded as live/internal candidate binding evidence against the observed runtime and never mint additional rows. The measured real-turn + title-generation sequence yields one runtime row plus candidates A and B, never two Sessions flashing. Promotion from candidate to Session binding requires evidence that the identity represents the user-facing provider session (future matrix-proven provider behavior or another strong identity primitive); no prompt-content sniffing, no lexical heuristics, no frozen second-occurrence rule without a matrix experiment proving a stable provider semantic contract. If PR7 has no strong discriminator for external Codex notify identities, the honest M1 result is runtime visible + identity unresolved + identity-requiring features unavailable — not ghost Sessions. Managed Codex identity paths are not weakened. Invariant: *Provider-emitted identity evidence may create identity candidates; a user-visible Session requires evidence that supports the literal claim that this identity is the user's session* |

## Founder rulings, verbatim (round 3)

```text
Post-spike rulings


Q1 — Claude residual integration must fail open

选择 (a′)。

Claude default-installed hook entry MUST use a fail-open guarded invocation.

Measured fact:

missing naked hook command
→ repeated user-visible provider errors

therefore naked command form fails ADR 0013 D8.


Accepted shape:

Claude integration invocation
→ execute Corral relay
→ regardless of relay exit/failure, return provider-success for the hook
   boundary

Conceptually:

<corral relay invocation> || true


Codex keeps its native argv invocation because measured missing-command
behavior is already silent/fail-open and no equivalent shell guard is
needed.


However, add a hard boundary:

The shell guard is not a new provider-data parser.

The guarded shell command may contain only Corral-owned static invocation
structure plus safely represented Corral-owned path/arguments.

It MUST NOT interpolate:

- hook payload
- prompt text
- provider event content
- session identifiers originating as arbitrary text
- user shell fragments

into shell syntax.

Provider event data must continue through the already-defined data channel
(stdin / fixed argv contract / other measured mechanism), not by string
concatenation into `sh -c`.


Therefore D8 freezes the semantic shape:

provider-visible hook result is fail-open

not a particular fragile quoting string.


The checked-in Claude integration grammar must recognize exactly the
Corral-owned guarded form it writes.

It must not treat arbitrary:

"... || true"

as proof of Corral ownership.


Tradeoff explicitly accepted:

A relay crash may be hidden from Claude's own UI.

That is intentional.

Integration delivery failure belongs to Corral delivery-health /
Limited-awareness reporting, not repeated interference in the user's
agent session.


Version-matrix requirement:

The measured fact that Claude judges this hook boundary by the guarded
command's resulting exit status is load-bearing and must be retested on
supported-version changes.


Q2 — D4 trigger inventory + write validation

接受 proposed provider/version-bound trigger inventory.

For the measured versions, seal the listed conditions into the matrix.


Common refusal/reconciliation triggers include:

- provider config cannot be parsed
- structure at the merge path has an incompatible type
- Corral-owned entry claims a representation/version newer than this
  Corral understands
- required file/directory is not safely writable
- source changed between the read basis and replacement attempt


Claude-specific:

effective disableAllHooks=true at any measured effective layer
→ integration cannot claim delivery

including layers Corral never mutates but must inspect.


Codex-specific:

- notify occupied by non-Corral value
- notify has incompatible type


Also accept a mandatory pre-replacement validation gate:

construct complete candidate content
→ parse the complete candidate again using Corral's strict parser for
   that provider's measured configuration grammar
→ only a successful full parse is eligible for atomic replacement


If reparsing fails:

→ D4 refusal
→ leave original bytes untouched


But wording correction:

Do not call this:

“parse with the provider's own parser”

unless Corral literally invokes that parser.

Call it:

Corral's provider-specific strict validation parser,
whose accepted grammar is tied to measured provider behavior.


This gate is necessary but not sufficient to prove provider acceptance;
the supported-version matrix remains the empirical authority.


The Codex hard-failure measurement raises the write-safety bar,
but the same gate applies to Claude as cheap defense-in-depth.


Q3 — provider-specific editing representation

接受。

Replace ambiguous D3 wording with an explicit two-level preservation model.


Universal invariant:

Corral preserves user-owned configuration semantics outside the exact
Corral-owned integration surface.


Representation policy is provider-specific.


Claude settings.json:

- strict JSON only
- parse whole document
- structured merge
- preserve unknown keys/values semantically
- serialize complete valid JSON using the accepted canonical formatting
- no attempt to preserve byte layout/key ordering
- Corral MUST NOT introduce comments


Reason:

measured Claude configuration behavior treats comments/JSONC as invalid
and itself rewrites the document structurally.


Codex config.toml:

- parse/edit as TOML
- preserve unrelated existing bytes/layout as far as the chosen
  format-preserving editor contract allows
- retain comments
- retain unrelated key order/spacing
- mutate only the owned notify surface


This means delete the overly broad phrase:

“additive structured editing over a preserved original”

if “preserved” can be read as byte preservation for every provider.


Freeze instead:

Semantic preservation is universal.
Byte/format preservation is provider-specific and required where the
measured provider format/workflow makes it meaningful.


Do not make Claude imitate Codex editing mechanics,
and do not make Codex pay for Claude's whole-file rewrite behavior.


Q4 — repeated repair circuit breaker

接受 initial policy:

3 automatic repairs within a rolling 24-hour window.

On the next matching auto-repair opportunity:

→ do NOT rewrite
→ open the circuit breaker for that fingerprint
→ Limited awareness
→ surface explicit resolution path


Fingerprint:

(provider, config target, drift class)


Initial drift classes:

- Corral-owned entry missing
- Corral-owned entry present in an older Corral-owned representation
- ownership conflict


But ownership conflict is already non-auto-repairable,
so it must not consume the same “three repair attempts” budget as the first
two.

It goes directly to:

Limited awareness / explicit resolution.


Important correction:

The breaker MUST NOT silently close merely because the rolling 24-hour
window later contains fewer than three historical repairs.

Otherwise a dotfiles authority can create:

repair for a day
→ automatic retry tomorrow
→ repair for a day
→ automatic retry tomorrow

forever.


So:

rolling 24h
determines when the breaker opens.

Once open,
it remains open until an explicit user-controlled reconciliation action
successfully re-establishes intent/ownership, e.g.:

corral integration repair

or equivalent explicit enable/re-enable flow.


That explicit action:

- rechecks current ownership/config
- if successful, clears the breaker and repair history for that fingerprint


Daemon restart alone MUST NOT clear it.


Therefore the breaker/history is Corral-owned durable operational state.

If the current registry schema cannot represent:

- fingerprint
- repair timestamps/count window
- breaker-open state

then this is a durable-state expansion and must be treated accordingly
under the already-frozen storage clock.

The founder acceptance in this grill covers the policy decision;
implementation still needs the appropriate schema/migration review.


Initial default:

3 auto-repairs / rolling 24h

is a dogfood-tunable policy default,
not a wire constant.


Future tuning may become stricter or looser based on dogfood,
but no implementation may silently exceed the currently accepted repair
authority.


Core invariant:

Enabled authorizes bounded self-repair, not an endless configuration
tug-of-war.


Q5 — process recognition grammar

接受 split sealing.


Seal now only the facts actually established by the matrix.


Accepted recognition primitives include:

- resolved executable identity/path is evidence;
  raw argv[0] is never sufficient identity evidence

- provider executable may sit one runtime hop below a launcher/wrapper;
  recognition must follow only measured provider-specific shapes

- truncated comm names are not primary identity evidence

- Claude hook ancestry has the measured provider-specific lower-chain shape

- Codex notify's measured parent relationship can be used only for the
  exact claim it supports

- arbitrary descendant-of-provider is NOT sufficient recognition evidence
  because providers spawn unrelated children such as git


Do NOT seal:

- tmux/screen/nohup/general terminal-host ancestry
- macOS host-chain shapes not yet measured
- Homebrew installation shapes not yet measured


Most importantly:

Unsealed upper-chain facts may be collected diagnostically,
but they MUST NOT contribute the evidence required for a user-visible
provisional session claim.


So until a matrix row is sealed:

implementation cannot say:

“this ancestor chain proves a supported external provider session.”


It may still obtain sufficient evidence through an independent sealed
path, such as provider integration delivery.


This preserves the Q5 display-gate invariant:

The recognizer may know more candidates than the UI is allowed to claim.


Matrix expansion is additive evidence work;
it does not require reopening the structural recognizer model.


Q6 — Codex ghost thread-id

Reject the exact proposed promotion rule:

“same thread-id arrives a second time
→ user-visible Session”

as a frozen rule.


The spike proved:

a Codex notify thread-id can name provider-internal work
that is not the user's interactive session.

Therefore the evidence carried by notify is:

a provider thread identity emitted an event

not automatically:

a user-facing live session exists.


Repetition increases confidence that the thread is persistent,
but repetition alone does not change the semantic type of the thing being
observed.

An internal provider thread could also emit multiple events in a future
version.


Therefore split two concepts:


1. Runtime/provisional row

User-visible provisional row is justified by approved runtime-recognition
evidence.

It represents:

a supported provider runtime appears to be running here

with:

Status unknown

It does NOT require a provider thread/session id to exist yet.


2. Provider identity candidates

Unknown Codex notify thread-ids are recorded as live/internal candidate
binding evidence associated with the observed runtime.

A new candidate thread-id MUST NOT mint an additional user-visible row
merely because a notify arrived.


Thus the measured sequence:

same Codex process
→ real turn notify thread A
→ internal title-generation notify thread B

produces at most:

one runtime row

plus internally:

candidate identity A
candidate identity B


not:

two Sessions flashing in the UI.


Promotion from candidate identity to Session binding requires an evidence
rule that establishes that the identity represents the user-facing
provider session.

Allowed sources may include future matrix-proven provider behavior or
another strong identity primitive.

Do NOT use:

- prompt-content sniffing
- “looks like title generation”
- arbitrary lexical heuristics


And do NOT currently freeze:

second occurrence == user session

unless a separate matrix experiment demonstrates that this is a stable
provider semantic contract rather than an accidental frequency pattern.


If PR7 has no strong discriminator for external Codex notify identities,
the honest M1 result is:

runtime visible
+
provider identity unresolved
+
 continuation/control features requiring that identity unavailable

rather than minting ghost Sessions.


For managed Codex, existing launch/binding evidence may independently
satisfy a stronger ladder; this ruling does not weaken already-proven
managed identity paths.


Core invariant:

Provider-emitted identity evidence may create identity candidates;
a user-visible Session requires evidence that supports the literal claim
that this identity is the user's session.
```

### Founder rationale, verbatim — why Q6′'s proposed promotion rule was rejected

```text
这其实还是一个 heuristic，只是比内容嗅探漂亮。

Spike 现在只证明了：

> 当前那个标题线程恰好只发一次。

它没有证明：

> provider 的内部 thread 定义上永远只发一次。

如果我们把“出现两次”写成 ADR identity semantics，下一版 Codex 内部多跑一个回合就会重新铸幽灵 Session。

更干净的架构是：

process/runtime discovery 决定“有一行”；provider identity evidence 决定“这行最终绑定谁”。

这样 Q5 与 Q6 正好统一：

weak evidence
    → candidate only

strong runtime evidence
    → provisional runtime row

strong identity evidence
    → bind that row to provider Session

extra unknown notify ids
    → candidate bindings, not extra rows
```

### Founder rationale, verbatim — Q4′'s governance consequence

```text
既然你要让“第 4 次后停手”跨 daemon restart 生效，这个 circuit-breaker 就不是
简单内存 debounce，而是 Corral-owned durable fact。最好现在承认，不要实现时
为了维持“PR7 durable diff 只有 succession”把计数偷偷塞成 process-local。

数字 3 / rolling 24h 我接受作为 initial dogfood default；但“停手”应当
sticky，直到用户显式重新授权/repair，否则它最终还是一个低频 rewrite loop。
```

# Round 4 — acceptance (2026-09-02)

Digest of what round 4 froze:

| Q | Ruling |
|---|---|
| Q7′ | **ADR 0013 accepted; ADR 0014 accepted.** No unmeasured item carries an accepted ADR semantic: every accepted D-item stands on measured provider behavior, an already-accepted invariant, or a rounds 1–3 ruling. Gates: **(i) macOS upper ancestry → post-merge matrix expansion** (ADR 0014 already bars unsealed ancestry from user-visible claims: missing rows can only under-claim, never over-claim; measuring adds a matrix row and fixtures, no ADR reopening unless evidence contradicts the structural model). **(ii) public dotfiles corpus → PR7 merge gate** (not needed for grammar/ownership decisions — those stand on stronger first-party measurement; it exists so the merge engine demonstrably survives representative real-world third-party shapes: unrelated Claude hooks, multiple entries/events, nested settings, Codex TOML comments, unrelated tables/keys, realistic layout, absent Corral slot, refused ownership conflict. Sanitized deterministic fixtures, not vendored dotfiles repos; a weird file is preserved/refused honestly, never normalized until editable; the corpus never widens Corral ownership). **(iii) Homebrew channel → post-merge matrix expansion**, with one escalation: if a claimed PR7 dogfood cohort obtains the provider via Homebrew, the Homebrew matrix row becomes an **entry gate for that dogfood evidence** — the observations don't count toward the evidence window until the channel is measured and sealed; this conditional gate must not be converted into a universal merge gate. Acceptance is recorded with the three items and gates named so "accepted" is never misread as "matrix complete". Boundary stressed by the founder: **"ADR accepted" does not mean "PR7 ready to merge"** — architecture is decided and grounded; implementation still owes one merge-critical evidence item, the real-world configuration-shape fixtures |

## Founder ruling, verbatim (round 4)

```text
Q7′ — ADR acceptance and remaining evidence gates

裁决：

ADR 0013 = accepted
ADR 0014 = accepted


理由：

The remaining unmeasured items no longer carry any accepted ADR semantic.

Every accepted D-item is now supported by either:

- measured provider behavior,
- an already-accepted architectural invariant,
- or an explicit founder ruling from PR7 integration grill rounds 1–3.

The three remaining measurements therefore belong to implementation /
matrix completeness gates, not ADR acceptance.


(i) macOS upper ancestry

Gate:
POST-MERGE MATRIX EXPANSION

Not an ADR acceptance blocker.
Not a PR7 merge blocker.


ADR 0014 already freezes:

unsealed ancestry shapes may not contribute evidence required for a
user-visible provisional-session claim.

Therefore lack of a macOS tmux/screen/nohup/wrapper ancestry matrix row
cannot make the current recognizer tell a stronger story than its evidence.

Until measured and sealed:

- implementation may observe such ancestry diagnostically
- implementation may not use it to satisfy the display gate
- no claim may depend on it


Once measured:

→ add matrix row
→ add recognition fixture/rule
→ no ADR reopening required unless evidence contradicts the structural model


So this is genuinely additive evidence work.


(ii) public dotfiles corpus

Gate:
PR7 MERGE GATE

Not an ADR acceptance blocker.


The corpus is NOT needed to decide:

- JSON vs JSONC
- TOML grammar
- provider parser behavior
- mutation ownership semantics

Those are already established more strongly by first-party/provider
measurement.


Its merge-gate purpose is narrower:

the integration merge engine must demonstrate that its accepted editing
policy survives representative real-world third-party configuration
shapes.


Therefore before PR7 merge, fixtures must contain a documented sample of
real-world configurations sufficient to exercise at least:

- unrelated existing Claude hooks
- multiple hook entries/events where applicable
- unrelated nested settings
- existing Codex TOML comments
- unrelated Codex tables/keys
- realistic whitespace/order/layout
- configurations where the Corral-owned slot is absent
- configurations where ownership conflict must be refused


The corpus does not authorize widening Corral ownership.

A weird real-world file remains:

preserve / refuse honestly

rather than:

normalize until Corral can edit it.


The evidence may be sanitized/minimized into deterministic fixtures;
PR7 does not need to vendor entire people's dotfiles repositories.


Core merge claim:

Corral's structured integration editor has been tested against
representative real-world configuration shapes, not only synthetic happy
paths.


(iii) Homebrew installation channel

Gate:
POST-MERGE MATRIX EXPANSION

by default.


Homebrew does not carry an ADR semantic.

It is another provider-installation/recognition shape to measure and seal.


Until sealed:

- no Homebrew-specific recognition claim
- no extrapolation from another installation channel


However there is one operational escalation rule:

If the actual machine/cohort used for a claimed PR7 dogfood result obtains
the provider through Homebrew, then the Homebrew matrix row becomes an
ENTRY GATE FOR THAT DOGFOOD EVIDENCE.


That means:

PR7 may still merge without it.

But Corral may not count Homebrew-based dogfood observations toward the
relevant recognition/integration evidence window until that channel has
been measured and sealed.


Do not convert this conditional dogfood gate into a universal PR7 merge
gate merely because Homebrew is common.


ADR acceptance

Proceed now:

ADR 0013:
status: proposed → accepted

ADR 0014:
status: proposed → accepted


Acceptance record must name the three remaining items and their gates so
that "accepted" cannot later be misread as "matrix complete".


Suggested acceptance note:

Accepted after PR7 integration grill rounds 1–3 and the provider behavior
spikes supporting its load-bearing claims.

Remaining evidence work does not alter the accepted architecture:

- macOS upper ancestry — post-merge matrix expansion
- public dotfiles corpus — PR7 merge gate
- Homebrew provider channel — post-merge matrix expansion, promoted to a
  dogfood entry gate wherever that channel is used


A future measurement reopens an accepted ADR decision only if it
contradicts a load-bearing accepted assumption.

Ordinary expansion of the supported-version/channel matrix does not.
```

### Founder closing boundary, verbatim

```text
这里我只会再强调一个边界：“ADR accepted”现在不等于“PR7 ready to merge”。

现在状态应该是：

> Architecture decided and empirically grounded enough to accept.
> Implementation still owes one merge-critical evidence item:
> real-world configuration-shape fixtures.

这个划分很干净。macOS ancestor chain 和 Homebrew 都已经被结构性
fail-closed 包住，缺测只会造成少认，不会造成乱认；而 dotfiles corpus
直接验证 PR7 要真正写用户配置的 merge engine，所以它值得留在 merge gate。

到这里 grill 可以正式结束，不需要下一轮 founder 裁决。
```

The grill is closed. Nothing remains open in this record.
