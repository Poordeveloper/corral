# PR9 Spike Grill — rulings over the GPUI integration spike record (round 1)

> Status: **round 1 closed** (2026-09-04); round 2 (sequencing and the
> PR9 plan boundary) pending. The record under grill is
> `docs/references/2026-09-04-pr9-gpui-integration-spike.md` (merged as
> PR #40). Round 1 rules the decisions the spike asked for: method
> deviations, the gpui pin, the client replica engine, the dependency
> policy gpui forces, the severity and shape of the six daemon defects the
> spike found, the protocol semantics they expose, and the deferred
> measurements. No ADR is accepted here; Q7 commissions one.
> Governing principle, ruled in Q1:
>
> A methodological deviation is acceptable when its limitation is explicit
> and the load-bearing conclusion is independently reproduced through the
> real execution path.

Questions asked by the grill session of 2026-09-04; rulings verbatim below.

# Round 1

Digest of what round 1 froze:

| Q | Ruling |
|---|---|
| Q1 | All three method deviations accepted (Metal toolchain stays installed; a real corrald under an explicit test root; self-driven `Window::draw` under a locked screen). Evidence is layered: self-driven numbers are diagnostic/comparative; only real display-link numbers carry a performance claim. The frozen e2e isolation invariant is not weakened. |
| Q2 | `gpui = "=0.2.2"` from crates.io; no git rev, no floating range. Upgrades are explicit and re-check render behaviour, cargo-deny delta, platform behaviour, and PR9 evidence. The ledger's §8 row is updated as a reference fact; the architecture decision is not reopened. |
| Q3 | qwertty-term-vt is the Desktop replica engine. The client owns a poison boundary: a parser panic destroys the replica, never the Desktop process or the session truth; rebuild only from a complete fresh snapshot; a bounded retry — no infinite resync loop — with a bounded-loop test. |
| Q4 | `deny.toml`: (i) graph targets macOS + Linux (Windows must widen it before becoming a supported target); (ii) allow BSD-2-Clause, BSD-3-Clause, ISC, MPL-2.0 (deliberately, as file-level weak copyleft), CC0-1.0, plus the `ring` and `encoding_rs` expressions; (iii) six unmaintained advisories ignored as itemised debt records with exit conditions; (iv) duplicates stay warn. Its own governance PR, human-reviewed and human-merged, before the PR9 plan. One repository keeps one merge-ready dependency policy. |
| Q5 | S6 (channel close under sustained output) is **P1** and precedes PR9 implementation. Semantics A + C accepted; a `send().await` on the authoritative broadcast path is rejected: the PTY reader / authoritative VT / publish step never blocks on a subscriber. Per-subscriber writer awaits with a 2 s no-progress deadline (operational policy, not a wire guarantee); a subscriber past the 4 MiB budget enters the resync path behind an explicit barrier; a subscriber making no progress for 2 s is dropped. Four regressions required. |
| Q6 | S1/S2: **high-consequence Class B**, not A. Corral's snapshot render adapter compensates after the formatter (viewport row completion, final cursor position, cursor visibility) to meet the ADR 0003 contract; the upstream patch is a follow-up, never a correctness dependency. The spike fidelity harness becomes a permanent regression test covering the 22 scenarios; Q6 fixes only the accepted snapshot contract and claims nothing Q7 has not ruled. |
| Q7 | New ADR (not an edit of ADR 0003, not plan prose): terminal snapshot geometry, dual-screen state, and palette transport, with compatibility/capability, old-client, resync and epoch behaviour ruled. Compatibility-facing Class C; no wire semantic change merges before acceptance. |
| Q8 | Release-profile numbers are PR9 DoD evidence (same harness, release profile). Linux rendering is not a PR9-plan blocker but is a support/release entry gate before any Linux Desktop claim. GPU present is not split out unless a concrete bottleneck points at that boundary. |

## Questions as asked (abridged)

- **Q1** — accept the three method deviations: Metal Toolchain download,
  a real corrald under `CORRAL_TEST_ROOT=/tmp/pr9`, self-driven frames
  under a locked screen then reproduced unlocked?
- **Q2** — gpui pin: exact crates.io version, git rev, or floating 0.2.x?
- **Q3** — replica engine: qwertty-term-vt or alacritty_terminal; and a
  `Poisoned`-style isolation in the client?
- **Q4** — `deny.toml`: target scoping, allow-list additions, advisory
  ignores, duplicates; or a separate workspace to dodge the gate?
- **Q5** — S6 severity and fix shape: bounded await, larger queue, or
  fold overflow into the existing per-viewer resync path?
- **Q6** — S1/S2 fix in corrald's `render` versus an upstream patch; the
  fidelity harness as a regression test?
- **Q7** — S3/S4/S5 protocol semantics: amend ADR 0003, a new ADR, or
  plan prose only?
- **Q8** — the unmeasured items: Linux rendering, release profile, GPU
  present.

Facts supplied to the round by the grill (not decisions): scoping
cargo-deny's graph to macOS + Linux drops the rejected-licence count from
25 to 22 (dwrote, instant, libfuzzer-sys are Windows- or fuzz-only); the
image codecs (`rav1e`, `ravif`, `exr`, `avif-serialize`) arrive through
gpui's default `image` features and cannot be shed without a fork.

## Founder rulings, verbatim

```text
这一轮我会裁：

* Q1：三项都接受，但锁屏自驱数据只算 spike 方法证据，最终性能门仍用正常 display-link 环境
* Q2：A，`gpui = "=0.2.2"`
* Q3：A，qwertty-term-vt；客户端必须 poison isolation，但禁止无限 resync loop
* Q4：接受 (i)–(iv)，独立 governance PR；advisory ignore 必须逐项有退出条件
* Q5：P1；接受 A+C 的语义，不接受"权威广播路径直接 `send().await`"
* Q6：A + fidelity regression；明确是 high-consequence B，不是 A
* Q7：B，新 ADR
* Q8：基本接受；但 Linux rendering 是 Linux Desktop support gate，不是无限期 follow-up

可直接落：
PR9 grill — round 1 rulings
Q1 — spike 方法偏差
三项全部接受。
Metal Toolchain
接受并保留本机安装。
它不是临时污染需要回滚：
PR9 的 GPUI/macOS 构建路径本身需要该 toolchain。
Spike 记录应写清：

* 安装行为发生过
* approximate download/install size
* 它修改的是开发机/Xcode toolchain state
* 不属于 Corral repository artifact

不要求为了"恢复 spike 前环境"卸掉。
`CORRAL_TEST_ROOT=/tmp/pr9` 下运行真实 corrald
接受。
前提证据已经表明：

* explicit test root
* no canonical `~/.corral` state
* no real provider
* only controlled `sh`
* no persistent product-state contamination

这属于合理的真实 runtime spike，
比纯 mock 更能验证 Desktop ↔ daemon integration assumptions。
但不要由此削弱前面已经冻结的 e2e isolation invariant：
production/default-root tests
仍然不得靠操作员小心来保证隔离。
锁屏时 `Window::draw` 自驱测帧
接受 spike 方法，
因为解锁后的 real display-link measurement 已经复测并支持同一结论。
但是证据分层：
lock-screen self-driven numbers
→ diagnostic / comparative evidence
unlocked real display-link numbers
→ performance claim evidence
不能以后只拿自驱 draw loop 的数字宣称实际 present/frame behavior。
核心原则：
A methodological deviation is acceptable when its limitation is explicit
and the load-bearing conclusion is independently reproduced through the
real execution path.
Q2 — GPUI pin
选择 A：
`gpui = "=0.2.2"`
不用 git rev。
不用 floating `0.2.x`。
理由：
当前已经存在满足 spike 的 crates.io release。
Exact crates.io version gives:

* reproducible source
* Cargo.lock-friendly provenance
* smaller upgrade review surface
* no unnecessary git-source exception
* no dependency on arbitrary Zed repository history

GPUI upgrade：
must be explicit
例如：
0.2.2 → 0.2.3
需要重新检查至少：

* terminal/render behavior relevant to Corral
* cargo-deny dependency/license/advisory delta
* platform behavior
* affected PR9 regression/performance evidence

`docs/references/architecture-benchmarks.md` / corresponding ledger
应根据新事实更新：
GPUI is now available as a crates.io release;
Corral initially pins exact version 0.2.2.
Reference 文档更新不重新打开原 architecture decision。
Q3 — Desktop terminal replica
选择 A：
qwertty-term-vt
理由：
Corral Desktop 应尽量复现 daemon authoritative terminal model，
而不是仅仅为了绘制引入第二套 VT interpretation。
好处：

* same terminal semantics
* same palette/mode interpretation
* same snapshot dimensions/features
* existing parser fuzz evidence directly relevant
* fewer daemon-vs-client semantic disagreements

第三方 crate 中现存 unsafe 不改变：
Corral-owned crates remain `forbid(unsafe_code)`。
但客户端必须建立独立的 poison boundary。
Replica poison behavior
A malformed snapshot/delta or parser defect must not crash the Desktop
window/process.
On replica parser panic:

1. catch at the narrow replica-apply boundary;
2. immediately discard the entire replica;
3. never reuse partially-mutated replica state;
4. mark terminal presentation unavailable / semantic state Unknown as
appropriate;
5. request a fresh resync;
6. rebuild only from a complete fresh snapshot.

Do NOT attempt to continue applying deltas to the poisoned replica.
No infinite resync loop
还要加一个约束：
A snapshot that repeatedly poisons the client must not cause:
panic
→ resync
→ same panic
→ resync
→ ...
forever.
After a resync attempt also fails before a successful replica is
established:
→ stop automatic retry for that failure episode
→ surface Screen/Terminal unavailable
→ retain daemon/session health independently
→ require a later explicit/new authoritative recovery event before trying
again
具体 retry token/epoch implementation 可以留实现，
但必须有 bounded-loop test。
客户端 poison：
does NOT imply daemon terminal is poisoned
does NOT imply process exited
does NOT imply Session failed
它只是：
this Desktop replica cannot currently make a safe screen claim.
核心不变量：
A replica panic destroys the replica, not the Desktop process and not the
session truth.
Q4 — `deny.toml` policy
接受 (i)–(iv)，并且在 PR9 implementation plan 之前单独落一个
dependency/governance PR。
(i) Graph targets
接受仅分析：

* macOS
* Linux

前提：
这正是当前 Corral Desktop supported build target envelope。
未来增加 Windows：
MUST expand cargo-deny target graph before Windows can become a supported
Desktop target.
不能让这个配置变成永久躲避 Windows-only dependency review 的办法。
(ii) License allowlist
接受新增：

* BSD-2-Clause
* BSD-3-Clause
* ISC
* MPL-2.0
* CC0-1.0

以及实测 dependency graph 中所需的复合 expression：

* ring expression
* encoding_rs expression

但 decision record 应说明：
MPL-2.0 is accepted deliberately as file-level weak copyleft,
not accidentally classified as permissive.
Any applicable source/distribution obligations remain applicable.
(iii) unmaintained advisory ignores
六项可以 ignore：

* async-std
* paste
* proc-macro-error2
* rustls-pemfile
* rustybuzz
* ttf-parser

但每条 ignore 必须是具体 debt record，而不是：
"GPUI dependency, shrug."
每项至少记录：

* advisory id
* package/version
* transitive path / owner = GPUI
* why Corral cannot reasonably replace it independently
* current exposure/risk note
* removal/review trigger

统一 review trigger 可以包括：

* every GPUI upgrade
* replacement/removal upstream
* advisory severity materially changes
* before M1 public release if still unresolved

如果 governance 支持 review-by date，
也可以附明确复核日期；
不要形成无人再看的永久 exception。
(iv) duplicates
保持 warn。
当前没有证据证明 duplicate-version cleanup 值得成为 merge blocker。
Classification
这是 dependency policy / third-party surface change：
HUMAN_REVIEW_REQUIRED
+
human merge
独立 PR 是对的。
它先于 PR9 implementation，
使后续 PR9 使用稳定的 merge-ready definition。
不要把 Desktop 做成第二 workspace 来绕 gate。
Core principle:
One repository keeps one merge-ready dependency policy;
necessary exceptions are explicit debt, not hidden by workspace topology.
Q5 — sustained-output channel failure
严重度：
P1。
原因：
在正常 reader 存在时，
持续输出即可高概率导致 terminal data channel 被关闭；
而且 TUI 与未来 Desktop 都受影响。
这是 shared terminal runtime/data-plane correctness failure，
不是 GPUI 特有问题。
接受：
A + C 的产品/协议语义
但拒绝：
在 PTY reader / authoritative VT update / shared broadcast path 上直接
`send().await` 最多 2 秒。
那会违反已经冻结的不变量：
A slow viewer must never backpressure the PTY reader or authoritative VT.
正确 owner boundary：
Authoritative path
PTY reader
→ update authoritative VT
→ publish/coalesce per-subscriber work
must remain non-blocking with respect to any particular subscriber.
Per-subscriber writer
每个 subscriber 自己的 writer/task 可以：
await socket progress
with bounded no-progress deadline:
2 seconds initial policy
因此一个 viewer 的 kernel/socket stall：
不能阻塞：

* PTY ingestion
* daemon VT truth
* other viewers

Short burst
短暂 socket/consumer jitter：
→ subscriber writer waits/drains
→ connection remains healthy
不要因为瞬时 8-frame queue 满立即断线。
Subscriber falls behind
如果该 subscriber 的 encoded pending terminal state 超过 accepted
per-viewer budget（现有 4 MiB）：
→ mark that subscriber desynchronized
→ never drop an interior delta and continue as if valid
→ discard/coalesce obsolete pending deltas behind an explicit resync barrier
→ arrange a fresh authoritative snapshot/current epoch for that subscriber
它重新进入：
snapshot
→ current ordered deltas
而不是：
old delta 1
[drop middle]
new delta 200
Completely non-reading subscriber
If the subscriber makes no useful write progress for the 2-second
no-progress deadline:
→ close/drop that subscriber
2 seconds is an initial operational policy,
not a wire guarantee.
"FLUSH_GRACE 也是 2s"可以作为初值一致性的理由，
但不要把两个不同生命周期概念硬定义为永远相同。
Required regressions
至少：

1. normal reader + sustained 10-second high-output storm
→ channel remains attached
→ replica converges to authoritative state
2. client stops reading completely
→ other clients/runtime continue unaffected
→ stalled subscriber is dropped within bounded policy time
3. slow reader crosses 4 MiB backlog
→ enters resync path
→ no invalid partial stream is treated as synchronized
4. two subscribers:
one stalled, one normal
→ normal subscriber remains healthy throughout

P1 修复必须先于 PR9 Desktop implementation，
因为 Desktop 不应该建立在一个已知会随机切断的 shared data plane 上。
Q6 — snapshot fidelity defects S1/S2
选择 A + permanent fidelity regression harness。
但是 classification 明确为：
high-consequence Class B
不是 Class A。
理由：
它触碰 terminal snapshot correctness，
即使没有改变 architecture/protocol semantics。
Local compatibility repair
Corral snapshot render adapter 可以在 formatter 输出后修正：

* required viewport row completion
* final cursor positioning
* cursor visibility state

使结果重新满足 ADR 0003 已接受的 snapshot contract。
不要声称 qwertty formatter 自身已被修复。
记录：
Corral adapter compensates for upstream formatter behavior.
Upstream patch/issue：
follow-up
不让上游 merge/release 成为 Corral correctness dependency。
Regression harness
把 spike fidelity harness 蒸馏成永久测试。
测试：
authoritative qwertty terminal state
→ Corral snapshot encoder
→ fresh qwertty replica
→ compare reconstructed state with authoritative state
至少比较：

* every visible cell/content
* styles/attributes relevant to snapshot contract
* cursor position
* cursor visibility
* geometry
* whichever modes are already part of the accepted snapshot contract

覆盖 spike S1 的 22 scenarios，
包括：

* trailing blank viewport rows
* cursor on/after blank regions
* tab-related sequences
* visibility transitions
* resize-related representative states

如果某些 state 属于 Q7 尚未接受的新协议语义，
不要在 Q6 偷偷把它们声明成已经支持。
Q6 只修：
existing accepted snapshot fidelity.
核心原则：
Corral may compensate for an upstream serializer defect,
but the regression test owns the user-visible fidelity contract.
Q7 — S3/S4/S5 protocol semantics
选择 B：
新 ADR。
不要只改 PR9 plan。
不要偷偷塞进 ADR 0003 implementation notes。
新 ADR scope：
Terminal snapshot geometry, dual-screen state, and palette transport
引用 ADR 0003，
明确这是根据 Desktop replica spike 暴露出的 protocol completeness
requirements。
至少需要裁三个 durable/wire semantics：
Dual screen
When alternate screen is active,
a recoverable snapshot must carry enough state to reconstruct:

* active alternate screen
* preserved main screen
* which screen is currently active

否则退出 alternate screen 后 client 无法恢复正确 main content。
Geometry
Snapshot must establish authoritative terminal geometry.
A newly attached/resynced replica must not need ambient/local assumptions
to know rows/cols.
具体：
snapshot field
or preceding geometry frame
由 ADR 裁。
Palette
If palette can mutate over terminal lifetime,
connection must receive the palette state/change semantics necessary for
faithful replica rendering.
这可能需要：

* snapshot palette baseline
* palette update frame
* or equivalent explicit per-connection semantic

不能靠客户端默认 palette 猜。
同时新 ADR 必须决定：

* compatibility/version/capability behavior
* old client behavior
* snapshot/resync interaction
* epoch interaction where relevant

在 ADR accepted 前：
可以研究/写 mechanics scaffolding，
不能 merge 实际 wire semantic change。
这是 compatibility-facing Class C decision。
Q8 — deferred measurements
基本接受。
Release-profile frame/performance numbers
推迟到 PR9 implementation completion。
但它是 PR9 DoD/release evidence：
the same relevant harness must be rerun under release profile.
不要用 debug/spike numbers 当最终 Desktop performance evidence。
Linux rendering
当前没有真实 Linux display environment，
所以不作为这个 spike/PR9-plan 的前置 blocker。
但它不能成为无限期 follow-up。
If Corral M1 claims Linux Desktop as supported:
real Linux Desktop rendering becomes a support/release entry gate before
that claim.
允许：
PR9 merge
with Linux compile/test coverage
+
Linux rendering marked unvalidated
前提是产品/support matrix 不提前宣称它已验证。
如果 PR9/M1 只先支持 macOS Desktop，
则 Linux 可以继续作为后续 platform matrix expansion。
GPU present timing
同意不单独拆。
现有 real display-link/vsync end-to-end observation
更接近用户实际感受到的 frame behavior。
除非以后性能问题明确指向：
CPU render vs GPU submission/present boundary
才值得增加单独 instrumentation。
Core principle:
Measure the user-visible frame path first;
split internal timing only when a concrete bottleneck requires it.
我认为这轮真正改变你下一步排序的是 Q5：
先修 shared terminal data plane，再写 PR9 plan。
否则 Desktop spike 已经证明出来的 9/12 断线会被 PR9 误当成 Desktop integration bug，后面调 GPUI 全是噪声。
然后是 Q6，把 snapshot fidelity harness 正式化；再落 Q7 ADR。这三件清掉之后，PR9 plan 才有一个稳定的 runtime/protocol 地基。
```

## What round 1 leaves open

Sequencing and ownership of the four follow-ups (S6 fix, S1/S2 fix with
the fidelity regression, the Q7 ADR, the Q4 governance PR) relative to the
PR9 plan, and the PR9 plan's own boundary. Round 2.
