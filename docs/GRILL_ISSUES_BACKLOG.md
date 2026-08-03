# GRILL ISSUES BACKLOG (Polishing phase — planning only, execute after all grill questions resolved)

> 状态约定：每条 issue = `[ID] [C-scope] [state] title — 1-line spec`, state ∈ pending(待 grill 确认) / confirmed(grill 已答, 待打磨) / done(已闭环, 引用 commit)。
> C1 = Topology canvas / 路由语义打磨；C2 = 系统性玩具感巡查；C3 = 既有延后项（mainline-B 发布线、ADR-0009 VPS parity 等）。
> **A8 用户约束**：当前在内部打磨期，不发布 v0.1.0；先把所有 grill 问题沉淀成此张表，日后按表统一执行打磨，不边问边改。

---

## C1 — Topology canvas / 路由语义

### C1-1 [confirmed=A10=B-B-3] A/B/C 三列语义对齐用户心智（route_id-derived key identity 在 GUI 显式）
- **用户答复 (Q10/A10=B-B-3 原教旨派)**：B 列 box 全改为 route_id-derived UID，直接连接 shell 拦截器 (P24-A4-3 axum interceptor 在 crates/resin-core/src/interceptor.rs 已注入 X-Resin-Account = ar-<16hex>，算子来自 crates/resin-core/src/lane.rs pub fn route_id)。这是把 P24-A4-3 的拦截器 dogfooded 进 GUI；最贴近用户毫秒级唯一性诉求；也是最重实施路径。
- **现状**：TopologyView 产 P24-R2 重写后三列已是 Entry / Platforms(by region) / NodeGroup(by region)。但 B 列目前渲染 Resin Platform 对象（name + region_filters + routable_node_count），未显式显示 (api_key mask, upstream_endpoint) tuple；route_id FxHash three-tuple + normalize_auth 已 8 个 cargo 测试绿 (lane.rs line 65, 75)，但 route_id-derived UID **没有任何 GUI 渲染点**。interceptor.rs line 95 注入 ar-<16hex> 后，LeaseEntry.account 字段承载此 hash，但 GUI 对用户只显示 hash 而非原 tuple — 不可读。
- **打磨目标 (A10=B-B-3 spec 细化)**：
  1. **B 列每个 box 显式标识一个 (api_key, upstream_endpoint) 元组**，box UID = route_id(normalize_auth(key), body.model, path) -> ar-<16hex> (与拦截器注入的 X-Resin-Account 同源算子)；显示形式 = key[首4位]...key[末4位] endpoint:api.openai.com/v1/chat/completions (key mask + endpoint 全显)。
  2. **平台绑定语义** = box 出现在该 platform 内（沿用 P21-B 双栏拖拽语义：拖 key candidate 到 platform card = attach）。同一 route_id 出现在多个 platform = pipelined lease。
  3. **shell 侧维护已观测 key pool** — 一个 route_id -> (apiKeyMask, endpoint, first_seen_ts) 的反查表。拦截器记录新 route_id 到此 pool，GUI 通过新 IPC observed_keys() 拉取并渲染。pool 初始为空，append-only；拦截器每次见到新 (key, endpoint) 组合 OR 新 body.model 时再 insert。
  4. **GUI 5s sync 已调 ipcLeaseMap() (P24-A4-3)**，把 lease.account (= ar-<16hex>) 与 pool.get(route_id) join，platform card 内 chip 列表显示 apiKeyMask + endpoint + egress_ip (替代当前只显示 hash)。
  5. **No-platform 的孤儿 key pool**：未拖入任何 platform 的 route_id 仍存在 observed pool 里，渲染在 B 列底部一个 Unassigned 区域，用户可拖到 platform 绑定。
  6. **闭环测试**：vitest 构造 mock observed_pool + mock lease_map -> 渲染 box 显示 mask+endpoint 而非 hash；cargo test 加 route_id_idempotent_across_normalize (normalize_auth(Bearer sk-A) == normalize_auth(sk-A) -> 同 route_id)。
- **关联 ADR**：ADR-0002 (canvas three-column), ADR-0003 (key+endpoint route_id, 已在 P24-Q3 corrected 为 ACCEPTED), ADR-0008 (拦截器 e2e 已证明两不同 egress IP)。A10 决策沉淀后需创建 ADR-0011 documenting B-B-3 selection (originalist 派) 及 trade-off。
- **关联 commits**：1aaab6b, f93707c, dfd84b2, 47e54ba, 54b6e64 (P25-Q9), cfc0619 (P21-B platform_create_with_fields IPC, A10 复用此入口).
- **打磨输入**：用户 A4 之前给 build.nvidia.com GLM-5.2 请求头实例 + diegosouzapw/OmniRoute 源码；P24-A3/A4 源码级研究 (docs/RESIN_ROUTING_ARCHITECTURE_RESEARCH.md)。
- **状态**：confirmed (grill Q10 已答 B-B-3), spec 已细化；待 backlog 饱和后统一执行打磨。打磨启动前需先 grill Q11（已观测 key pool 的存储选型 — 见下方 Open Questions）。

### C1-2 [confirmed] 热联线 = 原子事务性 PATCH + 状态刷新闭环
- **现状**：P24-Q2 修了 3 个竞态（patchingRef 重入锁、await sync after PATCH、transparent handle surface），但 ADR-0006 item1 `routable_view` 之后 canvans 状态刷新策略一度单跑 5s 轮询 + visibilitychange refocus；拖拽 PATCH 后的 server-side `routable_node_count` 重算值是否在下次 sync 实际反映，尚未有断言把它写成闭环测试。
- **打磨目标**：加 vitest 断言 "after onConnect -> one PATCH fired -> ipcPlatformListFull re-fetched -> new routable_node_count propagated to canvas node data"；加 "two consecutive PATCH 不会把 region_filters 中间状态变成 stale-snapshot 后再 PATCH 过去" 的闭环。
- **关联 commits**：47e54ba, 67d846c.

### C1-3 [confirmed] 拖拽删除连线 = 移除 region_filters（移除的等幂）
- **现状**：P24-R2 `onEdgesDelete` PATCH updates platform.region_filters 减去被删的 region。但前端 edge-removed 触发是否真的在 server 端把 region 删掉（而不是整段 region_filters 复盖回去），是否有瞬态 "edge 删了 → server 状态没变的竞态" 未闭环。
- **打磨目标**：同一 PATCH 幂等性闭环 + 删除 edge 再 sync 回来 edge 真的不回了。

### C1-4 [pending] 记忆 viewport + 初始 fitView 默认覆盖所有节点
- **现状**：P19 item1 加了 viewport 记忆，P20 item1 修了 "first-paint flash" 用 onInit+opacity gate；逻辑应该是闭环了，但用户在 P25-Q7 之后再提及 "拓扑层会跟默认初始位置抢" — 这条需要 grill 确认用户现在还看得到最初的 flash 还是只是历史描述。
- **打磨目标**：如果用户还能在当前 release exe 看到.flash 抖动，重看 onInit 时机；否则这条 done。
- **关联 commits**：0cb4186, 3cfb1a7.

---

## C2 — 系统性玩具感巡查（后端与 GUI 状态机一致性）

### C2-1 [confirmed] Subscriptions 拖拽排序是真实持久、切面回留不丢
- **现状**：P19 item6 修了本地 localOrder persistence；P20 item6 把 HTML5 DnD 换成 Pointer Events（WebView2 在 onDragStart 不设 effectAllowed 会禁止符号）；localSubOrder 存本地。但用户后来在 P25打磨期又报 "导入的订阅依旧无法排序"，怀疑是某次 release exe stale-bundle 让用户测错版本——需要用当前 staged exe (allocator 后的 hash) 复核。
- **打磨目标**：闭环 vitest 跑 "drag row A above row B → saveSubOrder persisted → remount view → localOrder 回留"。手工在当前 release exe 复测一次。

### C2-2 [confirmed] Subscriptions 重命名 / 删除共存
- **现状**：P19 item6 加了 handleRename (删旧的 + 用新名重建)；P20 item3 加重名检测防止覆盖；P22-A1 加了提交 toast i18n。
- **打磨目标**：闭环确认 "rename 名字冲突期不覆盖原 sub"，"删除 -> list 重 fetch -> UI 同步减少一行"；加 e2e assertion。

### C2-3 [confirmed] keyCandidates 跨视图不丢（平台双栏）
- **现状**：P21-B platform 双栏重构：左 = 候选 key combinations，右 = live platforms。candidates 持久化在 `settings.json#keyCandidates`。但 P22 之前用户在 Q9 cluster 也说过 "重开软件配置文件没固化"，同 principle 要确认 keyCandidates 在 app restart 后还在。
- **打磨目标**：vitest + integration assertion "loadKeyCandidates() returns same set after setKeyCandidates()".

### C2-4 [confirmed] 平台双栏拖拽外观区分（独立 key 组 vs 手动平台）
- **现状**：P21-B 实现了 `auto-{uid}` platforms 用实线卡片，手动 platform 虚线卡片；Pointer Events drag。但用户在打磨期可能还看到外观界限不够 — 由 C2 系统巡查时复核。
- **打磨目标**：用户实测如果觉得实线/虚线区分不够明显 → 加颜色区分；如果已足够 → done。

### C2-5 [confirmed] Tray i18n 实时同步
- **现状**：P24-A4-3 修 await 顺序 (changeLanguage → saveLocale → tray_refresh_labels)；P25-item4 加 tracing + 2 vitest。用户在 P25打磨期报 "慢半拍"，但 root cause audit 结论是 "测试 stale exe" — 应该在当前 staged release exe 复核。
- **打磨目标**：在当前 release exe 手动实测 "Settings 切换语言 → 立即右键托盘 → 菜单文字是当前 locale 不是前一 locale"。

### C2-6 [confirmed] ProcessRoute 增加用户可点击的冲突解决路径
- **现状**：P18 加了 IPC lane 冲突检测（process_route_conflict_check），右弹 toast "conflict"。但用户面对冲突没操作路径（只能手动改 lane）。
- **打磨目标**：toast 旁加 "查看冲突源" 按钮 -> 跳转到 lane 占用 view（可视化 lane 0 已被哪些进程占用）。或者 lane conflict 时改 dropdown 只显示 available lanes。
- **交付需求**：高质量 UX，不是再加个 toast。

### C2-14 [confirmed=b] PlatformsView 右栏补 "新建平台" 手动创建入口（Q9 A9 选定 b 方案）
- **现状**：P21-B 双栏重构后右栏顶端没有显式 "新建 Resin 平台" 按钮，用户只能通过拖拽 key candidate 到右栏空白处来隐式创建 `auto-{uid}` platform；显式创建带定制 name/allocation_policy/regex_filters 的手动平台没有 GUI 入口。IPC `ipcPlatformCreateWithFields(body)` 已在 P21-B commit `cfc0619` 落地但目前前端未桥接。
- **打磨目标（A9 确认 b 方案）**：
  1. 右栏顶端 + 右栏上下文 toolbar 加 `t("platform.create")` 按钮。
  2. 点击 → 弹 `<dialog>` (Tauri webview 不用 native modal，用 inline dialog div 满足可 inspectability)，表单字段：
     - `name` text input (1..128 chars, assertShortName 校验)
     - `allocation_policy` `<select>` 三个OPTION: `BALANCED` / `PREFER_LOW_LATENCY` / `PREFER_IDLE_IP` (与 IPC `ALLOWED_ALLOCATION_POLICIES` 同源)
     - `regex_filters` `<textarea>` 每行一个正则 (max 64 行，每行 ≤253 chars — 与 `platform_update` IPC 校验同源)
     - 重置按钮 + 提交按钮；提交按钮 disabled 状态由 dirty + 表单校验结果驱动。
  3. 提交 → `await ipcPlatformCreateWithFields(body)` → `await refreshPlatforms()` → dialog 关闭 + toast `platform.addOk` (已有 i18n key)。
  4. 失败路径：`ResinClient::create_platform_with_fields` 已有 unit test `cfc0619`；IPC 失败时 toast 显示 Resin 返回的错误 body excerpt；dialog 不关，让用户改了再提交。
  5. **闭环 vitest**: `PlatformsView.test.tsx` 新加 "user opens dialog → fills valid name 'OpenAI-Prod', POLICY=PREFER_LOW_LATENCY, 2 regex filters → submit → asserts `ipcPlatformCreateWithFields` 被调一次 with the right body → asserts `refreshPlatforms` 被调用 → resolves → dialog closes";另加一例 "submit with empty name → dialog stays open, `ipcPlatformCreateWithFields` NOT called"。
  6. **i18n**: 新 keys `platform.create` / `platform.createDialog.nameLabel` / `platform.createDialog.allocationPolicy` / `platform.createDialog.regexFilters` / `platform.createDialog.reset` / `platform.createDialog.submit` / `platform.createDialog.invalidName` / `platform.createDialog.invalidRegex` 加入18 个 base locales (按 AGENTS §3 lockstep)。
  7. **Keyboard accessibility**: dialog 按 Esc 关闭、焦点 trap 在 dialog 内、提交成功后焦点返回 "新建平台" 按钮 (AGENTS frontend guidance "feature-complete controls, states, and views").
- **验收准则**: 在当前 release exe 手工实测：(a) 打开 PlatformsView 右栏顶端看到 "新建平台" 按钮。(b) 提交一个 `name=OpenAI-Prod` `POLICY=BALANCED` 无 filters → toast addOk → 右栏列表多一行 OpenAI-Prod → Resin 后端 GET /platforms 看见此平台。(c) 提交空名 → dialog 不关，无 IPC 调用。(d) Resin 返回 400 时 dialog 不关，错误内容显示 toast。
- **关联 commits**: 4625b4a (P21-B 双栏), cfc0619 (platform_create_with_fields IPC), 10a7776/d459de0 (toast i18n 在 SettingsView / tray)。
- **状态**: confirmed（grill Q9 已答 b），待 backlog 饱和后统一执行打磨。

### C2-7 [confirmed] Settings 切换 locale 后整页节点框文案 i18n（画布节点盒子 en→zh）
- **现状**：P13 B7 filed 后修了；但用户后来反馈 "刚打开 GUI 是 zh，画布内节点框还是 en，切换界面再回来才刷新"。怀疑 TopologyView mount 时 i18n.ready 状态未等就 build 节点 box。
- **打磨目标**：TopologyView 用 `useTranslation` hook 改 `i18n.ready && i18n.language` gate，画布数据在 i18n ready 后才有 text；闭环 vitest "构造 locale=en, render, box text is en; change locale to zh, re-render, box text is zh"。

### C2-8 [confirmed] Settings 修改后统一保存按钮 + 按钮文案
- **现状**：P9-P10 far fix per-card save bar 去掉了，换 unified sticky save bar；但用户在打磨期有进一步诉求 "修改后立刻显示统一保存按钮"。
- **打磨目标**：表单 dirty state 驱动 sticky save bar 显示 (clean = hidden, dirty = visible + disabled=false, saving = visible + spinner)。企级轮子模板参考 formik / react-hook-form dirty field tracking。
- **关联**：用户在多条 bug 列表里要求 "请 exa 联网调研企业级轮子模板"。

### C2-9 [pending] log 系统成熟化（合规 / 限长 / 爆栈防护 / 用户可查）
- **现状**：tauri-plugin-tracing 已 daily + 10MB + keep 7；用户可见性只在 Settings > Open log directory 按钮。用户若干表达式 "我要的成熟 log 系统能给以上 bug 定位问题"。
- **打磨目标**：调查企业级 Rust tracing 轮子（tracing-loki / tracing-opentelemetry / tracing-gelf），调研后选一个且只在应用内附加 bounded channel + non-blocking writer，避免 tracing backpressure 阻塞 axum event loop。GUI 可加一个内置 "查看最近 N 条 tracing log" 的 read-only 面板。
- **关联**：用户要求 "使用 1mcp pwm/exa 联网调研企业级轮子模板"。

### C2-10 [pending] 路径防护 / 越权修改 全面巡查
- **现 audit**：P14 backup_upload path traversal 修复在 mod.rs:792-798 仍存在。
- **打磨目标**：扩展巡查所有 `#[tauri::command]` 在 `src-tauri/src/commands/mod.rs` 中拿到 PathBuf 的入口（不只是 backup_upload），看是否有 path 不被 canonicalize+starts_with 约束的。配置 export/import 走 JSON 内容 + 256KB cap (P18) 但 zip 路径仅 backup 一处。
- **注意**：这是完整性检查，不是新功能。

### C2-11 [pending] 环境变量 / OS 默认副作用防护
- **A9-0009** CLI stub 不动环境变量、不 spawn 网络；A7 user 约束 "防止勿操作滥用 cli 导致环境变量崩溃"——这条目前 CLI 二进制只读 `--config` 这个 opt-in path，没有 env var 读写面，已天然安全。
- **打磨目标**：audit cargo build 被 build.rs 的 env::var 调用面（tauri-build / build.rs 中 `env!("...")` 的 macro_expansion 行为），写一行 AGENTS 注释 "哪些 env var 会被消费，哪些不会"；不是改代码,是文档 implant。

### C2-12 [pending] 第二实例防护 + focus 主窗
- **现状**：single-instance plugin 保留，第一实例被吃后第二个 chain-exits 0；但用户打磨期报过 "左键点击托盘图标会有右键那个框闪现一下出现又消失"。
- **打磨目标**：audit tray.rs 的 left click handler 是否误解为 popup_open 逻辑；如果存在就硬修闭环。但又关联 single-instance 左键点击是否唤醒已开启窗口而未 focus — 需 grill 确认 "点击第二实例 .exe 应该把第一实例的窗口拉到前景" 这条需求具体怎么算。

### C2-13 [pending] 关 GUI 时托盘 icon 保留，不直接退出
- **现状**：P2-orphan kill (P22-audit) + tray quit item；close 到 tray 是 default 在 tauri.conf.json。但用户打磨期报过 "关闭 GUI 后软件直接退出，应该保留托盘图标"。这已修过（P15 以前）— 但需在当前 release exe 复核是否真保留。
- **打磨目标**：实测关闭主窗 -> process 仍存活 + 托盘 click 重开。

---

## C3 — 延后项（不属本轮打磨执行）

### C3-1 [confirmed] mainline B 发布流水线状态
- ADR-0010 落地（local/gui + CI workflow_dispatch）。用户 A8 不发布只打磨。A8 后这条 on-hold。

### C3-2 [confirmed] ADR-0001 VPS path (CLI/GUI parity)
- ADR-0009 记录 deferred；当前打磨期不开发 VPS headless parity。等用户切到 VPS phase 时走 ADR-0009 三个调查过的轮子（dualkit / fnrpc / helmor dispatcher）中的选型。

---

## 用户 P25打磨期 listed 但尚未 grill 过的爆发 bug（需要塞进此表并 grill）

### P25burst-1 [pending] 重置排序按钮 click → 多了之前的订阅链
- 用户原话 "点击重置排序之后，多了好多之前的订阅链接"。P20 item4 修过 `handleResetOrder` 直接 setLive(serverList) 不走 **applyOrder**；但用户之后可能又报。需要 grill 确认是 release exe stale 还是代码仍坏。
- **打磨目标**：复核当前 release exe；如仍坏 -> 复现 -> fix + e2e 闭环。

### P25burst-2 [pending] 节点池总节点数增长但下方表格内容不变
- 用户 "总节点数 295 vs 表格空白"。P20 item5 已把 ResinClient.list_nodes 改成 /nodes?limit=500；但用户后途经会再看到 — 复核当前 release exe。

### P25burst-3 [pending] 导入订阅 -> 0 nodes（clash UA + flow->block 已落地）
- P13 B4 已修，闭环 "distinct_keys_use_distinct_lanes_contract"。用户打磨期再报 "已将 0 个节点" — 需要 grill 用户测的是不是当前 release exe (Allocator 后的最新 hash)。

### P25burst-4 [pending] 平台添加 OK → 但没显示
- P13 B6 已修 `items_arr` 接 items-wrapper；P22 README 提及；但用户在 P25打磨期再报。需当前 release exe 复核。

---

## 后续 grill question 映射
- Q9 → C1-4 (viewport flash) + C2-1 (sub drag current state)
- Q10 → C2-6 (processRoute conflict UX resolution path) 或 C2-8 (Settings dirty state save bar)
- Q11 → P25burst-1/2/3/4 (当前 release exe 复测)
- Q12 → C2-9 (log system 轮子调研)
- Q13 → C3-2 VPS phase 启动决策（A8 之后）
... 后续每次 grill 答复后写 new issues 入表。

---

## 编号约定
backlog ID = cluster letter（C1/C2/C3）+ 序号 或 P25burst-N。grill 答复后改 state pending→confirmed 并把用户答案 1 句话点出。done state 用 commit hash 引证。


---

## Q8 证伪重测结论（A8=P8-1 evidence-based audit, 2026-08-03）

四条 P25burst 证伪过程：源码级复核 + 现场对 live subscription URL 的 per-UA HTTP 验证，非基于 commit log 假设。

### P25burst-1 — 重置排序后多余旧订阅行
- **证伪结论**：DONE。源码 `src/views/SubscriptionsView.tsx` `handleResetOrder` 走 bypass 路径 — `setLocalOrder([]); saveSubOrder([]); setLive(serverList)`，serverList 由 `ipcSubscriptionList()` 获取最新，不经过 `applyOrder`，因此无 stale-name ghost 行。代码与 P20 item4 commit `3cfb1a7` 状态一致。
- **引证 commit**：3cfb1a7 (P20 item 4 root-cause fix)
- **backlog**：移除这条 P25burst-1；C2-1 (sub drag) 独立闭环。

### P25burst-2 — 节点池统计增长但下方表格不变
- **证伪结论**：DONE。源码 `crates/resin-core/src/resin_client.rs` `list_nodes` 走 `/nodes?limit=500`，NodesView 统计端 `pool?.total_nodes ?? nodes.length` 两端对齐 500。代码与 P20 item5 commit `3cfb1a7` 一致。
- **引证 commit**：3cfb1a7 (P20 item 5)
- **backlog**：移除 P25burst-2。

### P25burst-3 — 导入订阅 -> 0 nodes（clash UA + flow->block 已落地）
- **半证伪 + 现场证据推翻部分 root cause audit**：
  - `crates/resin-core/src/resin_client.rs` `fetch_clash_subscription` UA pool = [`clash-verge/v2.0.0`, …] (4 个 clash family UA)，后 `clash_yaml_to_proxies_block` 转换为 block-style，local content POST 到 Resin (30s tick)。代码与 P13 B4 一致。
  - **现场对用户原 URL `https://link123.52pokemon66.cc/...token=79e1...`** 每 1 个 clash UA 做 live fetch，**7 个 UA 全部返回 403 Forbidden** —— 原审计结论 "默认 UA 403，clash UA 200" **在此 URL 现场阶段不成立**，端点本身已拒绝所有 UA（token 失效或上游被下架），并非 UA 问题，也非代码 bug。
  - **对照新 URL `https://jinxi2410.qzz.io/cfnew2/sub?target=clash` 每个 UA 都返回 200，80KB 真实 Clash YAML**（含 `proxies:` 块共 116 项 block-style 节点），证明 `fetch_clash_subscription` 代码路径在新生成 URL 下确实获取正文。`clash_yaml_to_proxies_block` tests 3/3 pass。
- **证伪结论**：DONE。代码本身没坏；用户报"已将 0 个节点" 是因为旧测试 URL token 已失效 (403)。修复路径扣押不需要。
- **引证 commit**：（旧 P13 B4 fixed path） + 现场对照证据 (新 URL 200/80KB) 
- **backlog**：移除 P25burst-3。
- **需用户操作**：之后重新测试用 `https://jinxi2410.qzz.io/cfnew2/sub?target=clash`，不要再用已过期的 52pokemon66 URL。

### P25burst-4 — 平台添加 OK 但不显示 [→ 升级 confirmed bug, 进入 backlog C2-14]
- **根因 audit 结论被推翻**：原 audit 把这条归入 `items_arr` 解析修完的 "false alarm"，但现场代码读取结果不一样。
- **现场源码状态**：`src/views/PlatformsView.tsx` 经 P21-B 双栏重构 (commit `4625b4a`)，当前 "Add Platform" 按钮实际指 `t("platform.addKey")` —— 它在 left pane 添加 key candidate (不调 Resin POST /platforms)。看起来"加完了 addOk 没显示平台"，是因为按钮名字用 "platform.add" 误导用户认为是添加 Resin 平台，实际功能 = 添加 key 候选到 left pane。
- **证伪结论**：FAILED → **升级为 confirmed bug C2-14**。
- **打磨目标**：将按钮文案/语义与前进功能对齐：要么按钮文案明确 "添加 key 候选（稍后手动拖到右侧建平台）"，要么同时补 Resin 平台创建入口（按钮 + name + filters input dialog）。grill Q9 需用户决定走哪种 UX 修复方向。

### 证伪检查完发现的额外 issue（不是 P25burst 原 4 条）
- **P25-Q8-extra-1 [pending]**：用户原测试 URL token 已失效（403）。建议在 `fetch_clash_subscription` 错误消息里将 403 上的"UA 重试用尽" 与"端点可能已下线/token 失效" 进 layer 分级，toast 用户可看到 `the subscription URL may have expired (HTTP 403 from all UAs)` 而非当前的统一 "0 nodes imported"。这对打磨期 + 发布期都是用户友好性提升。

---

## Open Questions (pending grill)

### Q11 — 已观测 key pool 的存储选型（A10=B-B-3 spec 阻塞前置决策）
A10 决议 shell 侧维护一个 route_id -> (apiKeyMask, endpoint, first_seen_ts) 的 append-only 反查表；route_id 是确定性 FxHash (lane.rs pub fn route_id, 8 个 cargo tests 已证明 idempotent+distinct)，意味着同一 (key, endpoint, model, path) 元组永远算出同一 ar-<16hex>，拦截器见的同一 tuple 不增条目。
但反向映射 (route_id -> readable tuple) 必须持久在某处，重启后才能立刻渲染历史观测；否则拦截器要等一次新请求流过才能重建 pool，GUI 首次打开拓扑会显示空白 B 列。

> **Q11 答复决定 C1-1 的依赖存储实施方式，且影响 IPC 表面与跨会话连续性。三种选型已用 pwm ask 调研 (Perplexity pro 184/300):**

1. **拦截器 in-memory HashMap + Tauri event push** (轻)
   - 优：零持久化代码；拦截器每次新 tuple 直接 emit `tauri::Emitter::emit("observed-key", {route_id, mask, endpoint})`，前端 listen 后直接 setState。
   - 劣：重启 app 进程则 pool 丢失，必须等一次流过才重建 — 用户开 GUI 的瞬间拓扑 B 列 空白，需要重新走流量才能看到 box。Ponytail 最小 diff 符合但 UX 不是最贴近"毫秒级唯一性"诉求。
   - 已知论点反驳：route_id 确定性意味着条目可在拦截器每次请求时校验 HashMap.entry().or_insert() 重建，所以"空"的窗口极短 (单请求后即有)；但首屏空白不可避免。
2. **settings.json#observedKeys 持久化数组** via tauri-plugin-store (中)
   - 优：复用现成 store infra (P21-B 的 keyCandidates 已用此模式)，重启保留；IPC observed_keys() 读 store 即可。
   - 劣：append 场景下每次新条目都要读写整个 JSON；无 query index；pool 膨胀需 cap (建议 max 1024 entries LRU)；serde_json round-trip 开销随条目数线性升。
   - 推荐用法：和 keyCandidates 同模式，settings.json 顶层 `observedKeys` 数组，每条 `{route_id, apiKeyMask, endpoint, first_seen_ts, last_seen_ts, request_count}`。
3. **新建 SQLite 表 observed_keys(route_id PRIMARY KEY, apiKeyMask, endpoint, first_seen, last_seen, request_count)** via rusqlite (重)
   - 优：canonical 存储路径 (pwm 调研明确推荐)，WAL mode 并发；query 反查高效；append-mostly 表现佳；restart safe。
   - 劣：AGENTS §Storage locations 当前写 "Database: none yet. rusqlite is a Cargo dependency but is NOT instantiated"，选 c 意味着项目**第一次落地真正 SQLite 实例**，需要 migration boilerplate + Connection lifecycle 在 main.rs setup 管理。
   - 重点 Ponialtail 推论：这是项目第一次开 DB，所以工作量是"首次落地 rusqlite migration 流程" + "Connection 池在 Tauri State" + "schema versioning" — 不是 c 个人选项的 cost，是首次落地 DB 这件事的 cost。

**Q11 问题**：A10=B-B-3 选定了 B 列 box 语义，但已观测 key pool 该存哪？(a) 纯内存 HashMap + event push (重启丢失，首屏空白) / (b) settings.json#observedKeys 数组 (中量，但 JSON round-trip 膨胀) / (c) 新建 SQLite observed_keys 表 (重，但项目首次落地 DB，注释 AGENTS 现有 storage 节)
理由应该是 (1) 未来会不会上 VPS headless parity，c 的 SQLite 也直接复用；(2) pool 膨胀到 1024+ 之后 b 的 JSON 读写成本 — Ponytail 应否把"最小可工作"放第一位；
答 Q11 决定 C1-1 在执行打磨期是用 in-memory 模式 + 用户自认首屏空白，还是 settings.json 模式 + 简化但需 cap，还是 SQLite 模式 + 首次落地的工程化成本。

--- (grill 状态：Q11 已抛出，等用户回归答；A8 规划期禁止边答边改)
---

## Q11 答复与决策 (A8 规划期沉淀)

### A11 = (c) 新建 SQLite observed_keys 表
- **用户答复 (Q11/A11)**：(c)，"需要工程化，所以必须选最佳工程化的方案"。
- **决策**：C1-1 打磨期实施 "已观测 key pool" 使用新建 SQLite 表 `observed_keys(route_id PRIMARY KEY, apiKeyMask, endpoint, first_seen, last_seen, request_count)` via rusqlite。这是项目**首次落地真正 SQLite 实例**。
- **连锁影响（落入 backlog 待执行）**：
  1. **C1-1 spec 补充**：route_id -> readable tuple 反查 JOIN 走 SQLite SELECT；拦截器每次新 tuple 走 INSERT OR IGNORE；GUI 5s sync 拉 SELECT * 然后与 ipcLeaseMap() join。
  2. **AGENTS §Storage locations 更新**（打磨期落地时同步改）：将 "Database: none yet. rusqlite is a Cargo dependency but is NOT instantiated" 改为 "Database: observed_keys SQLite table at app_config_dir()/ai-api-route.db (WAL mode)。rusqlite 现已通过 DbPool 在 main.rs setup 实例化。Schema migration via <TBD>。"
  3. **依赖与 lifecycle**：rusqlite 已在 Cargo.toml（resin-core），但首次需要：DbPool 抽象（r2d2-rusqlite 或手写 Mutex<Connection>）、schema migration 流程（refinery / rusqlite_migration / 手写 user_version PRAGMA）、Connection 在 Tauri State 注入。
  4. **跨进程边界**：拦截器 (axum task in tauri::async_runtime) 与 Tauri IPC handler 同进程多线程访问 SQLite — 需要 Mutex 或 r2d2 池；Resin Go sidecar 自己有 state.db 不与此 DB 冲突（不同文件）。
  5. **闭环测试要求**：cargo test 覆盖 (a) INSERT OR IGNORE 幂等 (b) route_id 反查命中 (c) schema migration 幂等 (d) рестарт app 进程后 pool 仍可读。vitest mock observed_keys IPC 返回值，渲染 box 显示 mask+endpoint 而非 hash。
  6. **回滚预案**：若 c 方案工程化时意外卡死，回退到 (b) settings.json 模式 — 所以实施前先在分支验证 DbPool + migration boilerplate 端到端跑通再 merge。
- **状态**：confirmed (grill Q11 已答 c)，spec 已落地；待 backlog 饱和后统一执行打磨。打磨启动前需先 grill Q12（SQLite 实例所有权与 schema migration 方案 — 见下方 Open Questions Q12）。

---

## Open Questions (pending grill) — 更新

### Q12 — SQLite 实例所有权 + schema migration 方案（A11=C 阻塞前置决策）
A11=C 选定新建 SQLite observed_keys 表。但项目首次落地 SQLite 有三个工程化决策点必须 grill 才能进 C1-1 打磨:

> **Q12 是复合问题，三个子决策可一次答复也可分次。**

1. **Connection lifecycle 方案** — 单进程多线程 (拦截器 axum task + Tauri IPC handler + GUI 5s sync polling) 都要访问同一 SQLite。选型:
   - (a) `Mutex<Connection>` 单连接 + tokio::task::spawn_blocking 包装（最简，但所有 DB 操作串行化 — Ponytail 最小可工作）。
   - (b) `r2d2` + `r2d2_rusqlite` 池（多连接并发，但加一个新 crate dep）。
   - (c) 每次 IPC 打开短期 Connection（无池，但频繁 open/close overhead）。
2. **Schema migration 方案** — 首次落地 DB 后表结构演进怎么办:
   - (a) 手写 `PRAGMA user_version` + 启动时 `if user_version < N { exec migration_N; user_version = N }`（最简，无新 dep）。
   - (b) `rusqlite_migration` crate（轻量，专为 rusqlite 设计，[creator维护](https://github.com/cljoly/rusqlite_migration)）。
   - (c) `refinery` crate（重量，跨多 DB backend,对 rusqlite 映射较弱）。
3. **DB 文件位置 + WAL mode** — AGENTS §Storage locations 当前写 "db should use app_config_dir() as parent so it sits beside settings.json"。
   - 是否启用 WAL (`PRAGMA journal_mode=WAL`) 以提升并发读？
   - 是否定期 vacuum 限制 db 文件膨胀？
   - 备份策略（与 settings.json backup 复用还是单独 db backup 入 backup/）？

**Q12 问题**：A11=C 已锁定 SQLite，请答 (1) Connection lifecycle (Mutex/r2d2/per-call) (2) migration 方案 (user_version/rusqlite_migration/refinery) (3) WAL + 位置 (启用 WAL/AppData/复用 backup 流程)。
理由应该是：(1) 实际并发强度 — 拦截器每个新 tuple 一条 INSERT，GUI 5s sync 一条 SELECT，并发极低，Mutex 大概率够用；(2) Ponytail 最小加 dep 倾向 user_version 手写；(3) WAL 几乎零代价就该启用。
答 Q12 后 C1-1 的打磨实施成本就能精确估算，可启动打磨期。

--- (grill 状态：Q12 已抛出，等用户回归答；A8 规划期禁止边答边改)
