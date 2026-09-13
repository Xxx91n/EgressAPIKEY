# Resin Platform Schema 字段映射表（round5 T18 / 裂痕 #8）

> 回答的问题：**前端同学要加或改一个 platform 字段时，该改白盒还是改 ResinClient body 形状？**
> 本表把同一字段在五层的落位对齐成一行；上游权威为 `resin/DESIGN.md` §「Platform 模型」
> （L1183-1300，创建与更新的字段要求同节）。基准版本 **v1.2.0**（`docs/RESIN_UPSTREAM_MANIFEST.yaml`）；
> 端点消费面见 [RESIN_API_COVERAGE.md](../architecture/RESIN_API_COVERAGE.md) #R07/#R08/#R11。
> 白盒写入口立法 ADR-0036（本表只做映射，不改字段与写路径）；人工维护、不做自动同步（spec §6）。

## 改动分流（先看这里）

| 想改什么 | 改哪一层 | 落点 |
| --- | --- | --- |
| 哪些节点或出口 IP 进平台（A 类策略意图） | 白盒 `StrategyConfig.PlatformStrategy`（L2） | `strategy_config_put` / `strategy_platform_regions_set` 写白盒，`strategy_apply` 派生并 PATCH Resin `region_filters` |
| 平台在 Resin 侧的代理行为（粘性、反代、熔断、分配策略） | 仅 ResinClient body（白盒不存这些字段） | `platform_update`（PATCH）或 `platform_create_with_fields`（POST） |
| 分配策略的用户可选项（B 类，UI 六选一） | shell `StrategyId` 经 TS 唯一映射点 | `src/lib/strategy.ts` `strategyToResinPolicy`（六对三）后走 `platform_update{allocation_policy}` |

## 五列对齐全字段表（Resin v1.2.0 全 12 字段）

可写 9 字段 + 只读 3 字段，每格非空（无对应物的格子写明「无」与原因）。

| Resin DESIGN 字段 | 白盒 `PlatformStrategy` 字段 | IPC 入参 / TS wrapper | 前端 form | 备注（类型 / 必填 / 默认 / 范围） |
| --- | --- | --- | --- | --- |
| `name` | `platform_name` | `platform_add{name}`、`platform_create_with_fields{body.name}`、`platform_update{name}`；TS `assertShortName`（1..128，无控制符） | PlatformsView 创建对话框 name 输入框（`platform-create-name`）与平台卡标题 | create 必填；全局唯一；保留名 `Default` 与 `api`（大小写不敏感）；禁特殊字符集（`.`、`:`、竖线、`/`、反斜杠、`@`、`?`、`#`、`%`、`~`）与空白；shell 1..128，ResinClient 侧 253 |
| `sticky_ttl` | 无（白盒不存粘性时长） | `platform_update{stickyTtl}`、`platform_create_with_fields{body.sticky_ttl}`；TS 与 Rust 均 ≤32 字符且无控制符 | 无输入控件；画布 `PlatformFull.sticky_ttl` 仅展示（快照恒 `0s` 占位） | Go duration 字符串（如 `168h`）；create 可选；缺省走环境变量 `RESIN_DEFAULT_PLATFORM_STICKY_TTL`（默认 168h） |
| `regex_filters` | 无（节点标签过滤属订阅与节点域） | `platform_update{regexFilters}`、`platform_create_with_fields{body.regex_filters}`；TS 与 Rust 均 ≤64 项且每项 ≤253 字符、无控制符 | 无 UI 入口（画布展示恒为 `null`） | 字符串数组；节点标签过滤规则（语义与校验见 DESIGN.md「节点标签过滤规则」）；缺省 `RESIN_DEFAULT_PLATFORM_REGEX_FILTERS`（空数组） |
| `region_filters` | 意图来源：`a_class` 与 `regions` / `manual_nodes` / `subscriptions` / `top_n` 经 `compute_plan` 派生（白盒存意图，不存最终列表） | 直连 `platform_update{regionFilters}`（≤64 项、每项 ≤16 字符、无空格）；收敛 `strategy_apply` 仅在真漂移时 PATCH 单字段 body（diff-then-skip，ADR-0057）；白盒深编辑 `strategy_platform_regions_set`（≤64 项、每项 1..32 字符） | TopologyView 区域 chips 增删（`patchAndSyncOnce`）；深编辑 `ipcStrategyPlatformRegionsSet` + `ipcStrategyApply`；PlatformsView A 类面板 regions / 订阅 / top-N | 字符串数组；每项小写 ISO 3166-1 alpha-2；缺省 `RESIN_DEFAULT_PLATFORM_REGION_FILTERS`（空数组）；收敛路径零漂移零写、wire 幂等 |
| `reverse_proxy_miss_action` | 无 | 无专用入参（仅 `platform_create_with_fields` 自由 body 透传可达；`platform_update` 不支持） | 无 | 枚举 `TREAT_AS_EMPTY` / `REJECT`；缺省 `RESIN_DEFAULT_PLATFORM_REVERSE_PROXY_MISS_ACTION`（`TREAT_AS_EMPTY`） |
| `reverse_proxy_empty_account_behavior` | 无 | 无专用入参（同上，仅 create 自由 body 透传） | 无 | 枚举 `RANDOM` / `FIXED_HEADER` / `ACCOUNT_HEADER_RULE`；缺省默认 `ACCOUNT_HEADER_RULE`；组合约束：取 `FIXED_HEADER` 时下一行字段必填 |
| `reverse_proxy_fixed_account_header` | 无 | 无专用入参（同上） | 无 | 多行字符串，每行一个合法 HTTP Header 字段名、按序尝试提取；缺省默认 `Authorization`；仅当上一字段取 `FIXED_HEADER` 时必填 |
| `allocation_policy` | 语义对应 `b_class`（shell `StrategyId` 六值，ADR-0042 S3） | `platform_update{allocationPolicy}`、`platform_create_with_fields{body.allocation_policy}`；TS `strategyToResinPolicy` 六对三多对一，Rust `ALLOWED_ALLOCATION_POLICIES` 白名单复校验 | PlatformsView B 类 chips（`strategy-bclass-*`）与创建对话框策略下拉（`createPolicy`） | 枚举 `BALANCED` / `PREFER_LOW_LATENCY` / `PREFER_IDLE_IP`；缺省 `RESIN_DEFAULT_PLATFORM_ALLOCATION_POLICY`（`BALANCED`）；映射：random/bandwidth/protocol_weight 对 `BALANCED`，latency 对 `PREFER_LOW_LATENCY`，sequential/quality 对 `PREFER_IDLE_IP` |
| `passive_circuit_breaker_disabled` | 无 | `platform_update{passive_circuit_breaker_disabled: bool}`（TS 第 6 参 `circuitBreakerDisabled`） | 无（wrapper 参数已通、视图暂无调用方） | bool；缺省 `false`（熔断启用）；`true` 时该平台用户代理失败不计熔断，成功仍清零计数，主动探测不受影响 |
| `id`（只读） | 无（白盒以 `platform_name` 为标识） | 不入参；shell 以 name 到 id 反查（`platform_id_for_name`，T17 票收敛为 resolve helper）后拼 `/platforms/{id}` | 画布节点 id 取 `platform_id`，缺省回退 `platform_name`（仅展示） | UUID v4，服务端生成；create 或 PATCH 传 `id` 报 400 |
| `routable_node_count`（只读） | 无 | 只读回传：`platform_list_full` / `authoritative_snapshot` | 平台卡只读展示 | 服务端计算；create 或 PATCH 传它报 400 |
| `updated_at`（只读） | 无（白盒文档级 `StrategyConfig.updated_at` 同名不同义：Unix 秒写时间戳） | 只读回传（RFC3339） | 不展示 | 服务端维护；create 或 PATCH 传它报 400 |

## 白盒独有字段（不直达 Resin）

`PlatformStrategy`（`crates/resin-core/src/strategy_engine.rs:66-85`）中不对应任何 Resin 列的输入字段——它们是 A 类策略的意图参数，唯一出口是被 `compute_plan` 派生成 `region_filters`：

| 白盒字段 | 含义 | IPC 写入 | 前端 form |
| --- | --- | --- | --- |
| `a_class` | A 类选法：`manual` / `region` / `quality` / `subscription` | `strategy_config_put` | PlatformsView A 类 chips |
| `manual_nodes` | manual 模式手工选中的节点 hash | `strategy_config_put` | PlatformsView 节点多选 |
| `subscriptions` | subscription 模式允许的订阅名 | `strategy_config_put` | PlatformsView 订阅 chips |
| `top_n` | quality 模式 Top-N 上限（缺省 10） | `strategy_config_put` | PlatformsView top-N 输入（`strategy-topn-*`，1..1000） |

## 字段映射示例（两条完整链路）

### 示例 1 — 创建 platform：三字段 body

Resin wire（POST /api/v1/platforms）：

```json
{ "name": "Platform-A", "region_filters": ["hk", "us"], "sticky_ttl": "168h" }
```

shell 链路：前端 `ipcPlatformCreateWithFields({ name, region_filters, sticky_ttl })`（TS 校验 name 1..128、region 每项 ≤16 字符、ttl ≤32 字符）→ IPC `platform_create_with_fields`（Rust 复校验同名字段）→ `ResinClient::create_platform_with_fields`（自由 body 原样 POST，#R08）→ Resin 补缺省后返回 201 与完整对象。省略的其余可选字段走 `RESIN_DEFAULT_PLATFORM_*` 环境变量缺省。

### 示例 2 — 更新 region_filters：单字段 PATCH

Resin wire（PATCH /api/v1/platforms/{id}）：

```json
{ "region_filters": ["hk", "us"] }
```

shell 两条路。其一，收敛主路径 `strategy_apply`：`compute_plan` 派生各平台 region 列表，与 live 行做集合等值比对，仅对真漂移平台 PATCH 该单字段 body（ADR-0057 diff-then-skip）。其二，画布直连：区域 chip 增删 → `patchAndSyncOnce` → `ipcPlatformUpdate(name, undefined, undefined, next)` → IPC `platform_update`（name 到 id 反查后 PATCH，#R11）。PATCH 语义见 DESIGN.md「更新平台」：空 patch 400、不可改字段 400、改 `Default` 平台 409。

## issue 猜测字段在 v1.2.0 的核实结果（未来字段待补入口）

issue #8 列举的部分候选名在 v1.2.0 DESIGN.md 中不存在（issue 原文标注「视 Resin 实际」），本表按实际 schema 记录：

| issue 猜测名 | v1.2.0 实况 |
| --- | --- |
| `reverse_proxy_pass` / `reverse_proxy_host_header` | 实际反代字段为 `reverse_proxy_miss_action`、`reverse_proxy_empty_account_behavior`、`reverse_proxy_fixed_account_header` 三件套（见全字段表） |
| `active_circuit_breaker_disabled` | 不存在；熔断开关仅被动面 `passive_circuit_breaker_disabled`，主动探测永不受平台开关影响 |
| `udp` / `multiplex` | platform 模型无此字段（SOCKS5 数据面不支持 UDP ASSOCIATE；`udp` 仅出现在节点 DNS upstream 配置） |

Resin 未来版本新增字段时：先在本页全字段表加行（五列填满）再动代码；基准版本升级时 diff `resin/DESIGN.md` §Platform 并更新页头版本号。

## 维护规则

1. 任一层新增或改名 platform 字段：同一 commit 更新本表对应行，五列必须全部有内容（无对应物写「无 + 原因」）。
2. 白盒 `PlatformStrategy` 增删字段属 schema 变更：须新 ADR 立法（ADR-0036 纪律），并同步更新收敛推导故事（ADR-0058）。
3. `resin_client.rs` 平台方法注释携带 `@see docs/reference/RESIN_PLATFORM_SCHEMA.md` 锚指向本表（update_platform 注释）。
4. sidecar 升级：diff `resin/DESIGN.md` §Platform，核对 12 行与核实节，更新页头基准版本号。
