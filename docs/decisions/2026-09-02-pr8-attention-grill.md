# PR8 Attention Grill — structural rulings over ADR 0015 / ADR 0016 (rounds 1–4)

> Status: **structural rounds closed** (2026-09-02). Rounds 1–4 froze
> the structural rulings over the proposed ADR 0015 (attention
> derivation) and ADR 0016 (history-enumerated sessions), the PR8 plan's
> shape, and the wire, copy, and CLI decisions that hang off them. Both
> ADRs stay `proposed` until the Q21 matrix measures their load-bearing
> facts; what follows is one **acceptance reconciliation** — checking the
> evidence against Q32's closing conditions and flipping
> `proposed → accepted` — never a re-grill of settled structure, under the
> PR7 principle: decision authority may precede empirical completion; ADR
> acceptance may not precede its load-bearing evidence. A matrix result
> that contradicts a load-bearing accepted assumption reopens that ruling
> with the reason recorded; ordinary expansion reopens nothing.

Questions asked by the grill session of 2026-09-02; rulings verbatim below.

# Round 1

Digest of what round 1 froze:

| Q | Ruling |
|---|---|
| Q1 | Two PRs, one shared plan: PR8a attention engine / evidence / projection / protocol / TUI+CLI (ADR 0015); PR8b history enumeration / recent list / continuation / disclosure (ADR 0016). Separate `writes:`, dependencies, tests, Definition of Done, review checklist; each PR independently correct on `main`; overlapping owners serialize rather than parallelize |
| Q2 | (a′) — a received, fresh, Attested, version-sealed provider event may be sufficient evidence for the exact positive claim it directly denotes. "Never load-bearing" is a delivery/completeness rule, not a semantic ceiling: absence proves nothing, stale never resurrects, fresher eligible evidence may invalidate. Canonical wording (ARCHITECTURE §2) must be corrected so it cannot be read as "a hook can never be sufficient". Invariant: *unreliable delivery does not imply weak semantics* |
| Q3 | D4 accepted: among eligible fresh claims the causally newest wins; authority breaks only a genuine ordering tie. "Latest" is Corral's evidence ordering — daemon observation sequence, provider sequence where available, same-runtime order — never cross-source wall-clock comparison. Invariant: *authority controls whether a source may claim; fresh contradictory evidence controls whether the claim is still true* |
| Q4 | Attested for a history-claimed identity, with assurance **claim-scoped**: provider-owned history at a sealed path Attests "history contains Session X", never "the observed runtime is Session X" — the live binding claim still needs live corroboration. Glossary wording that makes all history-derived identity Heuristic is corrected; cwd/time/process-proximity matching of a live process to a record stays Heuristic. Principle: *assurance qualifies a claim, not an object globally* |
| Q5 | (a) — a discovered external session whose runtime is still live outside Corral gets no Continue in Corral in PR8, with the reason stated; a phase limitation, never the product invariant "live observed sessions may never Continue". No fork-now-explain-later; no early branch model |
| Q6 | (a′) — ephemeral acknowledgement of an ephemeral live item, daemon-lifetime only; restart drops it and replays nothing; fingerprint matching across restart rejected. Not the AGENTS.md durable acknowledgement, and AGENTS.md is not weakened: when a durable AttentionItem identity exists, acknowledging it is a Corral-owned durable fact and is persisted. Invariant: *no acknowledgement without a stable object; no guessed object identity across restart* |
| Q7 | (a) — no `PostToolUse` in the injected or global entry sets. The stale-Needs-You-after-native-approval gap is a recorded fidelity limitation measured in dogfood (stale duration, frequency, whether later events clear it); systematic evidence reopens it as a deliberate matrix-measured addition. (c) rejected: managed sessions have authoritative terminal evidence |
| Q8 | (a′) — a dedicated diagnostic journal, not the registry: e.g. `~/.corral/diagnostics/attention-journal.jsonl`. Records state transitions with source class, reason, assurance, freshness, notification emitted, disputes, build; never raw screen, prompt text, hook payload, tool arguments, transcript, or secrets. Deletable, unrebuildable, unmigrated, bounded. Invariant: *the attention engine never reads the journal back into product state or inference*; reporting may read it |
| Q9 | (a)+(c) — a Session Corral already holds shows Corral's own title; a pure history row shows structural metadata only (provider, directory, age, short id). No first-message, summary, or transcript parsing for a title; a provider's explicit versioned metadata index is a separate future ruling. Principle: *missing display richness is cheaper than silently expanding the history parser contract* |
| Q10 | (a′) — three gates. PR8 merge: macOS real-provider matrix plus deterministic Linux harness, every unverified path fail-closed. Linux support/dogfood entry: a real provider on a real Linux process environment verifying the whole chain (recognition → token-less evidence → binding → projection → row → lifecycle) before Linux external Know counts toward the A-thesis, trusted transitions, or the supported matrix; udocker/mock `/proc` insufficient. Conditional escalation: if PR8 would claim Linux external Know to ordinary users by default at merge, real Linux E2E becomes the merge gate. Principle: *unverified implementation may merge behind an honest capability boundary; unverified product claims may not* |

## Founder rulings, verbatim

```text
这一轮我会裁：

* Q1 = 两个 PR，共用一份 plan，但各自独立 DoD / review seam
* Q2 = (a′)：收到的、Attested 且版本封印的 hook 事件可以单独断言它直接表达的正事实；"never load-bearing"不能解释成"hook 永远不能成为充分证据"
* Q3 = 接受 latest eligible fresh claim wins
* Q4 = 接受 Attested，但 assurance 必须绑定到"具体 claim"
* Q5 = (a) PR8 拒绝 live external Continue
* Q6 = (a′) live-only，但明确它还不是 durable acknowledgement
* Q7 = (a) 不加 PostToolUse
* Q8 = (a′) journal 接受，但必须明确是可删除 diagnostic evidence，且不能落 raw terminal/provider payload
* Q9 = (a)+(c)
* Q10 = (a′)：真 Linux E2E 不是无条件 merge gate；但在它验证前，Linux external Know 不得进入 supported/dogfood claim


PR8 grill — round 1 rulings


Q1 — split PR8

裁决：

two PRs
one shared implementation plan


建议命名概念上：

PR8a:
attention engine / evidence / projection / protocol / TUI+CLI five-state surface
→ ADR 0015

PR8b:
history enumeration / recent list / continuation policy / disclosure
→ ADR 0016


共用一份 plan 是合理的，因为：

- roadmap milestone 仍是同一个 PR8 product slice
- 两边共享 terminology / UI integration context
- 不需要为了 governance page count 人为复制背景


但 plan 必须把两边写成独立 workstream：

- separate writes:
- separate dependencies
- separate tests
- separate Definition of Done
- separate review checklist


每个 PR 必须能在 main 上独立成立。

不能出现：

PR8a merge 后 repo 暂时违反 invariant，
等 PR8b 来修

或反过来。


如果两个 workstream 都写同一个 TUI/protocol owner，
默认串行 branch/rebase，
不要为了"两个 PR"强行并行制造 owner conflict。


核心原则：

One plan may describe one milestone;
each PR must still be independently reviewable and correct.


Q2 — can an attested hook be sufficient for primary status?

修改成 (a′)。

接受：

A received, fresh, Attested, version-sealed provider hook event may be
sufficient evidence for the exact positive semantic claim that the event
directly denotes.


例如，若版本矩阵已经封印：

PermissionRequest
→ provider is currently waiting for user permission

则这个收到的事件本身可以产生：

Needs You


不要求再找一个 screen heuristic 给它"二次签字"。


同理，只有在事件语义已经实测并封印的情况下：

turn-start-like event
→ Working

turn-complete/stop-like event
→ Ready

才可以单源断言。


但是 ARCHITECTURE §2 的：

"one weighted source, never load-bearing"

必须澄清。

它不能继续被理解成：

hook can never be sufficient evidence

否则 external observed sessions 在 M1 确实无法完成 Know。


正确解释：

Hooks are never load-bearing as an availability/completeness assumption.

也就是说：

- Corral correctness cannot assume every hook is delivered
- absence of a hook is not proof of absence of a state
- missing/delayed hook must not wedge the engine
- stale hook must not resurrect state
- another fresher eligible evidence source may contradict/invalidate it


但：

a hook that DID arrive
and whose semantics are directly attested
may itself be load-bearing for that positive claim.


因此不要把所有 hook 一概叫"weighted heuristic"。

要区分：

transport reliability
与
semantic authority of a received event。


这需要同步修正 canonical wording，
否则 ADR 0015 与 ARCHITECTURE 会自相矛盾。


核心不变量：

Unreliable delivery does not imply weak semantics.

A received attested event may prove what it directly says;
the system must never infer the converse from an event that did not arrive.


Q3 — synthesis rule

采纳 D4：

among eligible, fresh semantic claims,
the causally newest claim wins.

Authority breaks a genuine ordering tie;
it does not let stale evidence dominate fresher contradictory evidence.


强调：

"latest" 必须是 Corral 能建立的 evidence ordering，
不是简单比较不同机器/来源的 wall-clock timestamp。


优先使用：

- daemon observation sequence
- provider event sequence where available
- same-runtime epoch/order

来确定新旧。


典型：

fresh PermissionRequest
→ Needs You

随后当前 screen 明确显示 agent 已继续工作
→ fresher Working-capable evidence
→ Needs You invalidated


不能因为 PermissionRequest 来源"更权威"
就在 freshness horizon 内一直压住现实。


同样：

Ready
不能因为曾经被高权威 Stop 声明
就在已经出现新活动后继续存活。


核心不变量：

Authority controls whether a source is allowed to make a claim.
Fresh contradictory evidence controls whether that claim is still true.


Q4 — provider history identity assurance

接受 Attested，
但必须把 assurance 定义成 claim-scoped，而不是 entity-scoped。


从 provider-owned history storage，
按照版本封印的路径/格式直接读取出：

provider_session_id = X

可以 Attested 地声明：

"provider-owned history contains Session X"


它不能同时 Attested 地声明：

"the currently observed process/runtime is Session X"


后者是另一个 claim：

live runtime ↔ provider session identity binding

它仍需要 live corroboration。


因此 glossary 要澄清：

Attested:
the evidence directly supports the specific claim being made.

For a live binding claim,
provider history alone is insufficient;
live corroboration is required.


而：

cwd/time/process-start proximity
把一个 live process 猜到某个 history record

仍然是：

Heuristic history correlation


所以需要从 glossary 删除/修正那种会让：

all history-derived identity == Heuristic

的笼统措辞。


HistoryBinding 可以是 Attested，
因为它只断言 provider-owned historical identity，
不偷渡 live association。


这也让 first-run recent list 合法：

provider-owned history entry
→ Attested historical Session identity
→ eligible for safe history/resume operations according to ADR 0016

无需假装用户点击本身提供 identity assurance。


核心原则：

Assurance qualifies a claim, not an object globally.


Q5 — Continue for currently live external sessions

选择 (a)。

PR8：

discovered external session
+
runtime still live outside Corral

→ Continue in Corral unavailable


原因：

PR8 尚未实现完整 branch UX：

- original branch retained visibly
- resumed branch gets its own row
- relationship disclosure
- attention ownership remains truthful
- old branch not silently presented as resolved


所以不做：

(c) fork now, explain later


这会制造一个控制动作，
却没有产品能力诚实表达它产生的 topology。


也不要求 PR8b提前实现整套 branch model。


用户应得到明确原因，例如概念上：

Still running outside Corral.
Continuation is unavailable while this session remains live.


这只是 PR8 阶段限制。

它不能被写成最终产品 invariant：

"live observed sessions may never Continue"。


如果 M1 最终仍需要 rung 3 fallback，
拥有 branch UX 的后续阶段必须补齐。


Q6 — acknowledgement persistence

选择 (a′)。

PR8 current attention items 没有 durable identity。

因此现在持久化：

AttentionAcknowledged(session, reason, evidence fingerprint)

是不诚实的，
因为 restart 后没有可靠方法证明：

new live item == previously acknowledged item


拒绝这种 fingerprint matching。


PR8 的当前操作应明确建模为：

acknowledge/dismiss the current live attention item

其生命周期：

daemon/runtime attention projection lifetime only


daemon restart：

- live attention projection disappears
- session returns to evidence-derived state / Unknown as appropriate
- no old acknowledgement is replayed onto a newly reconstructed item


但是要修正文档语言：

这还不是 AGENTS.md durable-state意义上的 durable acknowledgement fact。


也就是说不能一边叫它：

canonical AttentionAcknowledged

一边说：

we intentionally don't persist acknowledgements.


正确边界：

PR8 implements ephemeral acknowledgement of an ephemeral attention item.

When Corral later introduces durable AttentionItem identity,
acknowledgement of that durable item becomes a Corral-owned durable fact
and must be persisted.


所以：

no durable schema diff now

但不要修改/削弱：

acknowledgements are durable when their referenced attention fact is durable.


核心不变量：

Do not persist an acknowledgement without a stable object to acknowledge,
and do not guess object identity across restart.


Q7 — PostToolUse

选择 (a)。

PR8 不新增 PostToolUse global/injected hook。


当前缺口：

PermissionRequest
→ user approves natively
→ no immediate hook saying blocker cleared

确实可能导致 Needs You 留到：

- fresher screen evidence
- Stop / another provider event
- freshness expiry

才消失。


这是 fidelity limitation，
不是现在应该凭推测增加一个高频 hook 的理由。


尤其 global external sessions：

PostToolUse 每次 tool call 都触发 relay，
成本和噪音面明显高于现有 entry set。


PR8 dogfood 应先记录：

- stale Needs You duration
- how often native approval produces noticeable stale rows
- whether existing later events normally clear it quickly


如果数据证明这是系统性 missed-resolution source：

→ measure PostToolUse semantics/cost
→ amend integration matrix
→ add deliberately


不要选 (c)：

managed sessions已有 authoritative terminal evidence，
它们最不需要额外 hook。


Q8 — attention diagnostic journal

接受 (a′)。

建立 dedicated diagnostic journal，
而不是 authoritative registry/event store。


例如概念位置：

~/.corral/diagnostics/attention-journal.jsonl

而不是让文件名/位置看起来像 durable product truth。


可以记录：

- timestamp / monotonic evidence sequence where useful
- CorralSessionId
- previous projected state
- new projected state
- evidence source class
- reason code
- assurance
- freshness metadata
- notification emitted yes/no
- dispute record
- build/version


不得默认写入：

- raw terminal screen
- prompt text
- hook payload
- tool arguments
- provider transcript
- secrets/arbitrary session content


`attention dispute`：

append a diagnostic dispute record

而不是修改历史行。


`attention report`：

可以读取这个 journal 做 dogfood evidence aggregation。

这不违反"不读回"，因为真正的不变量应写成：

The attention engine never reads diagnostic journal data back into product
state or semantic inference.


reporting/diagnostics 当然可以读取 diagnostics。


journal：

- deletable
- rebuildability not promised
- not migrated as product semantic state
- never used to suppress/produce attention
- bounded/rotated by operational policy


这样才能支撑：

100 trusted Needs You transitions
zero avoidable false Needs You
14-day dogfood evidence

而不是靠 tracing grep。


Q9 — historical rows without titles

选择：

(a) + (c)


If Corral already has its own Session record with a trustworthy display title:

→ use Corral's title


Pure provider-history discovery:

→ do not parse provider transcript/summary merely to obtain a title


display using available structural metadata, e.g.:

Claude Code
~/work/demo
2h ago
<short provider session id>


不要为了 UI 好看提前实现：

- first-message parsing
- summary extraction
- transcript decoding
- title heuristics


如果 provider 将来提供一个明确、版本化、独立的 metadata index，
是否读取它可以单独裁。

当前 summary/transcript record 属于 M2 history parsing boundary。


核心原则：

Missing display richness is cheaper than silently expanding the history
parser contract.


Q10 — where the provider matrix must run

修改成 (a′)。

PR8 merge 可以在：

- macOS real-provider matrix for measured provider semantics
- deterministic/mock Linux process/discovery harness

的基础上进行，

前提是未验证的 Linux external-Know path保持 fail-closed。


也就是说：

true Linux E2E is not automatically a repository merge gate.


但是在真实 Linux provider E2E 完成前：

Corral MUST NOT claim Linux external-session Know as a validated supported
surface.


这产生三种 gate：


1. PR8 merge gate

mock/unit/integration coverage sufficient to verify internal mechanics

+
all code paths must fail safely when evidence is insufficient


2. Linux support/dogfood entry gate

Before counting Linux external observed-session behavior toward:

- A-thesis dogfood
- trusted Needs You statistics
- external Know validation
- supported provider/platform matrix

must run a real provider on a real Linux process environment and verify
the complete chain:

process recognition
→ token-less/global integration evidence
→ identity/binding
→ projection
→ user-visible row/status
→ lifecycle update


udocker/mock `/proc` is not sufficient for this evidence.


3. Conditional merge escalation

If PR8 implementation enables Linux external-session claims to ordinary
users by default at merge time,
rather than leaving the unverified path gated/unclaimed,

then real Linux E2E becomes a PR8 merge gate.


So the rule is:

Unverified implementation may merge behind an honest capability boundary.
Unverified product claims may not.


Whether you personally have a Linux box does not change this architecture
decision.

If no suitable Linux environment is currently available:

→ merge can still proceed fail-closed
→ Linux external Know remains matrix-unvalidated
→ it cannot start the relevant dogfood evidence window


核心原则：

Mocks can verify mechanics.
Only the real provider + real OS process environment can validate the
external Know claim.
```

### Founder framing, verbatim — why Q2 and Q6 were the corrections

```text
我认为这轮最值得修正的是 Q2。

`hook never load-bearing` 如果逐字解释成"一个 hook 永远不能单独证明 Needs
You"，那么它和 Observed Know 是直接矛盾的。但也不需要反过来把 hook 神化成
可靠通道。正确拆法是：

> hook delivery 不可靠；收到的某些 hook event 的语义却可以非常强。

"没收到 PermissionRequest"什么都证明不了；但一个已经 Attested、版本封印的
`PermissionRequest` 到了，完全可以证明"这一刻 provider 正在等用户许可"。

另一个需要写清的是 Q6：我同意 PR8 不持久化当前 ack，但不要因此改掉 AGENTS
的 durable acknowledgements 原则。这里真正缺的是 durable AttentionItem
identity，不是"ack 本来就不该 durable"。
```

# Round 2

Digest of what round 2 froze:

| Q | Ruling |
|---|---|
| Q11 | (a) — PR8a first; PR8b rebases. PR8a must be independently dogfoodable without history rows; PR8b independently reviewable as an additive history/continuation surface. No transitional ranking or presentation invented by PR8b |
| Q12 | (a′) — version from the runtime's installation, but only when Corral can bind that metadata to the concrete runtime observed. Tiers: a sealed versioned path establishes the version directly; a mutable package root is read but yields Unknown when its metadata changed after the process started; a managed launch establishes the version at the launch boundary (sealed metadata, `--version` fallback) and binds it to the Run in memory; an external runtime only where the sealed recognizer chains runtime ↔ installation ↔ metadata. Unbound or unreadable → unsupported/unvalidated version, no primary semantic assertion from version-sensitive evidence. Cached by runtime/install identity, never per event. Invariant: *version sealing applies to the provider version that produced the event, not the version currently on disk* |
| Q13 | Open-ended major.minor inheritance rejected: payload compatibility is not semantic compatibility. A semantic adapter claim is sealed only for an exact measured version or an explicitly approved finite range whose semantic compatibility the matrix established. A new version: journal "matrix expansion due", parsing continues diagnostically, runtime stays visible, version-sensitive claims do not become Working/Needs You/Ready, Limited awareness. Matrix expansion automation is an M1 release prerequisite. Principle: *forward-compatible parsing may be optimistic; semantic attestation may not* |
| Q14 | (a)+(y) with discipline: a rule is semantic-capable only with real positive captures, every other captured state as a negative, adversarial near-miss fixtures, no unresolved noise-catalog false positive, deterministic fixtures in `verify`, and the exact asserted state declared. PR8 screen rules may assert Needs You and Ready; screen Working is diagnostic only. A seal is revocable: a false positive disables/demotes the rule immediately, is P1 if it can create a false Needs You, adds a minimized negative fixture, and re-seals only with evidence |
| Q15 | Two numbers change: hook Needs You 10 → 5 min; external hook Ready 12 h → 2 h. Activity silence 3 s, screen settle 200 ms, hook Working 15 min kept. Policy defaults, not wire contract. The journal records state entered, source, configured horizon, actual expiration, and whether contradictory evidence arrived first |
| Q16 | (c) — a manual reproducible real-Linux evidence artifact first, then `verify-release` automation. The artifact lives in `docs/evidence/` (a product/support claim, not research), proving all seven chain steps and including a positive Claude flow, a noise process, exit, version determination, and a Needs-You-capable path where claimed. No suitable Linux host is currently confirmed; the plan may not assume one; merge stays fail-closed |
| Q17 | No hot reload, no SIGHUP semantics, no reload RPC, no watcher: load once at daemon start; a changed manifest means a daemon restart |
| Q18 | (1) `attention.acknowledge` carries `session_id` and the ephemeral `attention_item_id`; idempotent per item; a stale id is a no-op (`StaleAttentionItem`) and never acknowledges the replacement item. (2) A bare `disclosed: bool` is rejected: `session.continuation` returns decision, disclosure text/code, and `disclosure_revision`; `session.resume` carries the revision; the daemon recomputes eligibility and requires a matching revision where disclosure is required — *disclosure correlation, not consent*. (3) Ranking accepted: Needs You, Ready, Working, Unknown + live runtime, other recent/non-active rows, Exited; recency then deterministic id within a tier; acknowledgement does not change the primary state's rank; history-only rows sit in their own non-live tier. (4) Ready is acknowledged only when Open succeeded — data channel bound and initial snapshot established — never on the attach request; Needs You never by viewing |
| Q19 | Confirmed, as a real ephemeral wire identity: a new `AttentionItemId` on each entry into Needs You or Ready; the same id across an evidence-source change for the same blocker; leaving and re-entering mints a new id and re-arms notification and badge. Evidence-instance identity and attention-item identity are distinct. Never reconstructed across restart, not persisted. `RuntimeEnded` produces no item in PR8; tray policy owns exit notifications later |
| Q20 | (a), with support made capability-scoped: external Codex in M1 supports discovery, identity, and turn-completion/Ready where sealed, and does not support approval/awaiting-input detection. The release blocker reads: *no systematic missed states within a provider/version/surface/evidence capability that Corral claims as supported*. PRODUCT §10 states the Codex gap explicitly. Dogfood validity: a cohort dominated by external Codex cannot validate the Observed Needs You thesis; the 100-transition count cannot be met by a surface that never reports; the kill window needs enough flows with supported Needs You detection or the result is *insufficient evidence*, not pass |

## Founder rulings, verbatim (round 2)

```text
这一轮我会裁：

* Q11 = (a) 8a 先、8b 后
* Q12 = (a′) 从 runtime 对应安装物取版本，但必须证明"这个版本属于这个正在跑的 runtime"
* Q13 = 拒绝开放式 major.minor 继承；改为 exact / explicitly sealed range
* Q14 = (a)+(y)，加 adversarial negatives 与可撤销 seal
* Q15 = 改两数：Needs You 5 min，Ready 2 h；其余先照计划
* Q16 = (c)，但证据进 `docs/evidence/`；目前我没有可确认的现成真 Linux provider host
* Q17 = 不加 reload
* Q18 = 3/4 基本接受；1 必须带 attention item identity；2 把裸 `disclosed: bool` 收紧
* Q19 = 确认，并把 item identity 真正做成 ephemeral wire identity
* Q20 = (a)，但 capability-scoped support 必须写进 release gate


PR8 grill — round 2 rulings


Q11 — PR8a / PR8b merge order

选择 (a)。

PR8a first.
PR8b rebases on PR8a.


原因：

PR8a establishes the live list's final M1 semantic projection:

- attention state
- runtime secondary facts
- ranking
- acknowledgement behavior
- presentation vocabulary

PR8b then adds historical/recent rows into that already-existing list model.


不要让 PR8b 为一个暂时不存在 attention projection 的列表
发明 transitional ranking/presentation，
然后 PR8a 再改第二次。


Merge invariant:

PR8a must be independently dogfoodable without history rows.

PR8b must be independently reviewable as an additive history/continuation
surface after rebasing onto PR8a.


Q12 — establishing provider version for a runtime

方向接受 (a)，但增加一个重要限制：

Reading a nearby package.json is sufficient only when Corral can bind that
metadata to the concrete runtime being observed.


不能：

process P 正在跑旧版本
+
installation auto-updated in place
+
Corral 读取当前 package.json 的新版本
→ 把新版本错误绑定给 P


因此 version evidence 分级处理。


1. Versioned installation path

如果 sealed recognizer shape 本身含版本，例如：

.../claude/versions/<v>/...

且该 path 确实对应 runtime identity：

→ version may be established directly from sealed path semantics.


2. Mutable package root

例如 npm/current-style installation：

resolved provider installation
→ package.json

可以读取，
但必须考虑 runtime start time 与 installation metadata mutation。


如果 provider package/version metadata 在该 process 启动以后发生过更新，
Corral不能可靠声称：

current metadata version == running process version

→ runtime provider version = Unknown
→ semantic events are not version-sealed
→ Limited awareness


3. Managed launch

managed runtime 应尽可能在 launch boundary 建立版本，
并把该版本绑定到 concrete Run/runtime in memory.

优先：

sealed installation metadata

必要时：

provider --version fallback


不要每个 hook/event spawn `--version`。


4. External runtime

只在 sealed recognizer/channel 能把：

runtime
↔ provider installation
↔ version metadata

安全串起来时建立版本。


读不到/无法绑定：

→ Unsupported/unvalidated version
→ no primary semantic assertion from version-sensitive adapter evidence


另外 version lookup 应 cache by concrete runtime/install identity，
而不是每个事件重读磁盘。


核心不变量：

Version sealing applies to the provider version that produced the event,
not merely the provider version currently installed on disk.


Q13 — version drift policy

拒绝 proposed (b) 的：

same major.minor
+
payload shape still parses
→ automatically inherit Attested semantics


原因：

Payload compatibility
≠
semantic compatibility.


尤其 Needs You：

同样字段名、同样 JSON shape，
provider 完全可能在 patch release 改变：

- event timing
- event multiplicity
- meaning of completion
- approval lifecycle
- ordering

而 decoder 仍然 100% parse 成功。


这会直接违反 precision-first attention trust model。


所以冻结：

A semantic adapter claim is version-sealed only for:

1. an exact measured version; or
2. an explicitly approved finite/version range whose semantic compatibility
   has itself been established by matrix evidence.


例如可以有：

Claude 2.1.252
Claude 2.1.258

或未来经证据批准：

Claude 2.1.252..=2.1.258

但不能自动解释为：

Claude 2.1.x forever


新版本第一次出现：

→ journal: matrix expansion due
→ parsing may continue diagnostically
→ runtime remains visible
→ version-sensitive semantic claims that lack a sealed compatibility row
   do not become Working/Needs You/Ready
→ Limited awareness


这确实比 major.minor inheritance 更容易出现短期黑屏。

正确修复是：

automate matrix expansion

而不是降低 evidence standard。


可以把 version matrix 自动化列为 M1 release prerequisite，
尤其自动更新频繁的 provider。


核心原则：

Forward-compatible parsing may be optimistic.
Semantic attestation may not be.


Q14 — screen evidence sealing

采纳：

(a) sealing discipline
+
(y) only Needs You / Ready in PR8


一条 screen rule 升为 semantic-capable 之前至少需要：

1. real positive captures
   from the claimed provider/version/surface

2. all currently captured other semantic states
   exercised as negatives

3. adversarial near-miss fixtures

4. provider noise catalog has no unresolved false-positive case
   for that rule

5. deterministic regression fixtures live in verify

6. exact asserted semantic state is declared in the manifest


不要只测：

"permission prompt 可以匹配"

还必须测：

"普通 prompt / tool output / error / help / completion / redraw
不会被它误匹配"。


PR8 允许 screen manifest assert：

Needs You
Ready


PR8 screen Working：

diagnostic only.


Working 已有：

- runtime/activity
- suitable hook evidence

而"看起来正在工作"的 screen pattern 通常比 blocker/ready visual
更容易产生模糊边界。


另外 seal 不是永久神谕。

如果 dogfood dispute / provider upgrade 出现 false positive：

→ immediately disable/demote affected rule
→ P1 if it can create false Needs You
→ add minimized negative fixture
→ re-seal only with evidence


核心原则：

Screen rules earn semantic authority through demonstrated precision,
not merely through a recognizable positive screenshot.


Q15 — initial freshness horizons

我会改两个数。


activity silence:
3 s
→ KEEP initially

screen settle:
200 ms
→ KEEP

hook Working:
15 min
→ KEEP

hook Needs You:
10 min → 5 min

hook Ready external:
12 h → 2 h


Needs You 5 min：

外部 native approval 没有 resolution hook 时，
长时间挂着一个已经解决的 Needs You
比过早退回 Unknown 更伤 trust。

Unknown 是诚实降级。


Ready 12 h 我认为过长。

Ready 是 semantic claim，
而 external session 最危险的失真路径是：

Stop observed
→ Ready
→ later UserPromptSubmit/new activity hook missed
→ Ready continues


12 小时已经不太像 freshness horizon，
更像半永久缓存。


初值：

external hook Ready = 2 h

足够支持"回来看看刚结束的 session"，
又不会让早上的一次 Stop 到晚上仍被当 current semantic truth。


这些都是 policy defaults，
不是 wire contract。


journal 必须记录：

- state entered
- evidence source
- configured horizon
- actual expiration
- whether contradictory evidence arrived first

dogfood 后再调。


Q16 — real Linux external-Know evidence

选择 (c)。

第一阶段：

manual/reproducible real-Linux evidence artifact

第二阶段：

automated verify-release coverage when infrastructure exists.


但 canonical artifact 应进：

docs/evidence/

不是 docs/references/。


因为这不是 benchmark/research；
它是在证明一个 product/support claim。


例如：

docs/evidence/
  pr8-linux-external-know-2026-xx-xx.md


至少逐步证明：

1. real provider process exists in real Linux process environment
2. recognizer establishes approved runtime evidence
3. global/token-less integration event arrives
4. provider identity/binding is established at the allowed assurance level
5. attention engine produces the expected projection
6. TUI/CLI shows the corresponding row/state
7. lifecycle/state change converges correctly


最好同时包含：

- Claude positive flow
- known-negative/noise process
- process exit
- version determination
- one Needs You-capable path if that provider/surface claims it


之后自动化：

verify-release

而不是普通 verify，
因为 real provider credentials/runtime make it unsuitable for ordinary
per-PR CI.


在 manual artifact 出现之前：

Linux external Know remains unvalidated and cannot contribute to the
dogfood/release evidence claim.


关于机器：

我当前没有可确认的证据表明你已经有一台适合跑真实 provider +
真实 /proc 链路的 Linux host。

所以计划不能假设它存在。

这不阻塞 PR8 merge，只维持之前裁决的 fail-closed support boundary。


Q17 — manifest reload

照 D6：

PR8 不加 hot reload。

不加：

- SIGHUP semantics
- manifest.reload RPC
- filesystem watcher


daemon start:
→ load built-in + override manifests once

manifest changed:
→ restart daemon


鉴于当前 daemon 有 idle lifecycle，
dogfood 成本足够低。


以后若实际 rule iteration 证明重启影响开发效率，
再基于真实需求增加 reload surface。


不要为了 development convenience
现在新增永久 control-plane contract。


Q18 — protocol shape

四条里，(3) 基本接受，(1)(2)(4) 需要收紧。


(1) attention.acknowledge

同意：

NO durable command_id

因为它不产生 durable semantic fact。


但请求绝不能只带：

session_id


必须带当前 ephemeral：

attention_item_id


例如：

attention.acknowledge {
    session_id,
    attention_item_id
}


原因：

item A Needs You
→ client sends ack A
→ network delayed
→ A resolves
→ item B Needs You appears
→ stale ack arrives

如果 ack 只按 session：

→ B 被错误 acknowledge


所以 idempotency 来自：

same attention_item_id acknowledged repeatedly
→ same result

而不是：

ack whatever item is current.


stale item id：

→ no-op / StaleAttentionItem
→ MUST NOT acknowledge replacement item


(2) continuation disclosure

我不喜欢裸：

disclosed: bool


它无法证明客户端展示的是当前这一次 continuation 决策对应的 disclosure。


最低限度改成：

session.continuation
→ returns:
   decision
   disclosure text/code
   disclosure_revision


session.resume
→ includes:
   disclosure_revision


daemon 在 resume 时：

- recompute current continuation eligibility
- require revision still matches where disclosure is required
- otherwise reject and require fresh preflight


这个 revision 的语义：

the client obtained the disclosure associated with this exact continuation
decision

不是：

user consented

也不是 security authorization。


客户端是否真的 render 了文本仍然是 client UX contract，
协议无法证明人的眼睛看过。


因此 wire doc 明写：

Disclosure correlation, not consent.


如果你坚持保留 bool，
至少也必须同时有 decision revision；
单独 bool 太容易产生 stale disclosure race。


(3) session.list ranking

接受 initial daemon-owned ranking：

Needs You
Ready
Working
Unknown + live runtime
other recent/non-active rows
Exited


同一 rank：
→ recency
→ deterministic id tie-break


注意：

acknowledgement affects badge/item attention,
不必自动改变 primary state's ranking。

例如 acknowledged Ready 仍可以保持 Ready，
只是不再有 unacknowledged badge。


PR8b 加 history rows 后，
历史-only rows应进入其自己的 non-live recency tier，
不要突然插进 Needs You/Ready/Working 之间。


(4) Ready auto-ack on Open

接受语义：

successfully viewing/opening a Ready session acknowledges that Ready item.


但不要在收到：

terminal.attach request

的瞬间 ack。


只有 attach 成功建立到足以算"Open succeeded"的点才 ack。

概念上：

terminal data channel bound
+
initial snapshot successfully established

→ acknowledge matching Ready attention_item_id


如果：

attach rejected
snapshot failed
channel negotiation failed

→ Ready stays unacknowledged


Needs You：

viewing alone does not acknowledge

→ explicit acknowledge/resolution only


这保持之前冻结的：

Ready viewing acknowledges badge
Needs You viewing does not.


Q19 — attention item identity

确认，但把它做成真正可引用的 ephemeral identity。


每次进入：

Needs You
or
Ready

创建新的：

AttentionItemId

只在 current daemon/live projection 生命周期有效。


同一个 semantic item：

same blocker
+
evidence changes from hook → screen
or screen → hook

→ SAME AttentionItemId


状态离开：

Needs You → Working

之后再次：

Working → Needs You

→ NEW AttentionItemId
→ notification re-armed
→ badge unacknowledged again


同理 Ready。


这意味着 engine 需要区分：

evidence instance identity

和：

attention item identity


不要因为证据 source 改变就重发通知。


AttentionItemId：

- ephemeral
- protocol-visible where acknowledgement requires it
- never reconstructed across daemon restart
- not persisted in PR8


RuntimeEnded：

PR8 不产生 AttentionItem。

Exited 是 primary state transition，
不是 Needs You/Ready attention class。


是否对 process exit 发送 tray notification：

later tray/notification policy owns it.


Q20 — external Codex approvals and release gate

选择 (a)。

但把：

"supported hooked flow"

改成 capability-scoped support。


External Codex M1 可以支持：

- runtime discovery where sealed
- identity where sealed
- turn-completion/Ready evidence where available

而不支持：

approval / awaiting-input Needs You detection


因此：

missed external-Codex approval

不是：

systematic miss inside an advertised supported Needs You hook


而是：

known provider/surface capability gap.


所以 release blocker 应表述成：

No systematic missed states within a provider/version/surface/evidence
capability that Corral claims as supported.


PRODUCT §10 / support matrix 必须明确：

Codex external interactive:
Needs You / approval detection — unsupported in M1
(or Limited awareness, exact product wording)


managed Codex：

若 sealed screen/OSC evidence 能可靠检测 approval，
可以独立支持 Needs You。


还要加一条 dogfood validity rule：

A dogfood cohort dominated by external Codex cannot by itself validate
the Observed Needs You thesis if that surface has no Needs You evidence
primitive.


也就是说：

100 trusted Needs You transitions

不能拿：

"external Codex 没误报，因为它从来没报"

来满足。


A-thesis kill window 必须包含足够数量真正具备 supported Needs You
detection 的 observed flows，
否则结果只能标：

insufficient evidence

而不能判：

pass.


如果未来 Codex 提供 approval event / OSC 被 matrix seal：

→ capability matrix additive expansion
→ then systematic misses become release-relevant for that capability.
```

### Founder framing, verbatim — the two round-2 corrections

```text
我觉得这一轮两个最关键的变化是：

第一，Q13 不要为了自动更新牺牲 attention assurance。 `2.1.252 → 2.1.258` 是
一个非常真实的运营问题，但正确答案是把 matrix expansion 变便宜、自动化，而
不是把 `2.1.*` 默认为语义兼容。尤其你们已经把"zero avoidable false Needs
You"定成 release bar，这里不能自己开洞。

第二，Q18 的 ack 必须带 item identity。 否则一个延迟的 ack 可以吃掉下一次真
正的 blocker，这会直接破坏你们刚刚冻结的"离开再进入 = 新 item、重新武装通
知"。
```

# Round 3

Digest of what round 3 froze:

| Q | Ruling |
|---|---|
| Q21 | Scenario inventory accepted with transition negatives added. AskUserQuestion and ExitPlanMode approval are Needs You when the measured scenario shows the provider blocked on an explicit user response — semantics come from "blocked on the user", never from the event or tool name. Claude positives: tool permission, AskUserQuestion, ExitPlanMode, Stop → idle. Negatives/transitions: spinner, silent long tool, compaction, `/resume` picker, API error/retry, help overlay, resize/redraw, typing at the prompt, long paste, permission-like wording in ordinary output, a Needs You prompt resolved and the screen moving on, a prompt rejected/cancelled. Codex likewise, plus every notify variant and `tui.notifications`/OSC. Capture every observed `Notification.notification_type`, and the ordering/noise of PermissionRequest, Stop, SubagentStop, background-task events. Invariant: *a semantic fixture tests both entry into the claimed state and credible near-misses / exit from it* |
| Q22 | (b) — automation gathers evidence and proposes; it never grants semantic authority. Flow: version discovered → credentialed `verify-release` matrix job → sealed scenario suite → captures → compare against sealed fixtures → proposed row / fixture diff / noise changes → human review → merge → sealed. A version inside the accepted semantic envelope is high-consequence Class B + `HUMAN_REVIEW_REQUIRED` + human merge; an empty screenshot diff never classifies semantics as unchanged — review examines ordering, multiplicity, presence, timing, payload, captures, negatives, catalog. New semantic authority (a new source category, a state assertion ADR 0015 does not authorize, a changed ladder, changed identity/assurance) crosses to a decision boundary. `sealed_by` records the human-reviewed sealing evidence, never the automation actor |
| Q23 | (b), with the summary strictly derived from the item/session projection. Items carry id, reason, since, acknowledged; at most one current Needs You/Ready item per session in this model, the array extensible but never historical storage. `attention.summary` = per class `{ total, unacknowledged }`, `0 ≤ unacknowledged ≤ total`; the TUI header shows totals, the badge unacknowledged; clients never recompute totals from a filtered list. `since_ms` rejected as ambiguous: `since_unix_ms` for an instant, `age_ms` for a duration. Invariant: *the summary is a daemon-derived projection of current items, not an independent state* |
| Q24 | Two distinct facts, two copies, neither says "unsupported". Known but unsealed version: "Running · Limited awareness · Claude Code 2.1.258 not yet verified by Corral" with help "Corral has not yet verified attention support for Claude Code 2.1.258." Unbindable version: "Running · Limited awareness · Claude Code version unknown" (or "Provider version could not be verified") with help "Corral could not reliably determine which Claude Code version this session is running." "Unsupported version" is reserved for a real support decision. Principle: *Limited awareness copy describes Corral's evidence limitation, never implies provider incompatibility* |
| Q25 | Defaults accepted (14 days, 30 per provider, newest mtime first, dedupe by `(provider, external_id)`, newest file wins), as query defaults not wire constants. Claude top-level `<uuid>.jsonl` is a candidate; directories and `memory/` are not. Codex `rollout-<timestamp>-<uuid>.jsonl` yields the id from the name. Location rule (a) rejected: the dash encoding is not reversible, and a decoded candidate existing on disk proves nothing. A pure history row shows a location only when Corral owns an exact cwd, the encoding is proven reversible, or a sealed metadata source supplies it; for the ambiguous Claude encoding, omit — "Claude Code · 2h ago · <short id>". Never show the encoded name as a path. Principle: *a hint may be optional; it may not be fabricated because identity does not depend on it* |
| Q26 | (a) with explicit overflow: daily files, 30-day retention, pruned at startup and at day rollover; 16 MiB per-day budget. Exhausting it stops ordinary records for that day, writes an explicit overflow marker (or a sidecar `.incomplete`), warns, and `attention report` marks the interval INCOMPLETE; that day cannot count as a complete evidence day. Early records are never rotated away to stay under the cap. Invariant: *a bounded diagnostic journal may become incomplete; it must never become silently incomplete* |
| Q27 | Approved canonical wording. §5: "No systematic missed states within any provider / version / surface / evidence capability that Corral claims as supported (`PRODUCT.md` §10)." §6 adds the Needs You evidence floor: counts and any A-thesis verdict may count only flows on surfaces with a supported Needs You evidence primitive; a cohort or window without sufficient such flows yields *insufficient evidence*, never a pass; a surface that never reports Needs You cannot contribute "zero false positives" |
| Q28 | (a) with a release-gate qualification: PR8 seals only versions whose attention semantics were measured — Claude 2.1.258 and Codex 0.145.0 for exactly the capabilities proved; no backfill onto 2.1.252. PRODUCT §10's "latest stable + previous tested" is not deleted: PR8 merge proceeds on the measured set; a public release claiming both needs the additional sealed rows (Q22's automation, human-sealed). Weakening §10 would be a separate canonical product decision. Principle: *never invent historical semantic coverage; never silently lower the declared support policy* |
| Q29 | `docs/references/provider-noise-catalog.md`: stable id, provider, observed version/range, surface, phenomenon, evidence/fixture, risk if misinterpreted, disposition (unresolved · suppressed by adapter · excluded by manifest negative · diagnostic-only · not semantic evidence), regression references. Tests may cite ids; runtime code never parses the catalog. Disputes → human triage → catalog entry + deterministic fixture → reseal if needed. Positive semantics ("idle prompt means Ready") belong to the sealed matrix/manifest, not the catalog; the catalog records the confusion ("idle prompt must not be read as Needs You"). Initial entries: Codex title-generation thread/notify, post-Stop SubagentStop, background-task hook after Stop, PTY echo false-activity risk, permission-like strings in ordinary output. Principle: *the matrix says what evidence may mean; the noise catalog records measured ways evidence misleads* |

## Founder rulings, verbatim (round 3)

```text
这轮我会改三处推荐：Q25 的 Claude 路径不能靠"候选存在"就当成可靠 display hint；Q26 的 16 MiB 溢出必须让证据窗口显式变成 incomplete；Q28 允许 PR8 只封当前版本，但不能把 PRODUCT §10 的双版本 release 承诺一起降掉。

其余方向基本照收。

# PR8 grill — round 3 rulings

## Q21 — matrix scenario inventory

裁决：接受现有清单，并补充 transition negatives。

AskUserQuestion 与 ExitPlanMode approval 都属于 Needs You，前提是实测场景确认：

the provider is blocked waiting for an explicit user response before it can continue.

它们的产品语义来自"当前是否阻塞在用户身上"，不是 tool/event 名字本身。

因此 Claude matrix 至少覆盖：

Positive / candidate semantic cases:

* Bash / tool permission prompt
* AskUserQuestion
* ExitPlanMode approval
* Stop → idle prompt / ready surface

Noise / negative / transition cases:

* thinking spinner
* silent long-running tool
* compaction
* `/resume` picker
* API error / retry
* help overlay
* resize/redraw
* user typing at the normal prompt
* long pasted user input
* ordinary tool/output text containing permission-like wording
* user resolves/approves a Needs You prompt and the screen moves on
* user rejects/cancels a Needs You prompt where supported

最后两条很重要：

screen rule 不只要证明：

"I can recognize Needs You"

还要证明：

"I stop claiming Needs You when the visible blocker is gone."

Codex matrix 保留计划中的：

* command approval
* question/input prompt if one exists
* completed-turn idle
* working spinner
* errors
* all observed notify variants
* `tui.notifications` / OSC behavior

并同样加入：

* user typing
* pasted input
* blocker resolution transition
* permission-like text appearing as ordinary output

对于 hook：

capture every observed `Notification.notification_type`,
not merely the ones we currently expect to use.

Also capture ordering/noise involving:

* PermissionRequest
* Stop
* SubagentStop
* background task events

这些 capture 可以最后证明"不值得成为 semantic evidence"，
但 matrix 必须先知道它们存在。

核心不变量：

A semantic fixture must test both entry into the claimed state and
credible near-misses / exit from that state.

## Q22 — matrix expansion workflow and sealing authority

选择 (b)。

Automation may generate evidence and a proposed matrix expansion.

Automation may NOT grant semantic authority to itself.

Recommended flow:

provider/version discovered
→ credentialed verify-release matrix job
→ execute sealed scenario suite
→ capture provider events/screens
→ compare against previous sealed fixtures
→ produce proposed matrix row / fixture diff / noise changes
→ human review
→ merge
→ only then is the new row sealed

A new version row that stays entirely inside already-accepted evidence
semantics is:

high-consequence Class B
+
HUMAN_REVIEW_REQUIRED
+
human merge

例如：

Claude 2.1.259 produces the same already-approved PermissionRequest
semantics and the complete matrix supports that conclusion

→ B + human seal

But automation MUST NOT silently classify a semantic change as
"same because screenshot diff is empty".

Human review examines:

* event ordering
* multiplicity
* absence/presence
* timing semantics
* payload semantics
* screen captures
* negative fixtures
* provider noise catalog

If a new matrix result requires a new architectural semantic permission,
for example:

* a new evidence source category
* a new state assertion not authorized by ADR 0015
* a changed claim ladder
* materially changed identity/assurance semantics

then it crosses the ordinary matrix-expansion envelope and requires the
appropriate new decision/Class C process.

So:

new version inside accepted semantic envelope
→ B + human merge

new semantic authority
→ decision boundary

`sealed_by` records the human-reviewed sealing commit/evidence,
not the automation actor.

核心原则：

Automation gathers and compares evidence.
Humans grant semantic authority.

## Q23 — attention item and summary wire projection

选择 (b)，但 summary 必须严格是 item/session projection 的派生汇总，
不能成为第二套 truth source。

Per session:

attention.items[] entries conceptually contain:

* attention_item_id
* reason
* since
* acknowledged

PR8 当前 active semantic model normally has at most one current
Needs You/Ready attention item for a session, because an item is created
when the session enters that semantic attention state and retires when it
leaves it.

The array shape may remain extensible, but clients must not interpret it
as historical attention-item storage.

`attention.summary`:

needs_you:
total
unacknowledged

ready:
total
unacknowledged

Definitions:

total
= current sessions/items presently projected in that attention state

unacknowledged
= the subset whose current AttentionItem is not acknowledged

Therefore:

0 <= unacknowledged <= total

TUI header may show total:

Needs You 3 · Ready 2

Tray/badge uses unacknowledged counts.

This avoids:

three Needs You rows
but header says two

merely because one was acknowledged.

Clients MUST NOT recompute authoritative totals from a partially loaded
or filtered list.

Daemon owns the summary.

One wire naming correction:

`since_ms` is ambiguous.

If it is an absolute timestamp, name it explicitly, e.g.:

since_unix_ms

If it is an elapsed duration:

age_ms

Do not ship an ambiguous `since_ms` contract.

核心不变量：

Summary is a daemon-derived projection of current attention items,
not an independent attention state.

## Q24 — unsealed / unknown-version wording

方向接受，但 remove "unsupported" from the CTA too.

Known provider/version, newer or otherwise unsealed:

Running · Limited awareness
Claude Code 2.1.258 not yet verified by Corral

CTA/help:

Corral has not yet verified attention support for Claude Code 2.1.258.

Unknown/unbindable runtime version:

Running · Limited awareness
Claude Code version unknown

or:

Running · Limited awareness
Provider version could not be verified

Help:

Corral could not reliably determine which Claude Code version this
session is running.

Do not use the same wording for these two cases.

They mean different facts:

known version + missing matrix row
→ Corral has not tested it yet

unknown version
→ Corral cannot establish the version of this runtime

"Unsupported version" should be reserved for a real support decision,
not merely absent evidence.

核心原则：

Limited awareness copy must describe Corral's evidence limitation,
not falsely imply provider incompatibility.

## Q25 — PR8b history enumeration defaults and location hint

Accept initial enumeration defaults:

* lookback window: 14 days
* maximum: 30 rows per provider
* newest mtime first
* dedupe by `(provider, external_id)`
* duplicate files for same external id → newest mtime wins

These are initial product/query defaults, not wire constants.

Enumeration remains filename/directory-shape based.

Claude:

top-level `<uuid>.jsonl`
→ candidate history entry

top-level directories / `memory/`
→ not history rows under the current sealed enumerator

Codex:

sealed `rollout-<timestamp>-<uuid>.jsonl`
→ external id may be obtained from the filename without transcript parsing

But reject proposed location rule (a) for an ambiguous Claude encoding.

The encoding:

`/Users/foo-bar/project`
→ dash-based directory representation

is not reversible when literal `-` and `/` share the same representation.

"The decoded candidate happens to exist on disk"
does NOT prove that it was the original path.

Two different original paths can map to the same encoded directory name,
and both possible filesystem paths may exist.

Therefore pure history rows may display location only when Corral has
evidence sufficient for that display claim, for example:

1. Corral already owns an exact cwd for this Session; or
2. the provider's location encoding is proven reversible; or
3. another sealed provider metadata source directly supplies the path.

For the currently ambiguous Claude project-directory encoding:

pure provider-history row
→ omit cwd/location

rather than display a guessed path.

So pure history fallback may be:

Claude Code · 2h ago · <short id>

while Corral-known sessions may still show:

Claude Code · ~/work/demo · 2h ago

Do not display the encoded directory name as though it were a filesystem
path.

This is still consistent with ADR 0016:

location is display-only and never identity input.

But "display-only" does not grant permission to knowingly show ambiguous
facts.

核心原则：

A hint may be optional.
It may not be fabricated merely because identity does not depend on it.

## Q26 — attention journal retention and rotation

方向选择 (a)，with explicit overflow semantics.

Use daily files:

attention-journal-YYYY-MM-DD.jsonl

Retention:

30 days

Prune:

* daemon startup
* and when rolling over to a new journal day

so a daemon that remains alive for weeks does not indefinitely retain
files merely because it did not restart.

Initial per-day safety budget:

16 MiB

But "16 MiB hard cap" must not silently drop diagnostic evidence.

If the daily budget is exhausted:

→ stop accepting ordinary journal records for that day
→ create/retain an explicit diagnostic overflow marker
→ emit operational warning/tracing
→ `attention report` marks that date/evidence interval as INCOMPLETE

The affected interval cannot be counted as a complete dogfood evidence day
for claims requiring full attention-transition accounting.

Do not silently rotate away early records merely to remain under the cap.

The important property for the 14-day evidence window is:

we know whether the evidence is complete.

An implementation may reserve enough space for its final overflow marker
or use a tiny sidecar marker such as:

attention-journal-YYYY-MM-DD.incomplete

The journal remains:

* diagnostic
* deletable
* non-authoritative
* never fed back into attention inference
* no raw prompt/screen/provider payload by default

30 days gives enough margin around the 14-day dogfood window without
turning diagnostics into permanent history.

核心不变量：

A bounded diagnostic journal may become incomplete;
it must never become silently incomplete.

## Q27 — ROADMAP §5 / §6 canonical wording

批准，但我会稍微收紧英文。

ROADMAP §5 replace with:

"No systematic missed states within any provider / version / surface /
evidence capability that Corral claims as supported (`PRODUCT.md` §10)."

ROADMAP §6 add:

"Needs You evidence floor: trusted Needs You transition counts and any
A-thesis verdict may count only flows on surfaces with a supported
Needs You evidence primitive. A cohort or evidence window without
sufficient such flows yields *insufficient evidence*, never a pass.
A surface that never reports Needs You cannot contribute 'zero false
positives' toward validating Needs You fidelity."

This is the approved Class C canonical wording.

The point is not merely statistical.

It prevents a capability-blind interpretation such as:

external Codex produced zero false Needs You alerts

when the reason was:

external Codex could not detect Needs You at all.

## Q28 — which provider versions PR8 must seal

选择 (a)，with a release-gate qualification.

PR8 may seal only versions for which PR8 attention semantics were actually
measured.

Do NOT backfill semantic authority onto Claude 2.1.252 merely because
earlier PRs tested unrelated attachment/hook facts.

For the current PR8 work:

Claude Code 2.1.258
→ may be sealed if Q21 matrix evidence passes

Codex 0.145.0
→ may be sealed for exactly the attention capabilities the matrix proves

Previous versions with no PR8 semantic matrix:

→ unsealed
→ no inherited Attested attention semantics

However PRODUCT §10's public support goal:

latest stable + previous tested

is not automatically deleted by this ruling.

Distinguish:

PR8 merge requirement
from
M1 public release/support requirement.

PR8 merge:

may proceed with the actually measured version set.

Before public release claiming:

latest stable + previous tested

the release matrix must contain the required additional sealed version
rows.

Q22 automation can make this cheap,
but automation output still requires human sealing.

If the product team chooses to weaken PRODUCT §10 instead,
that is a separate canonical product decision;
PR8 implementation must not silently do it.

核心原则：

Never invent historical semantic coverage,
but do not silently lower the declared release support policy either.

## Q29 — provider noise catalog

接受 a repository entity at:

docs/references/provider-noise-catalog.md

It records measured provider/runtime phenomena that matter to evidence
interpretation but are not themselves runtime configuration.

Each entry should minimally contain:

* stable noise id
* provider
* version / version range actually observed
* surface
* observed phenomenon
* supporting evidence/fixture
* risk if misinterpreted
* current disposition
* regression fixture/rule references where applicable

Disposition should distinguish at least:

* unresolved
* suppressed by adapter
* excluded by manifest negative rule
* diagnostic-only
* determined not to be semantic evidence

Manifest/adapter tests may cite catalog ids.

Runtime code MUST NOT parse this Markdown catalog as configuration.

Journal disputes:

diagnostic dispute
→ human triage
→ if it reveals a reusable provider-noise class
→ add/update catalog entry
→ add deterministic fixture
→ adjust/reseal rule if necessary

One taxonomy correction:

Do not put a positive semantic rule into the noise catalog merely because
it was discovered while investigating noise.

For example:

"Claude idle prompt means Ready"

belongs in the sealed semantic matrix/manifest evidence.

The noise catalog may instead record:

"idle prompt must not be confused with Needs You"

with the relevant negative fixture.

Good initial noise entries include:

* Codex internal title-generation thread/notify
* post-Stop SubagentStop ordering/noise
* background-task hook after main Stop
* PTY input echo causing activity/screen false-positive risk
* permission/question-like strings appearing as ordinary terminal output

核心原则：

The matrix says what evidence is allowed to mean.
The noise catalog records measured ways that evidence can mislead us.

这一轮我觉得 Q25 是最容易"因为只是 display hint 就放松真实性"的地方。`-Users-hzhou-corral` 的问题不是"可能路径不存在"，而是编码本身不可逆；即使 `/Users/hzhou/corral` 存在，也不能证明原路径不是 `/Users/hzhou-corral`。既然 M1 已经接受历史行无标题，同样应该接受"纯历史 Claude 行可能无 cwd"。

而 Q28 要把两个时钟分清：PR8 没必要为了"previous tested"倒装旧版补语义测试才能合并，但 M1 真正对外宣称 `latest stable + previous tested` 前，release matrix 还是得兑现这个承诺。
```

# Round 4

Digest of what round 4 froze:

| Q | Ruling |
|---|---|
| Q30 | (a) — `AttentionReason` stays `NeedsInput` / `TurnComplete` (wire `needs_input` / `turn_complete`); `RuntimeEnded` reserved, unproduced. Permission prompt, AskUserQuestion, and ExitPlanMode all produce `NeedsInput`; the difference lives in the existing `NeedsInputContext` as display context, which must not pre-empt M2's structured request contract (no Allow/Deny/Answer, tool schema, or response payload in the reason). Invariant: *AttentionReason classifies why attention is needed; it does not predefine the shape of the response* |
| Q31 | Six conditions accepted: PR8a merged; a human advanced `STORAGE_EPOCH` to dogfood; the exercised provider/version/capability rows sealed; diagnostics functioning and the counted interval not INCOMPLETE; Linux external Know excluded until the Q16 artifact; PR8b not required. And two evidence questions kept apart: a managed-only window can accumulate C (attention-fidelity) evidence — false positives, staleness, acknowledgement, re-arming, synthesis — but cannot validate A (observed aggregation); trusted Needs You counts contribute to an A verdict only from surfaces meeting the evidence floor. Every report names which question a window supports: C, A, or both. Principle: *a dogfood interval may be complete evidence for one claim and insufficient for another* |
| Q32 | (b) — early branch work allowed for components fixed by rulings and independent of unmeasured provider behavior: core vocabulary, the pure engine and freshness mechanics, item identity, synthetic-evidence tests, acknowledge-by-item, the journal, grill-decided protocol structures, version-evidence plumbing, generic manifest schema/validation. Must wait for the matrix: event → state mappings, Needs You / Ready screen rules, sealed version rows, capture-dependent noise suppression, any adapter behavior claiming measured semantics. Nothing merges while the ADRs are proposed. Acceptance closes when: the matrix artifact exists; every Q21 scenario has a capture or an explicit measured "not present"; the initial noise catalog exists; every load-bearing fact is measured, covered by an earlier accepted invariant, or explicitly marked a non-load-bearing limitation; no semantic-capable rule exists merely because code preceded evidence. Principle: *code may materialize decided mechanics early; it may not manufacture empirical semantic authority ahead of evidence* |
| Q33 | Copy accepted with tightening. History row, no known live Run: "Corral can't tell whether this session is still running somewhere else. Continuing here starts another Claude Code process for this session." [Continue] [Cancel], provider name dynamic, "for this session" not "on the same conversation"; the disclosure means possible concurrent continuation, not that another process is known. Discovered live: "Still running outside Corral. Continuation is unavailable while this session remains live.", no action. Managed live: ordinary Running/Open. Managed Unverifiable: "Corral couldn't verify that the previous process ended, so continuation is unavailable." — never "may still be running" without evidence. All under `disclosure_revision`: correlation, not consent |
| Q34 | Verbs accepted — `corral needs`, `corral ack <session>`, `corral attention dispute <session>`, `corral attention report [--since]`, `corral continue --yes` — bound to the frozen identities. `ack` resolves the session's current acknowledgeable item, sends its exact `attention_item_id`, tolerates `StaleAttentionItem` without acknowledging a replacement, and reports no-current-attention rather than acknowledging a future item. `dispute` records the exact current/recent item id, noting when it was already stale. `report` reads the journal only and reports INCOMPLETE intervals, transition totals, trusted Needs You totals, and known disputes, never treating INCOMPLETE as zero. `--yes` skips only the CLI's own confirmation: preflight runs, the disclosure renders, `session.resume` carries the revision, the daemon recomputes and validates, a stale revision is rejected even with `--yes`. DoD additions accepted: PR8a — noise catalog exists and every semantic-capable rule cites its sealed evidence and noise fixtures; a delayed ack cannot acknowledge a replacement; successful Open acks Ready and failed Open does not; overflow makes the report INCOMPLETE. PR8b — no inferred cwd on pure Claude rows; a Corral-known Session shows its exact cwd under snapshot; continuation covers the historical disclosure, external-live refusal, managed-live Open, Unverifiable refusal, stale-revision rejection, and `--yes` still preflighting. Principle: *CLI convenience may simplify interaction; it may not weaken item identity, disclosure correlation, or daemon-owned truth* |

## Founder rulings, verbatim (round 4)

```text
这轮基本可以收口。我会裁：

* Q30 = (a)
* Q31 = 六条接受，但明确 C 窗口与 A-thesis 窗口不是同一个有效性条件
* Q32 = (b)
* Q33 = 接受，微调两句文案
* Q34 = 动词接受，但 `ack` / `dispute` 必须落到具体 `AttentionItemId`，`continue --yes` 仍必须走 `disclosure_revision` preflight

有两个地方不能只"照写"：Q34 必须服从上一轮已经冻结的 item identity 与 disclosure revision，否则 CLI 会重新打开刚堵上的 race。

# PR8 grill — round 4 rulings

## Q30 — `AttentionReason` wire vocabulary

选择 (a)。

PR8 保持：

NeedsInput
TurnComplete

对应 wire：

needs_input
turn_complete

`RuntimeEnded` 保留在现有内部/预留词汇中，
但 PR8 不产生对应 AttentionItem。

不要现在把 NeedsInput 拆成：

needs_approval
needs_answer

原因：

AttentionReason 回答的是：

Why does this session currently need the user's attention?

而不是：

What exact structured response can the user send?

在 PR8：

permission prompt
AskUserQuestion
ExitPlanMode approval

都可以产生：

AttentionReason::NeedsInput

差异可以存在于当前已有的：

NeedsInputContext

中，作为 display/context information。

但 context 不得提前承担 M2 structured request contract。

尤其不要把：

Allow / Deny / Answer
tool schema
provider-specific response payload

塞进 AttentionReason。

未来真正支持 centralized Respond 时：

NeedsInputRequest

拥有：

* request kind
* response choices/schema
* delivery semantics
* request identity

届时是否细分 request kind 由那个阶段决定。

核心不变量：

AttentionReason classifies why attention is needed.
It does not predefine the shape of the response.

## Q31 — dogfood evidence window start

接受六个条件，但区分两个 evidence questions。

PR8 attention-fidelity dogfood window may begin only when:

1. PR8a is merged.

2. A human has explicitly advanced:

   STORAGE_EPOCH = dogfood

3. the actually exercised provider/version/capability rows are sealed.

   Initial examples may include:

   * Claude Code 2.1.258
   * Codex 0.145.0

   only for capabilities their matrices actually prove.

4. attention diagnostics are functioning and the counted day/interval is
   not marked INCOMPLETE.

5. Linux external Know observations do not count until the real-Linux
   evidence artifact required by Q16 exists.

6. PR8b is NOT required to begin PR8a attention-fidelity dogfood.

But distinguish:

C-quality evidence window

from:

A-thesis validation window.

C / attention-fidelity evidence can be accumulated from supported managed
flows.

For example, managed Claude Needs You transitions can test:

* false positive rate
* stale-state behavior
* acknowledgement behavior
* notification re-arming
* screen/hook synthesis

A / Observed-session thesis requires qualified observed/external flows.

Therefore:

14 days of excellent managed-only PR8a dogfood

may validate important attention-fidelity behavior,

but it cannot by itself validate:

"Observed aggregation is the product bet."

Likewise the trusted Needs You count contributes to an A verdict only where
the counted surfaces satisfy the accepted Needs You evidence floor.

So reports must identify which evidence question a window supports:

* attention fidelity / C
* observed aggregation / A
* both

Never silently reuse a managed-only window as proof of A.

Core principle:

A dogfood interval may be complete evidence for one claim and insufficient
evidence for another.

## Q32 — implementation before matrix completion

选择 (b)。

Implementation may begin on a branch before ADR acceptance for components
whose semantics are already fixed by founder rulings and do not depend on
unmeasured provider behavior.

Allowed early work includes:

* core attention vocabulary
* pure attention-engine state machine
* freshness mechanics
* AttentionItem identity
* deterministic synthesis tests using synthetic evidence
* acknowledgement-by-item semantics
* diagnostic journal
* protocol structures already decided by grill
* provider-version evidence plumbing
* generic manifest schema/validation machinery that does not grant any
  unmeasured rule semantic authority

Must wait for matrix evidence:

* provider event → semantic-state mappings
* screen rules that assert Needs You / Ready
* sealed version rows
* provider-specific noise suppression whose correctness depends on captures
* any adapter behavior that claims measured provider semantics

Nothing from either category merges while ADR 0015 / 0016 remain proposed.

ADR acceptance closing conditions:

1. matrix evidence reference/evidence artifact exists;

2. every required Q21 scenario has either:

   * an actual capture/result, or
   * an explicit measured "not present / unsupported" result;

3. initial provider noise catalog exists;

4. every load-bearing fact in ADR 0015 / 0016 is:

   * supported by measured evidence,
   * supported by an earlier accepted invariant,
   * or explicitly marked as an unresolved non-load-bearing limitation;

5. no semantic-capable manifest rule exists merely because implementation
   was written before the matrix.

Then:

proposed → accepted

and implementation may cross the matrix-sensitive boundary.

Core principle:

Code may materialize already-decided mechanics early.
It may not manufacture empirical semantic authority ahead of evidence.

## Q33 — PR8b user-facing continuation copy

方向接受，with minor copy tightening.

### Pure history row / no known live Run

Use provider name dynamically.

Recommended:

"Corral can't tell whether this session is still running somewhere else.
Continuing here starts another Claude Code process for this session."

[Continue] [Cancel]

I prefer:

"for this session"

over:

"on the same conversation"

because the durable fact Corral owns is provider session identity;
"conversation" is provider/product language that may not generalize cleanly.

For Codex:

"Corral can't tell whether this session is still running somewhere else.
Continuing here starts another Codex process for this session."

The disclosure means:

possible concurrent continuation/fork

not:

Corral knows another process currently exists.

### Discovered external session still live

Approved:

"Still running outside Corral. Continuation is unavailable while this
session remains live."

No Continue action.

### Managed session still live

No continuation disclosure.

Show normal Running/Open behavior.

### Managed Run ended Unverifiable

Tighten to:

"Corral couldn't verify that the previous process ended, so continuation
is unavailable."

This is slightly more precise than:

"this session ended"

because the whole point of Unverifiable is that Corral does NOT know
whether the process ended.

Do not say:

"the session may still be running"

unless current evidence actually supports that stronger claim.

All continuation disclosures remain subject to Q18/QADR 0016:

`disclosure_revision` correlates the client's displayed disclosure with
the current daemon decision.

It does not represent consent in the protocol.

## Q34 — CLI verbs and workstream DoD

CLI surface accepted:

corral needs
corral ack <session>
corral attention dispute <session>
corral attention report [--since ...]
corral continue ... --yes

But these commands must honor the already-frozen protocol identities.

### `corral needs`

Shows current Needs You sessions/items.

It is a projection of daemon truth,
not a second client-side attention engine.

### `corral ack <session>`

CLI may expose a session-oriented UX,

but internally it MUST:

1. fetch/resolve the current acknowledgeable AttentionItem for that session;

2. obtain its exact `attention_item_id`;

3. send:

   attention.acknowledge(session_id, attention_item_id)

4. tolerate `StaleAttentionItem` without acknowledging a replacement item.

Therefore the CLI syntax does NOT weaken Q18's wire invariant.

If no acknowledgeable current item exists:

→ explicit no-current-attention error/no-op

rather than acknowledging a future item.

### `corral attention dispute <session>`

Same identity discipline.

The dispute record should include the exact current/recent
AttentionItemId when one exists.

Do not journal merely:

"user disputed session X"

if a precise item can be identified.

This matters when:

item A resolves
→ item B appears shortly afterward
→ user disputes A

The diagnostic must not accidentally attribute the dispute to B.

Because the journal is diagnostic,
a dispute may additionally record that the referenced item was already
stale by the time the command arrived.

### `corral attention report [--since ...]`

Reads diagnostic journal only.

It must clearly report:

* incomplete dates/intervals
* transition totals
* trusted Needs You totals
* disputes/false-positive classifications where known

and must not silently treat INCOMPLETE intervals as zero events.

### `corral continue --yes`

`--yes` is CLI UX only.

It MUST NOT bypass continuation preflight.

Flow:

session.continuation
→ daemon returns current decision + disclosure + disclosure_revision

CLI:
→ renders disclosure normally

without `--yes`:
→ asks interactive confirmation where applicable

with `--yes`:
→ skips the interactive confirmation step

then:

session.resume(..., disclosure_revision)

Daemon still recomputes continuation eligibility and validates the
revision.

Therefore:

`--yes`
≠
wire `disclosed=true`
≠
authorization bypass

It simply means:

the CLI user requested non-interactive acceptance of the currently
returned disclosure.

If the revision is stale:

→ resume rejected
→ fresh preflight required

even with `--yes`.

### PR8a DoD addition

Accept:

initial provider noise catalog exists and all semantic-capable screen /
adapter rules reference the relevant sealed evidence/noise fixtures.

Also require:

* delayed/stale `corral ack` cannot acknowledge a replacement item;
* successful Open acknowledges Ready;
* failed Open does not;
* journal overflow makes report explicitly INCOMPLETE.

### PR8b DoD addition

Accept:

pure Claude history rows do not display an inferred cwd from the ambiguous
project-directory encoding.

Also require a positive counterpart:

A Corral-known Session with an exact trusted cwd displays that exact cwd,
covered by deterministic presentation snapshot/fixture.

And continuation DoD must cover:

* pure historical unknown-live-state disclosure;
* external-live refusal;
* managed-live Open;
* Unverifiable refusal;
* stale `disclosure_revision` rejection;
* `--yes` still performs a current continuation preflight.

Core principle:

CLI convenience may simplify interaction.
It may not weaken item identity, disclosure correlation, or daemon-owned
truth.

有两处值得现在锁死。

第一，Q31 不要说"dogfood window"时只剩一个模糊的窗口。PR8a 合并后，你完全可以开始积累 C 的 attention fidelity 数据；但如果全是 managed Claude，那个窗口不能顺手变成 A-thesis 的 Observed aggregation 证据。报告里最好从第一天就标明它在验证 C、A、还是两者。

第二，Q34 的 `corral ack <session>` 只是 UX 语法糖。协议层仍必须是：

`session + exact attention_item_id`

同理 `continue --yes` 也只是 CLI 省掉确认问题，不得重新退回前面已经否掉的裸 `disclosed: bool`。

这轮之后结构性 grill 基本可以结束。下一步应该先跑 Q21 matrix；回来以后只需要做一次 acceptance reconciliation：检查 evidence 是否足以把 ADR 0015/0016 从 `proposed` 翻成 `accepted`，不再重新 grill 这些已经裁完的结构问题。
```

# Round 5 — acceptance reconciliation (proposed by the agent, pending the founder)

The matrix ran 2026-09-02 (`docs/references/2026-09-02-pr8-attention-matrix.md`).
Checked against Q32's closing conditions:

| Q32 condition | Finding |
|---|---|
| 1. Matrix evidence artifact exists | Yes — the reference above; captures under `crates/corrald/fixtures/screens/`; rendered by `replay_capture` |
| 2. Every Q21 scenario has a capture or a measured "not present" | Every positive surface captured (Claude: tool permission, AskUserQuestion, ExitPlanMode, idle prompt, trust dialog; Codex: command approval). Measured absent: a Codex question surface; OSC 9/777 under `tui.notifications`. **Not induced**: Claude API error/retry and compaction; Codex `/` popup and compaction — four negatives, no positive |
| 3. Initial provider noise catalog exists | Yes — 15 entries, each with its fixture |
| 4. Every load-bearing fact measured, covered by an accepted invariant, or marked a non-load-bearing limitation | ADR 0015: all measured. ADR 0016: measured, with four items explicitly marked non-load-bearing (resume touching mtime, headless Codex rollouts by name, `--resume` across directories, enumeration cost) |
| 5. No semantic-capable rule exists merely because code preceded evidence | No engine, adapter, or manifest code exists yet |

Proposed ruling: **accept ADR 0015 and ADR 0016**, with the four
un-induced negatives recorded as sealing prerequisites — a Needs You or
Ready rule that a compaction, error, or command-popup screen could fool is
not sealed until that negative is captured (Q14's "all currently captured
other states" widens as they land) — and grill Q16's Linux gate unchanged.
Nothing measured contradicts a load-bearing assumption of either ADR; two
measurements strengthen them: Codex's rollout-file identity discriminator
(ADR 0016 D1's enumeration reaches the identity PR7 could not), and the
title-thread notify arriving before the user turn completes (ADR 0015
D3's identity axis is what stops a false Ready).

## What this leaves open

Nothing structural. The Q21 matrix ran 2026-09-02 on Claude Code 2.1.258
and Codex 0.152.0 (`docs/references/2026-09-02-pr8-attention-matrix.md`);
next is one acceptance reconciliation against Q32's closing conditions,
flipping ADR 0015/0016 `proposed → accepted` or recording which
load-bearing fact failed and reopening exactly that ruling. Concrete
sealed rules, sealed version rows, and noise-catalog entries are matrix
evidence, not grill questions.

**Superseded by round 5**, which split the two ADRs: 0016 is accepted, 0015
stays proposed against nine named conditions.

## Q35 — the directory a history row continues in (opened by the matrix, 2026-09-02)

Opened after the structural grill closed, because a measurement made it
askable: both providers resume a session id from any directory and carry
on *in the new one* (matrix record, "Resume from a directory other than
the session's"). A history row carries no location (Q25), so the
directory is Corral's choice, and D5's disclosure has to name it.
`history::resume_location_sealed` stays false for both providers until
this is ruled; a history row's continuation is refused until then.

❓ **Q35** - **Which directory does Corral continue a history row in, and who says?**

(a) The client's own working directory: `corral continue` uses the shell's
cwd, the TUI its process cwd, and `session.resume` gains an optional
`working_directory` that only a history-row continuation may carry
(refused with `invalid_params` for a Session Corral already knows). The
disclosure names it: "…Continuing here starts another Claude Code process
for this session in `/Users/me/proj`."

(b) The person names it every time: `corral continue --in <dir>` is
required for a history row, and the TUI prompts for a path.

(c) The daemon's account home, or the provider's default, silently.

➡️ **(a)**, with the directory always in the disclosure and never
defaulted when the client sends none (refused, not guessed). It is what
the provider itself does — resume runs wherever you run it — so it never
surprises a person who knows the tool; the cwd is the one fact the
launching client holds authoritatively; and (b) is available on top of it
as `--in <dir>` later if the default proves wrong. (c) decides for the
person and hides it.

---

# PR8 grill — round 5 rulings

> Founder, 2026-09-03. Recorded verbatim. Split deliberately: Q35 is decided,
> ADR 0016 may close, ADR 0015 may not, and the e2e binary footgun becomes a
> merge gate.

这里我会分开裁：**Q35 现在定；ADR 0016 可以独立收口；ADR 0015 还不能翻 accepted。** 另外你碰到的 e2e 二进制覆盖问题，我会从 follow-up 升为 PR8 merge gate。

## Q35 — historical continuation working directory

裁决：使用发起客户端明确提供的 working directory。

对于 pure history row：

client working directory
→ becomes the requested continuation working directory

daemon MUST NOT fall back to:

* daemon cwd
* inferred Claude project-directory path
* provider history file location
* home directory
* last guessed location

If the client does not provide a working directory:

→ continuation refused

rather than silently choosing one.

This is especially important because the measured provider behavior shows
that resume identity and working directory are independent:

Claude Code 2.1.258:

* resume from another cwd succeeds
* provider session id remains unchanged
* original session history continues
* resumed process adopts the new cwd

Codex 0.152.0:

* resume from another cwd succeeds
* thread id remains unchanged
* same rollout continues
* the new turn records the new cwd

Therefore the cwd is not recoverable merely from provider session identity.

Corral must make the choice explicit.

### Wire shape

`session.continuation` request must include the proposed working directory
when the continuation requires spawning a new process.

Conceptually:

session.continuation {
session_id,
working_directory
}

The response disclosure and `disclosure_revision` are calculated from,
and therefore bound to:

* session identity
* current live/run facts
* continuation decision
* provider
* requested working directory
* other facts already used by the continuation ladder

`session.resume` must carry:

* session_id
* working_directory
* disclosure_revision

The daemon recomputes the decision.

Changing the working directory after preflight:

→ disclosure revision mismatch
→ fresh preflight required

Do not make `disclosure_revision` a server-side temporary lease that
requires remembering the preflight request.

It remains a correlation over the decision facts.

### Client policy

For PR8b:

CLI:
→ default requested cwd = the CLI process's current cwd

TUI:
→ default requested cwd = the cwd from which that TUI client was launched

The client sends it explicitly.

This is client policy, not daemon ambient-state fallback.

A future directory picker may replace that client default without changing
the continuation semantics.

For a Corral-known Session with an exact trusted cwd, clients may display
that location as context, but a pure historical continuation must not
silently substitute it for the initiating client's requested cwd unless a
future UX explicitly defines that behavior.

### Disclosure

For a pure historical row:

“Corral can't tell whether this session is still running somewhere else.
Continuing starts another Claude Code process for this session in
`<working directory>`.”

Provider name is dynamic.

The important facts disclosed are:

1. liveness elsewhere is unknown;
2. another provider process will be started;
3. the exact directory in which Corral will start it.

`--yes` still means only:

skip interactive CLI confirmation for this current disclosure

and still performs:

preflight
→ render disclosure
→ resume with matching disclosure_revision

### Validation

Before eligible preflight/spawn:

working directory must be:

* explicitly supplied
* representable as the platform path type
* currently existent
* a directory

The spawn path must revalidate the directory sufficiently to avoid turning
a stale preflight into silent fallback.

If it disappears or becomes unusable:

→ spawn/continuation failure
→ never substitute another directory

Core invariant:

A historical provider identity tells Corral what to resume.
It does not tell Corral where the resumed process should run.

## ADR acceptance reconciliation

### ADR 0016 — history / continuation

Founder decision frontier is now closed.

Q35 resolves the remaining working-directory policy.

ADR 0016 may move:

proposed → accepted

once the reported history/resume measurements are committed as durable
evidence with:

* exact provider version
* exact command/fixture shape
* observed session/thread identity behavior
* observed cwd behavior
* history-file behavior

The current evidence must remain version-specific.

In particular:

Codex 0.152.0 resume/cwd evidence
does NOT automatically seal Codex 0.145.0 for that same fact.

No further founder grill is required for ADR 0016 unless committing the
evidence reveals a contradiction.

### ADR 0015 — attention

DO NOT move to accepted yet.

Keep:

status: proposed

Reason:

The previously accepted closing condition was empirical, not merely
structural.

The current update reports implementation work and resume/history
measurements, but it does not establish completion of the full Q21
attention matrix.

ADR 0015 may move to accepted only after reconciliation confirms all of:

1. the attention matrix evidence artifact exists;

2. every required Q21 scenario has:

   * capture/result, or
   * explicit measured unsupported/not-present result;

3. current provider/version rows are explicit;

4. every semantic-capable event/screen rule is sealed by human-reviewed
   evidence;

5. `sealed_by` points to the actual reviewed evidence/commit;

6. Claude `Notification` variants have been enumerated and classified,
   with unknown/unsealed variants remaining diagnostic only;

7. initial provider noise catalog exists and relevant fixtures cite it;

8. each ADR 0015 load-bearing fact points to:

   * measured evidence,
   * an earlier accepted invariant,
   * or an explicitly non-load-bearing unresolved limitation;

9. glossary / ARCHITECTURE / PRODUCT wording has been reconciled with:

   * received Attested hook evidence may directly support its positive
     claim;
   * assurance is claim-scoped;
   * capability-scoped support/release semantics;
   * unverified versions mean Limited awareness, not inherited authority.

Once those are true:

→ acceptance reconciliation only
→ proposed → accepted

No reopening of Q1–Q34.

## PR8 implementation status before ADR 0015 acceptance

The work you described remains allowed under Q32.

In particular these can exist on the branch before acceptance:

* activity → Working mechanics
* pure attention engine
* journal
* protocol/item identity
* echo discount mechanics
* history enumerator
* continuation preflight framework
* disclosure revision machinery

provided unsealed provider-specific semantics remain incapable of granting
semantic authority.

So PR8b being developed on top of PR8a is fine.

Neither PR may merge across an unaccepted load-bearing ADR boundary.

## E2E binary contamination footgun

Promote this from Follow-up to:

PR8 MERGE GATE

Reason:

The test harness allowed a concurrently rebuilt production `corral`
binary to be used by attention e2e tests.

That caused tests intended to operate in isolated test state to reach the
real user namespace and start a daemon using `~/.corral`.

The observed damage was small:

* log writes only
* no registry mutation
* no Session creation

but the invariant violation is not small.

A test suite must not depend on:

“nobody happens to rebuild this binary while verify is running.”

Before PR8a or PR8b merge, test support must ensure:

1. both `corral` and `corrald` binaries used by e2e are validated as the
   intended test-support build;

2. a wrong/non-test binary causes the harness to fail BEFORE starting the
   process;

3. e2e execution cannot silently fall back to the user's canonical Corral
   endpoint/state paths;

4. concurrent ordinary `cargo build -p corral` cannot redirect an
   in-progress test to a production binary;

5. there is a permanent regression test for the wrong-binary case.

The exact implementation may use:

* immutable copied test binaries,
* build-specific paths,
* a test-only build marker/handshake,
* or an equivalent fail-closed mechanism.

Do not change production daemon identity/rendezvous semantics merely to fix
the test harness.

This is not a new Class C product decision.

It is a correctness/isolation defect in verification infrastructure.

Core invariant:

A Corral e2e test must fail rather than touch the user's real Corral state
when its expected test binaries or isolation contract are not present.

所以当前状态我会记成：

**ADR 0016：Q35 后可以收 accepted。ADR 0015：继续 proposed，等 Q21 matrix evidence 回来做最后一次 reconciliation。**

另外，`task/pr8b-history` 现在继续开发没问题，但我不会让它先于 `task/pr8-attention` 合并；而那个 test-support footgun 要在两条 PR 任何一条 merge 前修掉。它已经证明 `verify` 的隔离正确性存在实际竞态，不应留到后续。

## What round 5 leaves open

ADR 0015 only: the Q21 acceptance reconciliation against the nine conditions
above. ADR 0016 is closed. Q35's answer is implemented on
`task/pr8b-history`; the store operation and spawn that a history-row
continuation still needs are ordinary PR8b work under it.
