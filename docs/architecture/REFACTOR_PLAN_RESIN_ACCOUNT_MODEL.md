# P21 — Resin Account / Platform 模型映射（架构基线）

> 来源：Resin DESIGN.md (master 分支, v1.1.2/v1.2.0) + ai-api-route IPC 现状。
> 用途：定义"平台 / api key 组合 / ip 通道"三者在 Resin 真实后端里的对应实体，
>  消除"GUI 像玩具 vs 真实功能闭环"的落差。Milestone B/C 落地的 single source of truth。

## 真实概念映射

| 用户心智模型            | Resin 真实实体              | 真实 API 路径                                       | 持久化位置                |
|------------------------|----------------------------|-----------------------------------------------------|---------------------------|
| 平台 (Platform)        | Resin Platform             | POST/GET/PATCH/DELETE /api/v1/platforms             | state.db platforms 表     |
| api key 组合           | Resin Account（路径段字符串）| 无独立 POST — 由代理流量产生 lease 自动出现          | cache.db leases 表        |
| key 的唯一标识         | sha1(v1_endpoint + api_key)[:8]（shell 侧 UID） | 无 Resin 原生 UID — shell UI 生成展示用 UID    | 仅 UI 展示，不持久化后端  |
| ip/ip 通道             | Resin Node (subscription)  | GET /api/v1/nodes、GET /metrics/snapshots/node-pool  | cache.db nodes_static/dynamic |
| 平台内 key→IP 出口绑定 | Resin Lease                | GET /platforms/{id}/leases                          | cache.db leases 表 (platform_id, account, node_hash, egress_ip) |
| ip 出口策略            | Platform allocation_policy | PATCH /platforms/{id} body.allocation_policy       | state.db platforms 表     |

## 关键现实（避免玩具 UI）

**Account 没有"创建"端点。** Resin 的 Account = 一次代理流量的 `Platform.Account` 身份字符串，
触发 lease 后自动出现在 `GET /platforms/{id}/leases` 列表里。
"把 api key 拖到平台"在真实后端语义里 = **主动触发一次流量** 让 Resin 为 (platform, account_sha1)
建立 lease 并在此平台上选择一个出口节点。
- 我们不能"凭空创建 account"；只能 (a) 在本地 shell DB 登记已知的 (v1_endpoint, api_key) 组合作为"候选 left 栏"；
  (b) 拖到平台的时候，shell 主动以 `${platform}.${uid}` 身份发一次探测请求让 Resin 建立 lease。
- 不放到平台的 key 组合在后端**永远不会产生出口 IP** —— 对应 Resin 的 `reverse_proxy_miss_action=REJECT`：
  未识别身份的请求直接拒绝，没有 egress。这正是用户要的"默认禁止端口出口"防误操作语义。

## 平台左右双栏交互（Milestone B 真实落地）

**左栏（候选 key 组合）**：来自 `gateway_request_log`（shell Tauri 端打开代理审计）或本地手动录入。每条 key 组合：
- 唯一标识 UID = `sha1(v1_endpoint + "::" + api_key).slice(0, 8)`（shell TS 侧生成，仅展示用）
- 展示字段：UID + endpoint host + key 掩码（前4后4）
- 持久化：写入 `settings.json#keyCandidates` （shell 本地，不上 Resin 后端）

**右栏（已激活的平台 + 已绑定的 account）**：来自 Resin `GET /platforms`（已实现 IPC `ipcPlatformListFull`）
+ `GET /platforms/{id}/leases`（新 IPC `ipcPlatformLeases`）。

**拖拽语义**：
1. 左栏 key 拖到右栏空白 = 创建一个"独立平台"，名字 = `auto-{uid}`，POST `/platforms`，并触发一次 `${platform}.${uid}` 探测建立 lease
2. 左栏 key 拖到右栏已有平台 = 在该平台 PATCH `allocation_policy` 保持，触发 `${favoritePlatformName}.${uid}` 探测 → lease 出现在该平台下
3. 独立平台（带 Default 后缀的 auto-*）不能再拖入别的独立 account — 拖入即与已有平台合并（重命名 + 删除旧 auto + lease 迁移 via inherit-lease action）
4. 防覆盖：左栏 `(endpoint, api_key)` 唯一去重；重名 UID 拒绝录入

**中栏宽度调节**：用 CSS grid + 一个 `<div class="resizer" onpointerdown...>` 手柄（Pointer Events 同款），不引入 npm 包。左右宽度持久化到 `settings.json#splitRatio`。

## IP 通道 GUI + 出口策略（Milestone C 真实落地）

- "IP 通道" = Resin Node 分组视图（按 region 分组），数据源 `GET /api/v1/nodes` 已实现 `ipcNodeList`。
- "策略" = platform.allocation_policy 唯一存在的真实后端机制：BALANCED / PREFER_LOW_LATENCY / PREFER_IDLE_IP。
- GUI 出口策略选择器（随机/顺序/延时/带宽/质量）：映射到上面三个 Resin policy（其它的内容如"顺序"映射到 BALANCED，"延时"映射到 PREFER_LOW_LATENCY，"质量"映射到 PREFER_IDLE_IP），映射关系持久化到 `settings.json#ipChannelPolicyMap` 并 PATCH 到 platform。
- 节点协议权重（SSE/WebSocket 影响）：从 `docs/research/PROTOCOL_WEIGHT_RESEARCH.md` 读 docs 数据，GUI 展示权重标注，不进 runtime injection（Resin 没有此 API）。
- 白盒配置文件：`config_export/import` 已实现（R4），GUI 上把该能力绑到 Settings 页"导出/导入"。

## 闭环 test 纲要

- vitest: TS 侧 UID generator hash 唯一性 + endpoint+key 不窜位
- vitest: 拖拽左→右空白 = 触发 `platform_add` + `gateway_select_account` 模拟探测
- vitest: 重复 UID 拒绝录入
- vitest: platform 删除调用 `platform_remove` + 双栏再渲染
- vitest: 平台内 account 排序拖拽（Pointer Events）
- vitest: ip 通道策略选择器 → Resin policy 名映射断言
- mockito (Rust): `ResinClient::platform_leases` `GET /platforms/{id}/leases` happy path + items-wrapper parse
- mockito: `ResinClient::create_platform_with_fields` full schema POST 含 allocation_policy / regex_filters

