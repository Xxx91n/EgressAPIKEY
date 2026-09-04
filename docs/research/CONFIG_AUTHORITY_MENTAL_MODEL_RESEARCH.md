# 配置权威心智模型调研：桌面代理 × 基础设施两套生态的对照与 Egress 选型

> 沉淀自 2026-09-03 派发的 atomcode 三引擎调研（architecture-recovery 票 37；17 检索：Exa 8 / Tavily 5 / AnySearch 4；15 篇全文核验 + 12+ 引擎内联正文核验；角度 Official / Comparative / Criticism / Community / Currency 5/5）。性质：研究记录（Explanation 象限），不是现状规范——as-built 模型见 docs/architecture/ARCHITECTURE.md「Config Authority」一节，设计决策见 docs/adr/（尤其 0036 / 0039 / 0042 / 0051 / 0054 / 0056）。
> 引用纪律：本票与票 21（docs/research/CONFIG_AUTHORITY_INDUSTRY_RESEARCH.md）的关系是「增量，不复读」——票 21 已建立「桌面客户端三层 + 单向生成管线 + 三道证明闸」行业基线，本文在其上回答两件事：最适合本架构的配置权威心智模型是什么、还缺哪一环工业级成熟做法。正文引注 \[S#\] 对应 §5 来源清单。

## 1. 摘要（结论先行）

两套生态的根本分野不在「声明式 vs 命令式」，而在**是否有一个独立于写路径的观察者（controller）持续比较 desired 与 live，并把比较结果发布为机读状态**。

- 基础设施侧（K8s controller → ArgoCD/Flux → Terraform + HCP 定时 drift）已把它做成工业级：spec/status 分离、`observedGeneration`、Synced/OutOfSync × Healthy/Degraded 双轴、selfHeal/prune 默认关、字段级 ignoreDifferences。
- 桌面代理侧（Clash Verge Rev、v2rayN、sing-box）普遍停留在「多层输入 → 生成/合并 → 丢给内核重载」的单向流水线：内核能 `GET /configs` 读回运行态（mihomo）、sing-box 有 check-then-swap 重载，但**没有任何客户端把「用户最新一次编辑是否已被内核消化」做成持久、机读、双轴的状态**——「改了什么 vs 实际跑什么 vs 服务是否真的可用」三件事混在一起，故障形态是静默覆盖（Verge #7696 / #7804）与「配置对但没生效」（TUN/系统代理开关独立）。

对一个单机桌面代理应用，最合适的组合是 **OpenGitOps 四原则缩小为单机 controller 循环（git→白盒文件、cluster→sidecar L3、ArgoCD 的 sync 语义 + selfHeal 默认关）+ Terraform 式绑定存储（SQLite 存 identity 映射）+ kubectl apply 式 3-way 所有权 + K8s 的 spec/status 与 observedGeneration 纪律**。Egress 的 L1/L2/L3 + `authoritative_snapshot` + 单向 reconcile 已站在这个模型的正确一侧（ADR-0051 / 0054 / 0056 已显式引用 ArgoCD desired/live、selfHeal 默认关与 OpenGitOps）。

仍缺的工业级四环（按性价比排序）：**(a) 持久化「spec 代次 → observed 代次」回显**；**(b) 只读后台定时复验 + 状态跃迁通知**；**(c) sync 轴与 health 轴分离**；**(d) 轻量写审计日志**。

Confidence：**高**（两套生态的事实性结论均有官方文档全文核验 + 多源交叉）；**中**（推荐部分为设计判断，锚定本仓 ADR 语境，非基准评测）。

## 2. 两套生态的心智模型

### 2.1 桌面代理客户端：「分层/生成 → 内核重载」，缺独立观察者 + 机读状态

- **A1. Clash Verge Rev = 主 profile + 增强层链式合并 → 一次性生成最终 YAML 交给内核**。主 profile（Remote 订阅 / Local 文件）+ 增强层（Merge/Extended Config、Script/Extended Script，全局或按订阅）链式顺序处理；本地自建 Merge 与订阅文件分开存储，刷新订阅不擦除，但直接编辑下载下来的订阅文件会在刷新时被抹掉。另有保留键分界：`mixed-port`/`log-level`/`external-controller` 等由程序控制、用户 merge 无法覆盖——这是 GUI 自留字段 vs 用户字段的显式所有权划分 \[S1\]\[S6\]\[S7\]。
- **A2. mihomo 内核提供「运行中配置」读回与热重载 API——桌面端唯一标准的「生效配置」事实源，但各 GUI 没有把它升级为面向用户的证明状态**。`GET /configs` 返回当前内存运行态；`PATCH /configs` 热改单项；`PUT /configs?force=true` 整包重载（解析失败返回 400、不生效）；`POST /restart` 重启。也就是说「内核实际跑的配置」技术上可读、可核对，但 Clash 系 GUI 的「已生效」证明停留在 UI 开关与手动验证 \[S2\]\[S14\]。
- **A3. 桌面端代表性失败教训 = 静默覆盖与静默重置（用户编辑丢失而不报错）**。根因多是「部分字段写入 + merge 默认值语义」与「索引与文件双写不一致」：Verge #7696（缺省字段覆盖用户存的 `false`）、#7804（启动清理竞态删掉 18/20 个 profile）、讨论 #4060（自定义规则写进订阅 YAML 刷新即丢）。另一经典源：内核配置与系统代理/TUN 开关是两条独立路径（「配置看着对但流量不走代理」）\[S7\]\[S8\]\[S9\]\[S11\]。
- **A4. v2rayN = 「生成型」模型：用户输入是 SQLite 里的 ProfileItem + 全局设置，运行配置是每次连接时按需重建的派生 JSON**。生成前 `NodeValidator` 预校验节点/内核兼容性。心智模型是「GUI 模型为权威、内核配置为可丢弃派生物」，比「持久 YAML 文件权威」更接近三层架构，但同样没有「运行配置 vs 用户输入」的读回对比 UI \[S12\]\[S13\]。
- **A5. sing-box = 内核派最接近「validate-then-swap」：单一 JSON 权威 + `check` 先行验证 + SIGHUP 整实例重载，无效配置保持旧配置继续运行**。check 失败则旧实例继续跑（nginx/haproxy 式「先验证、坏配置不换」）；代价是重载=全新实例、在途连接被重置。sing-box 把「输入→生效」显式化为 CLI 工具链而非 GUI 魔法——这是它对「证明生效」最接近工业纪律的部分 \[S15\]\[S16\]\[S17\]。

**A 侧小结**：桌面生态的共同心智模型是「分层/生成 → 内核重载」，缺「独立观察者 + 机读状态」；验证闸门（sing-box check、Verge 脚本错误变红、v2rayN NodeValidator）零散存在但不成体系；最成熟的一环恰是内核侧（mihomo `GET /configs` 读回运行态、错误配置拒绝加载）。

### 2.2 基础设施侧：「desired/live + 独立 controller + 双轴机读 status」

- **B1. 心智模型 = 期望（声明在权威存储）vs 实况（运行时），由独立 controller 持续 reconcile，状态发布为两轴机读 status**。ArgoCD：Target state（git）= desired、Live state = cluster 实况、Sync = 拉近两者、Sync status =「live 是否等于 target」、Health =「是否真的在正确工作」，两者独立（Synced-but-Degraded 与 OutOfSync-but-Healthy 都是合法象限）。OpenGitOps 四原则：Declarative / Versioned & Immutable / Pulled Automatically / Continuously Reconciled \[S3\]\[S5\]\[S18\]。
- **B2. ArgoCD 的安全语义是「默认不自动纠正」：selfHeal 与 prune 均默认关，同步是显式/按策略触发**。事故教训（多源一致）：团队手动 `kubectl scale` 降载，auto-heal 在 ~90 秒内把副本拉回原值、压垮数据库引发级联——「紧急运维必须走 git 而非 kubectl」是纪律约束而非技术特性。本仓 ADR-0054 拒绝 auto-heal/后台自动 reconcile 引用的正是同一论据 \[S4\]\[S19\]\[S20\]。
- **B3. 工业级「证明已生效」的机读机制 = spec/status 分离 + `observedGeneration` + conditions（reason/message/lastTransitionTime）**。controller 处理完一代 spec 后把 `status.observedGeneration` 置为 `metadata.generation`，从而区分「已按最新 spec 收敛」与「controller 还没看到/还没处理完」；ArgoCD 的 Deployment health（Progressing）同样把 observed generation 落后列为条件 \[S10\]\[S18\]\[S22\]。
- **B4. Terraform 模型 = 配置文件是期望，`.tfstate` 是「remote object ↔ resource instance」的绑定存储；drift 靠每次 plan/apply 前的 refresh 暴露，HCP 提供定时只读 drift detection**。`-refresh-only`（0.15.4+）只探测与呈现漂移、不写状态不动作。已知教训：被管理资源的手工改动 = drift，下一次 apply 可能「无意销毁或重建」资源；state 文件丢失/损坏、多 actor 并发改同一 state、`ignore_changes` 漏标 \[S23\]\[S24\]\[S25\]。
- **B5. 两轴与 diffing 的已知坑：同步成功仍可能立刻 OutOfSync；controller/webhook 改写与 server 默认值会造成「假漂移」（注：此外部 GitOps 机制与本仓无关——Resin 上游无 webhook/callback/push 出口，G4 信号通道为 sidecar-status 事件订阅），工业解法是字段级/owner 级 ignoreDifferences**。核心教训：「漂移判定」必须理解谁合法地改字段（server 默认、其他 controller、自家程序保留键），否则产生永久性噪音——这与 A1 里 Verge 的「程序自留键不可覆盖」是同一问题的两侧 \[S3\]\[S6\]。

### 2.3 成熟度与取舍

基础设施侧十年演进形成了完整词汇表（desired/live/refresh/sync/health/observedGeneration/selfHeal/prune/ignoreDifferences/operationState），且**状态是持久化、机读、持续刷新的**；桌面侧词汇表只有「导入/更新/应用/重载/报错变红」，状态是瞬时的、面向人眼的。

取舍上：GitOps 的持续自动纠正在桌面单机场景反而危险（会把用户正在手动调试的运行时拉回），因此 Egress 选「显式 apply + 观察性快照」是正确缩放；Terraform 的 plan/apply 人际闸门在单机上缩成 reconcile preview（预演→确认→执行→自动复验），其「只读 refresh 探测」阶段却值得补回（见 §4 缺口 b）。

## 3. 对本架构的映射（L1/L2/L3 + desired/live 三态）

| Egress 概念 | 工业对应物 | 心智模型映射 |
|---|---|---|
| L2 白盒 JSON（strategy/ports）单写入口 | git（GitOps desired）/ Terraform HCL | **desired state**：唯一权威输入，版本化、immutable by design |
| L3 Resin 运行时 + sidecar | cluster（live）/ 真实云资源 | **live state**：派生、可重建，绝不手编（v2rayN #6327 同源教训） |
| L1 GUI 偏好 settings.json | app 层设置（verge.yaml 同族） | 偏好层，永不改变代理行为（Verge #3395 的层归属教训） |
| `authoritative_snapshot` 三态合并 | ArgoCD sync status / CQRS read model | **读回观测**：consistent≈Synced、divergent≈OutOfSync、missingOnResin≈Orphaned/Missing |
| 单向 reconcile（白盒必胜） | ArgoCD sync + selfHeal/prune 默认关 | **显式单向收敛**，无「接受现状」反写、无自动重放 |
| `acknowledged` 已知豁免 | ArgoCD `ignoreDifferences` | **假漂移治理**：豁免只改呈现、不改三态合并（与 B5 同构） |
| Whitebox Versioning（backup/ 10 份轮转） | Versioned & Immutable / Terraform state 版本 | **可回滚历史**，但≠审计（缺「谁/何时/改了什么」，见缺口 d） |
| `egressapikey.db` `port_mappings` | Terraform `.tfstate` 绑定存储 | **身份映射存储**，不是期望配置；端口号即身份 |
| WhiteboxConfigStore 文件 watch + apply 事务 | kubectl apply 3-way / 外部编辑发现 | **外部编辑靠 watch 发现**，单一声明式写者 |
| validate-before-swap | sing-box `check` 闸门 | **验证闸门**：坏配置不换、旧态保持 |

## 4. 推荐模型与补充清单（四环）

**应采用（Egress 已对齐的 60%）**：以「白盒=唯一 desired、sidecar=live、快照=读回观测」为骨架，叠加 K8s 的代次回显、Terraform 的只读漂移探测、ArgoCD 的双轴状态与默认不自动纠正。桌面代理不需要 git，需要的是把「最后一次编辑被收敛」变成一条持久、机读、双轴、有审计的状态线。

**仍缺的四环（按性价比排序）**：

1. **`generation → observedGeneration` 回显（最缺、最便宜）**。现在快照证明的是「当前 desired==live」，**证明不了「你 10 秒前那次编辑已被内核消化」**——若 apply 失败或队列未处理，三态仍可能暂时显示一致。给两个白盒各加单调写计数（每次写入入口 bump），apply/reconcile 后把 `applied_generation`/`last_applied_at`/`last_apply_error` 持久化并随 snapshot 回带，UI 顶部常驻「已生效于 HH:MM（rev N）」。这是把 K8s `observedGeneration` 惯例与 Terraform「apply 后导出状态工件」落到桌面；与 ADR-0054 已拒绝的「持久 divergentSince」不同——那是漂移计时，这是**最后一次成功收敛的版本标记**。
2. **只读后台定时复验 + 状态跃迁通知**。现复验是事件/手动驱动；sidecar 被外部重启、region_filters 被其他客户端改掉时，应用空闲期会静默漂移直到用户打开视图。补一个 sidecar Running 时的低频只读 snapshot（60–300s，HCP drift assessment / Flux interval 同构），通知语义从「每进程一次」升级为「状态持久 + 通知只在跃迁时」。这与 ADR-0054 拒绝的 auto-heal **不是一回事**：只观察、不自动纠正（`-refresh-only` 语义）。
3. **sync 轴与 health 轴分离**。把现有 `port_health_check`/`probe_exit_ip` 等健康数据并入每实体状态，采用 conditions 词汇（reason/message/lastTransitionTime），让 UI 能表达桌面领域最高频的「Synced but Degraded」象限——「三态徽标全绿 ≠ 流量真的在走代理」（TUN/系统代理独立开关）。这直接回答排障文档第一行「改了为什么没生效」之外的「生效了为什么没用」。
4. **轻量写审计日志**。版本化备份保留 10 份 ≠ git 历史：无「谁/何时/哪一层/从什么改成什么」。补 append-only JSONL（ts / source GUI|外部文件|进程回滚 / before-hash / after-hash）即获得 GitOps 审计能力（「versioned & immutable」原则的桌面实现）。

一句话总结：**以「白盒=唯一 desired、sidecar=live、快照=读回观测」为骨架，叠加 K8s 的代次回显、Terraform 的只读漂移探测、ArgoCD 的双轴状态与默认不自动纠正——桌面代理不需要 git，需要的是把「最后一次编辑被收敛」变成一条持久、机读、双轴、有审计的状态线。**

## 5. 来源清单

| # | 标题 / URL | 角度 | 贡献 |
|---|---|---|---|
| S1 | Clash Verge Rev User Guide — <https://clashvergerev.com/en/guide> | Official(社区维护) | 主 profile+增强层链式合并；脚本错误变红 |
| S2 | mihomo docs — API — <https://wiki.metacubex.one/en/api/> | Official | GET/PUT/PATCH /configs 读回与热重载、/restart |
| S3 | ArgoCD Core Concepts — <https://argo-cd.readthedocs.io/en/stable/core_concepts/> | Official | target/live/sync/health 官方定义 |
| S4 | ArgoCD Automated Sync Policy — <https://argo-cd.readthedocs.io/en/stable/user-guide/auto_sync/> | Official | selfHeal/prune 默认关、prune 安全机制 |
| S5 | OpenGitOps — <https://opengitops.dev/> | Official (v1.0.0) | 四原则 |
| S6 | CVR Merge 配置指南 — <https://clashvergerev.com/en/guide/merge> | Official(社区) | 程序自留键不可覆盖 |
| S7 | CVR discussion #4060 — <https://github.com/clash-verge-rev/clash-verge-rev/discussions/4060> | Community | 编辑订阅被刷新抹掉；merge 层独立 |
| S8 | CVR issue #7696 — <https://github.com/clash-verge-rev/clash-verge-rev/issues/7696> | Criticism | 部分 payload+serde 默认+merge 覆盖用户值 |
| S9 | CVR issue #7804 — <https://github.com/clash-verge-rev/clash-verge-rev/issues/7804> | Criticism | 索引/文件双写竞态、静默删 profile |
| S10 | Kubernetes Controllers — <https://kubernetes.io/docs/concepts/architecture/controller/> | Official | 控制回路、desired vs current、spec/status |
| S11 | K8s Declarative Management (kubectl apply) — <https://kubernetes.io/docs/tasks/manage-kubernetes-objects/declarative-config/> | Official | last-applied 3-way merge、diff 预览 |
| S12 | v2rayN Configuration Generation (DeepWiki) — <https://deepwiki.com/2dust/v2rayN/5.2-configuration-generation> | Community(代码派生) | ProfileItem→内核 JSON 生成管线、NodeValidator |
| S13 | v2rayN DeepWiki 总览 — <https://deepwiki.com/2dust/v2rayN> | Community(代码派生) | 存储分层、CoreConfigHandler 分派 |
| S14 | mihomo hub/route/configs.go — <https://github.com/metacubex/mihomo/blob/v1.19.27/hub/route/configs.go> | Official | GET 返回内存态；PUT 解析失败 400 |
| S15 | sing-box Configuration — <https://sing-box.sagernet.org/configuration/> | Official | JSON 结构 + check/format/merge CLI |
| S16 | sing-box issue #23 — <https://github.com/SagerNet/sing-box/issues/23> | Criticism | 先验证后换、坏配置保持旧实例 |
| S17 | sing-box issue #3731 — <https://github.com/SagerNet/sing-box/issues/3731> | Criticism | 重载=新实例、在途连接重置 |
| S18 | ArgoCD Sync vs Health 详解 — <https://oneuptime.com/blog/post/2026-02-26-argocd-sync-status-vs-health-status/view> | Comparative | 双轴独立、observed generation 参与 health |
| S19 | ArgoCD GitOps auto-heal 事故复盘 — <https://thecodeforge.io/devops/argocd-gitops/> | Criticism | auto-heal 拉回手动降载→级联 |
| S20 | ArgoCD vs Flux 2026 对比 — <https://www.portainer.io/blog/argocd-vs-flux> | Comparative | ArgoCD opt-in vs Flux 默认自动 |
| S21 | GitOps in 2026 综述 — <https://es.nl/2026/gitops-in-2026-reconciliation-drift-argo-cd-vs-flux> | Currency | 「Terraform 但无人 watch≠GitOps」 |
| S22 | CRD Status Convention (kpt.dev) — <https://kpt.dev/reference/schema/crd-status-convention> | Official | observedGeneration 语义与 conditions 惯例 |
| S23 | Terraform State — <https://developer.hashicorp.com/terraform/language/state> | Official | state=绑定存储；apply 后导出 JSON 工件 |
| S24 | Terraform Manage resource drift — <https://developer.hashicorp.com/terraform/tutorials/state/resource-drift> | Official | 手工改动→drift；-refresh-only；HCP 定时 alert |
| S25 | HN: Stategraph / Terraform drift 讨论 — <https://news.ycombinator.com/item?id=45273352> | Community | ignore_changes 漏标、多 actor 漂移 |
| S26 | CNCF: Solving config drift with Argo CD — <https://www.cncf.io/blog/2020/12/17/solving-configuration-drift-using-gitops-with-argo-cd> | Official | kubectl 旁路→out-of-sync；git 历史=部署审计 |

站点可信度分歧提示：CVR 文档分散于多个社区站且互有版本差（如 Merge→Extended Config 更名时间线不一致），本票仅采用 GitHub issue/讨论与多站一致的结论。

## 6. 快照时点与信息缺口

**docs 快照时点：2026-09-03**。本票读取时，并行票 30–35 已提交（其报告已落 `.scratch/architecture-recovery/reports/`），CONTEXT.md / ARCHITECTURE.md / glossary.md / architecture-state.md 已是各票修正后的状态；票 36（strategy apply 幂等语义）仍在 W2（Blocked by 35），可能改动 `strategy_service.rs` 与 reconcile 相关文档——本结论以当前 as-built 为准，若票 36 落地改变 reconcile 幂等语义，§3 映射表中「单向 reconcile」一行需复核。

**信息缺口（照录，供后续调研接力）**：

1. v2rayN 一手设计文档缺失——其模型结论全部来自代码派生的 DeepWiki，无官方「为什么这样存」的说明。
2. CVR 官方文档权威站不明（clashvergerev.com / clashverge.dev / clashverges.org 均似社区维护），Merge→Extended Config 更名的确切版本未逐版本核对。
3. 桌面客户端「已生效证明」无任何形式化研究（无论文/行业报告量化「配置改了没生效」类工单占比）。
4. mihomo 内核 embed 模式下 `GET /configs` 是否同样可用未在 GUI 源码中验证。
5. 开放问题：缺口 a（generation 回显）若实现，快照 three-state 判定是否应把「observed_generation < generation」单列为第四态（K8s 中这正是 Progressing 与 Unknown 的边界）；写审计日志与现有 backup/10 份版本化机制如何不重复。
