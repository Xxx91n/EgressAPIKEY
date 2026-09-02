# 行业对照研究:桌面代理客户端的「配置权威」与三层模型

> 沉淀自 2026-08-31 派发的 atomcode 三引擎调研(生态 17 源 + 学术/工业命名两轮;知识库留档批次 atomcode config-authority research、atomcode mental-model research v2/v3),落盘于 2026-09-01(architecture-recovery Round 3 · 票 21)。
> 性质:研究记录(Explanation 象限),不是现状规范——as-built 模型见 docs/architecture/ARCHITECTURE.md 的 Config Authority 一节,设计决策见 docs/adr/。
> 引用纪律:除 §8.1 中 S11/S12 两条沿用原调研「搜索摘要核验」标注外,全部链接在落盘当日逐一实抓核验(HTTP 200)。正文引注 [S#] 对应 §8.1, [A#]/[R#] 对应 §8.2。

## 1. 摘要

1. 业内没有统一的正式术语 config authority,但所有成熟桌面代理客户端(CFW / Clash Verge Rev / v2rayN / Hiddify / NekoBox / Mihomo Party / sing-box 系)事实上收敛到同一个三层心智模型:
   - **L1 GUI 偏好层**:app 自己的设置文件(verge.yaml、HiddifyOptions、v2rayN 的 guiNConfig 数据库等);
   - **L2 用户可编辑的权威配置输入层**:profiles / 订阅 / 自定义 JSON——用户编辑的是输入;
   - **L3 内核运行时状态层**:mihomo / sing-box / xray 内核加载的产物与运行时——内核加载的是产物。
2. 三层之间由一条**单向生成管线**连接:GUI 编辑 → 合并/转换 → 写运行时配置 → 校验门 → 重启/热重载内核。
3. 「证明配置真的生效」不靠信任,而靠**三道显式证明闸**:校验门(mihomo -t / sing-box check)、有效配置可视化(Runtime Config 视图 / GET /configs 回读)、运行时状态回读(/proxies、/rules hitCount、connections)。
4. 该心智模型最接近「正式命名」的文档化:Clash Verge Rev 的 Profile Processing Flow(官方文档)与 sing-box 官方 Graphical Clients 规范(强制 GUI 为 Profile 提供编辑器/查看器);基础设施侧的成熟对应物是 desired-state reconciliation(OpenGitOps 四原则、Kubernetes controller、Terraform plan/apply、CQRS read model)。
5. 该领域最大的痛点不是「模型不存在」,而是「模型存在但未向用户良好传达」:CVR #1715(扩展配置叫 Merge 功能却是覆盖)与 #3395(verge.yaml 被覆盖)是一等公民证据;§5 的四则事故全部同源。

## 2. 业界三层模型对照表

| 客户端 | L1 GUI 偏好存储 | L2 权威配置输入 | L3 运行时状态 | 证明生效的机制 | 生成引擎 |
|---|---|---|---|---|---|
| CFW(Clash for Windows) | GUI 设置 | 订阅 + 「配置文件预处理」parser/mixin(Merge/Script 前身)\[S11\] | clash core | 内核日志 | parser/mixin 预处理 |
| Clash Verge Rev | verge.yaml + profiles.yaml \[S12\]\[S8\] | profiles/ 四类 profile(Remote/Local/Merge/Script)链式增强 \[S1\] | mihomo core;REST+WS API | Settings→Runtime Config 视图、mihomo -t 校验门、Runtime Logs、运行时 YAML 导出 \[S12\]\[S9\] | Rust enhance() 十步流水线 + Boa JS + deep merge \[S8\] |
| v2rayN | GUI 数据库(guiNConfig)唯一写入口 [S13] | ProfileItem 服务器列表 + Custom 文件 | 启动的核心进程(sing-box/xray/mihomo) | 生成 config.json 落盘文件 + 核心日志 [S13] | CoreConfigHandler 三路 Service(mihomo/sing-box/v2ray)\[S13\] |
| Hiddify | HiddifyOptions(JSON, app settings)\[S5\] | 订阅链接(Clash YAML / V2Ray / sing-box JSON 自动探测)\[S5\] | sing-box core(libbox) | BuildConfigJson + CheckConfigOptions 加载前校验(失败 = 整配置被拒)\[S5\]\[S6\] | hiddify-core 多格式解析→统一 sing-box 构建 \[S5\] |
| NekoBox (NB4A) | 服务器数据库 + 全局自定义 JSON(1.4.0+)\[S7\] | 自定义 JSON(+inbounds / outbounds+ 混入)+ sing-box .json 导入 [S7] | sing-box core | 导出生成的 config.json、内置 Dashboard [S7] | GUI→sing-box 转换;手写完整 JSON 会使 GUI 路由配置失效 [S7] |
| Mihomo Party | app 设置 [S14] | 订阅 + 覆写(override)+ 深度集成 Sub-Store [S14] | mihomo core | Smart Core 规则覆写、WebDAV 备份恢复 [S14] | Electron 覆写引擎(未深挖源码)\[S14\] |
| sing-box 官方规范 | (未规定,交各客户端) | Profile(Local/Remote/iCloud),强制编辑器/查看器 [S2] | sing-box core | check/format/merge 命令 + JSON Schema 校验 [S2] | 规范只定义义务,不定义实现 [S2] |

对照结论(强证据,三源交叉):

- **结论 A**:「GUI 偏好」与「内核配置」存在不同的文件/数据结构。CVR 三文件职责:verge.yaml = app 层设置(主题/语言/热键/端口/tray 行为),profiles.yaml = 订阅列表/排序/当前 profile,内核设置(端口/DNS/TUN/logging)单独一份 \[S12\][S8];langlabs 架构分析将其命名为 IVergeConfig / IConfigData 两个独立配置面 \[S8\];clash-verge.com 用表格逐键标注层归属,并警告不要把不同层的键混在一块编辑 \[S9\];v2rayN 的权威数据面是 GUI 数据库而非任何 YAML——GUI 是唯一写入口 \[S13\]。
- **结论 B**:L2(用户编辑的输入)与 L3(内核加载的产物)物理上是两个文件。CVR 的 profile 链:选主 profile → 增强 profile(Merge/Script)顺序链式处理,前一个的输出是后一个的输入 [S1];NB4A 的自定义 JSON 混入 sing-box 配置,并警告手写完整配置会使 GUI 路由功能失效(GUI 状态与手写 JSON 互斥)\[S7\];sing-box 官方把 L2 规范化为 Profile 概念并强制编辑器/查看器义务 [S2]。
- **结论 C**:L3 经控制 API 回读(mihomo external-controller / libbox),与文件层明确可区分 \[S3\][S2]。
- **CFW 注**:CFW 的「配置文件预处理」(parser)是 CVR Merge/Script 的前身;中文社区大量「订阅更新会覆盖手改配置,要靠 parser/mixin 保活自定义规则」的表述,是「订阅是权威、手改会被覆盖」这一模型最广泛的社会化表述 [S11]。

## 3. 单向生成管线与三道证明闸

### 3.1 生成管线(GUI 编辑 → 权威运行时配置)

- **CVR**(最完整的公开实现):src-tauri/src/enhance/mod.rs 的 enhance() 十步流水线 [S8]:收集 clash 配置与 verge flags(TUN/端口/DNS)→ 加载活动 profile 及其 merge/script/rules/proxies/groups → Global Merge(iterative deep merge, stack-based)→ Global Script(Boa JS 引擎)→ profile 级 Rules(前插/后插/删除)→ profile 级 Merge → Script → 覆写 clash.yaml 默认项(端口/TUN/external-controller,带平台条件逻辑)→ builtin 脚本(meta_guard.js、meta_hy_alpn.js)→ 清理 proxy groups + TUN DNS 注入 + 键排序,然后写运行时配置并触发内核重载。官方文档称为 Profile Processing Flow [S1]。
- **v2rayN**:CoreConfigHandler.GenerateClientConfig 按核心类型三路分派(mihomo → CoreConfigClashService、sing-box → CoreConfigSingboxService、v2ray → CoreConfigV2rayService),生成结果落盘 config.json 再喂给核心进程;Custom 配置直接复制用户文件 [S13]。
- **Hiddify**:hiddify-core 按顺序尝试 JSON → V2Ray → Clash YAML 解析,统一转标准 sing-box 格式,再由 BuildConfigJson(HiddifyOptions, options) 构建完整配置 [S5]。
- **NekoBox**:GUI 服务器数据库 → 生成 sing-box config.json;「导出软件生成的 sing-box config.json」为内置功能 [S7]。

### 3.2 三道证明闸

1. **校验门**(生成后、加载前):mihomo 生态的规范动作是 mihomo -t(test config)——metacubexd 官方 dashboard:Activate a profile to compose it, validate it with mihomo -t, and restart the kernel only after validation succeeds(原调研以搜索摘要核验)\[S3\];Hiddify 调 sing-box libbox.CheckConfigOptions 做加载前校验,其强度见事故参照:hiddify #2228 中一个非法 VLESS 节点使整个配置校验失败被拒 [S6];sing-box 官方提供 check/format/merge 命令与 JSON Schema 校验 [S2]。
2. **有效配置可视化**:CVR Settings → Runtime Config 只读视图,展示合并后的最终 YAML 原文(源码位于 src/components/setting/mods/config-viewer.tsx)\[S12\]\[S9\]。
3. **运行时状态回读**:mihomo /proxies、/rules hitCount、connections 等运行时 API [S3]。

## 4. 成熟实现亮点(版本化 / 预览 / 回滚 / desired-vs-live)

### 4.1 Roxy-WI(原 HAProxy-WI):负载均衡配置 GUI 的版本化先例

官网原文(2026-09-01 实抓):Change configs safely — Validate before deployment, keep version history and restore a known-good configuration when something changes. 功能清单:Config validation and history;Git synchronization and backups;统一管理 HAProxy、NGINX、Apache、Keepalived [R1]。

可借鉴点:部署前校验 + 版本历史 + 一键恢复已知良好配置,是「配置权威」在负载均衡运维 GUI 里的同族实现;本项目 ADR-0054 §B 的白盒版本化(写前备份 / 轮转 / 回滚复用同一 validate→apply 链)与之同构。

### 4.2 Kubernetes controller / OpenGitOps:desired/live 对比的立法来源

- **Kubernetes 官方**:Controllers are control loops that watch the state of your cluster, then make or request changes where needed. Each controller tries to move the current cluster state closer to the desired state. spec 字段即权威;观测现状回写 status 供其它控制环消费 [R2]。
- **OpenGitOps 四原则(v1.0.0)**:Declarative;Versioned and Immutable;Pulled Automatically;Continuously Reconciled [R6]。
- **Flux**:reconciliation 默认每 5 分钟重跑;If you make any changes to the cluster using kubectl edit/patch/delete, they will be promptly reverted. You either suspend the reconciliation or push your changes to a Git repository.(官网原文,实抓核验)\[R4\]。
- **ArgoCD**:desired-vs-live diff 为一等公民(Application 详情页 + Sync 状态 + ignoreDifferences 豁免)\[R5\];自动同步的自愈默认**关闭**——By default, changes that are made to the live cluster will not trigger automated sync;self-heal 为显式 opt-in(argocd app set --self-heal 或 spec.syncPolicy.automated.selfHeal: true)\[R3\]。
- **Terraform**:plan 生成执行计划、先预览变更(creates an execution plan, which lets you preview the changes)再 apply——「先预演、人决策」语义的来源 [R7]。
- **CQRS(Martin Fowler)**:读写职责分离、读侧独立模型——authoritative_snapshot(一次读齐三层、视图只消费不合并)的学术对应 [R8]。

对映:EgressAPIKEY 的 L2 白盒 = desired state;L3 Resin 运行时 = live state;authoritative_snapshot = CQRS 读侧 read model;三态合并(consistent / divergent / missingOnResin)= 漂移判定,对应 GitOps 的 in-sync / out-of-sync / orphaned。置信度高:OpenGitOps、Kubernetes、Fowler、ArgoCD/Flux/Terraform 官方文档直接支撑。

### 4.3 业界如何摆放「生效配置」视图(信息架构对照)

| 产品 | 生效配置位置 | 漂移机制 | 自愈默认 | 权威源 |
|---|---|---|---|---|
| ArgoCD | 一级页面(Application 详情 diff 面板 + Sync 状态)| 逐资源 desired-vs-live diff | selfHeal 默认关 | Git(desired)vs 集群(live)|
| Flux | 无第一方 UI(CLI + 指标)| reconcile 直接纠正,默认 5 分钟一轮 | 默认开(suspend 才关)| Git/OCI(desired)|
| Terraform / TFC | plan 输出即 diff;TFC 有 Drift 专属页 | 定期漂移检测 | 不自动;plan → 人决策 → apply | HCL 配置 + state |
| Clash Verge Rev | 设置内只读对话框(Settings → Runtime Config)| 仅展示合并后 YAML 原文,无逐字段 diff | 无 | verge.yaml + profile 管线 |
| EgressAPIKEY(本仓)| 一级「生效配置」视图(ADR-0051,Round 2 落地)| 三态合并快照 + 单程调和 + 一次性托盘通知 | 无自动重放(仅启动恢复)| L2 白盒(desired)vs L3 Resin(live)|

范式分歧(如实记录):GitOps 原则 4「持续调和」与 Terraform 的「触发式 plan/apply」存在张力;ArgoCD selfHeal 默认关、Flux reconcile 默认开——两个工业默认相反。本项目取「触发式 + 用户显式调和」一侧(reconcile_now 单程、白盒必胜、无自动重放),与 ArgoCD 的谨慎默认同侧,并有立法:不引入 K8s 式级别触发调和(Round 3 spec Out of Scope)。

## 5. 覆盖事故教训四则

每则按「标题原文 → 机理 → 对本项目的教训」记录:

1. **CVR #1715**「[BUG] 扩展配置叫Merge功能却是覆盖」(2024-09)\[S4a\]:用户对 Merge 的自然预期是合并,实际是覆盖式增强——命名与语义不符。教训:合并/覆盖语义必须在 UI 与文档里显式传达;「模型存在但未传达」正是本领域一等痛点。
2. **CVR #3395**「[BUG] clash-verge.yaml配置修改」(2025-04)\[S4b\]:用户改的是 app 层文件,重启后被程序按自身权威覆盖——用户把 app 层文件当内核配置层编辑。教训:层归属必须可见;用户在错误的层编辑,配置必然「静默丢失」。
3. **CVR #3256**「[BUG] 自定义规则(全局扩展脚本 Script.js)在应用 profile 时被错误地执行了两遍,导致 duplicate group name error」[A1]:生成管线被重复执行(双管线),同一脚本跑两遍产生重复实体。教训:生成/调和动作必须幂等——幂等不是优化,是防重复实体的事故防线。
4. **v2rayN #6327**「[Bug]: manual changes to config.json are removed by v2rayN」[A2]:用户手改生成的 config.json(内核配置产物),随后被 v2rayN 重新生成时移除。教训:L3 是产物、永远会被下次生成覆盖——「L3 永远可重建、不可手编」正是该事故的架构化表达;用户的编辑必须落在 L2。

## 6. 本项目相对业界的差异化定位(防重复调研)

一句话定位:把 GitOps 的核对纪律(desired/live 三态、版本化回滚、显式调和)带进桌面代理客户端——生态里没有同行做这件事;同时规避 CVR「埋没在设置里」的传达失败(#1715/#3395)与 v2rayN「产物被覆盖」(#6327)的旧坑。

空白对照(截至 2026-09-01 检索):

- 代理客户端生态(CFW/CVR/v2rayN/Hiddify/NekoBox/Mihomo Party)全部停在「生成管线 + 校验门 + 只读展示」;无一家有结构化的 desired-vs-live 三态漂移判定、版本化备份回滚、显式调和动作。最接近的只是只读 Runtime Config 对话框(CVR)与 WebDAV 备份(Mihomo Party)\[S14\]。
- 基础设施侧(GitOps / Terraform)有全套机制,但面向集群与云,不服务单机桌面代理场景。

本项目已落地的先发优势(全部有 ADR / 实现背书):

- **漂移检测**:authoritative_snapshot 三态(consistent / divergent / missingOnResin)+ lastCheckedAt / divergentSince 元数据 + acknowledged 已知豁免(ADR-0051、ADR-0054 §C/§D)。
- **版本化备份**:两份白盒 JSON 写前备份到 backup/(轮转 10 份),backup_list / rollback IPC 复用同一 validate→apply 链,回滚本身也被备份(ADR-0054 §B)。
- **单程调和**:reconcile_now 预演(compute_plan)→ 用户确认 → strategy_apply + restore_ports_from_whitebox → 自动重快照;白盒必胜,无「接受现状」反写(ADR-0054 §A)。
- **传达面**:一级「生效配置」视图 + 一次性托盘漂移通知 + WHY-NOT-EFFECTIVE 排障文档(ADR-0051/0054)。

有意不做(与差异定位同样重要):持续自动自愈 / 级别触发调和(Round 3 spec Out of Scope);跨客户端覆写语法标准化(业界亦无人推动,见 §7 缺口 5);Resin Go 侧改造。

## 7. 原调研遗留信息缺口(照录,供后续调研者接力)

1. 正式术语缺位:未检索到 config authority / layered config authority 的学术或官方术语;最接近的工程化表达是 CVR 源码中受控字段(guarded keys)清单 [S1]。
2. Mihomo Party 覆写引擎内部实现未深挖(仅到 README 层;如需 override 与 CVR merge 的语义对比需再读其 src)\[S14\]。
3. v2rayN「查看生成的核心配置」UI 细节未核验(源码证实生成侧,展示侧待查)\[S13\]。
4. sing-box 1.12/1.13 迁移对 Profile 格式的影响未展开 [S2]。
5. 跨客户端覆写语法不互通:同一订阅换客户端时「覆写要按新客户端重做」[S16];未见任何组织推动统一 override 层规范。

## 8. 来源清单

### 8.1 代理客户端生态(原调研 17 源,编号沿用)

| # | 标题 / URL | 角度 | 日期 | 贡献 |
|---|---|---|---|---|
| S1 | Clash Verge Rev User Guide — <https://clashvergerev.com/en/guide> | Official | 2026 | 四类 profile、Profile Processing Flow、Merge/Script 语义、受控字段警告 |
| S2 | sing-box: Graphical Clients General — <https://sing-box.sagernet.org/clients/general/> | Official | 2026 | Profile 规范化 + GUI 编辑器/查看器义务 = 官方化的 L2/L3 契约 |
| S3 | mihomo: APIs — <https://wiki.metacubex.one/en/api/> | Official | 2026-07 | GET/PATCH/PUT /configs、/rules/disable 临时语义、/restart = 运行时层契约 |
| S4a | CVR Issue #1715 Merge 却是覆盖 — <https://github.com/clash-verge-rev/clash-verge-rev/issues/1715> | Criticism | 2024-09 | Merge 语义反直觉的用户困惑 = 心智模型传达失败证据 |
| S4b | CVR Issue #3395 verge.yaml 被覆盖 — <https://github.com/clash-verge-rev/clash-verge-rev/issues/3395> | Criticism | 2025-04 | 用户混淆 app 层与内核层的真实案例 |
| S5 | hiddify-core Configuration System (DeepWiki) — <https://deepwiki.com/hiddify/hiddify-core/3-configuration-system> | Official | 2025-04 | 多格式解析→统一 sing-box、HiddifyOptions→BuildConfigJson 管线 |
| S6 | hiddify-app Issue #2228 校验 panic — <https://github.com/hiddify/hiddify-app/issues/2228> | Criticism | 2026-06 | CheckConfigOptions 加载前校验的强度与失败语义(整配置被拒)|
| S7 | NekoBox NB4A 配置文档 — <https://matsuridayo.github.io/nb4a-configuration/> | Official | 2026 | 自定义 JSON 混入语法、导出生成的 config.json、GUI 与手写 JSON 互斥 |
| S8 | langlabs.io clash-verge-rev 架构分析 — <https://langlabs.io/clash-verge-rev/clash-verge-rev> | Community | 2026 | IVergeConfig vs IConfigData、enhance() 十步管线、deep merge 行为、draft/apply/save |
| S9 | clash-verge.com 分层文档 — <https://clash-verge.com/en/clash-verge-global-extend-config> | Community | 2026-08 | belong to different layers 表 + Global/Subscription 执行序 + 排障表 |
| S10 | clash-verge-rev 官方仓库 — <https://github.com/clash-verge-rev/clash-verge-rev> | Official | 2026 | 功能声明 + Tauri 架构 + 文档入口 |
| S11 | CFW 配置文件预处理文档 — <https://doc.clashforwindows.app/parser/> | Official | — | Merge/Script 前身 parser 的官方定义(搜索摘要核验,未实抓)|
| S12 | CVR UserGuide 编译本 (doccompiler) — <https://doccompiler.ai/api/v1/jobs/shared/job_1776340060826_3025e165/download/clash-verge-rev__clash-verge-rev__UserGuide.pdf> | Community | 2026-05 | 三文件职责表 + Runtime Config 视图问答(搜索摘要核验,未实抓;临时下载链接,预期失效)|
| S13 | v2rayN CoreConfigHandler.cs 源码 — <https://github.com/2dust/v2rayN/blob/master/v2rayN/ServiceLib/Handler/CoreConfigHandler.cs> | Official | 2026 | GenerateClientConfig 三路分派 + 落盘 + Custom 复制 = v2rayN 管线实证 |
| S14 | mihomo-party-org/clash-party — <https://github.com/mihomo-party-org/clash-party> | Official | 2026 | 覆写(override)/Sub-Store/Smart Core 特性声明 |
| S15 | mihomo wiki 客户端列表 — <https://wiki.metacubex.one/en/startup/client/client> | Official | 2026 | 生态全景(clash-verge 已 unmaintained,CVR/Nyanpasu 等 maintained)|
| S16 | clash-verge.com CVR vs Mihomo Party — <https://clash-verge.com/en/clash-verge-vs-mihomo-party> | Comparative | 2026 | 订阅互通、覆写规则需按各自机制重做的对比结论 |

### 8.2 本票落盘时实抓核验的补充源(2026-09-01)

| # | 标题 / URL | 贡献 |
|---|---|---|
| A1 | CVR Issue #3256 全局扩展脚本被执行两遍 — <https://github.com/clash-verge-rev/clash-verge-rev/issues/3256> | 事故③:生成管线非幂等的真实形态 |
| A2 | v2rayN Issue #6327 手改 config.json 被移除 — <https://github.com/2dust/v2rayN/issues/6327> | 事故④:L3 产物不可手编 |
| R1 | Roxy-WI 官网 — <https://haproxy-wi.org/> | HAProxy GUI 的部署前校验 + 版本历史 + 恢复已知良好配置 |
| R2 | Kubernetes Controllers — <https://kubernetes.io/docs/concepts/architecture/controller/> | desired/current state 与控制环的立法原文 |
| R3 | ArgoCD Automated Sync Policy — <https://argo-cd.readthedocs.io/en/stable/user-guide/auto_sync/> | selfHeal 默认关的官方原文 |
| R4 | Flux Core Concepts — <https://fluxcd.io/flux/concepts/> | reconcile 默认 5 分钟一轮;kubectl edit/patch/delete 会被 promptly reverted |
| R5 | ArgoCD Diffing Customization — <https://argo-cd.readthedocs.io/en/stable/user-guide/diffing/> | desired-vs-live diff 一等公民 |
| R6 | OpenGitOps — <https://opengitops.dev/> | 四原则:Declarative / Versioned and Immutable / Pulled Automatically / Continuously Reconciled |
| R7 | Terraform plan 命令 — <https://developer.hashicorp.com/terraform/cli/commands/plan> | 预演(preview the changes before apply)语义 |
| R8 | Martin Fowler: CQRS — <https://martinfowler.com/bliki/CQRS.html> | 读侧 read model 与 authoritative_snapshot 的对应 |

注:

- S11/S12 沿用原调研「搜索摘要核验」的标注原样保留,本票未实抓;S12 为临时下载链接,预期失效,其结论已与 S1/S9 的源码与文档交叉一致。
- 派发词(handoff/issue)中的「Nodus SHA-256 drift」要点:在原调研留档与 2026-09-01 两组关键词实检(『Nodus config drift SHA-256』/『nodus drift detection』)中均未能定位到对应来源;为避免引用失真未纳入正文,HAProxy GUI 的版本化/回滚亮点由 Roxy-WI [R1] 实抓支撑。详见 .scratch/architecture-recovery/reports/21-research-docs-sedimentation-report.md 的勘误说明。
