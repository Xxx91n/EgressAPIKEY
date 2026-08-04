# Memory: 轮子复用决策 — 调研结论压缩

> 2026-07-27 调研；数据源：1MCP exa deepdive + 开源爬取
> 受众：下一个接手的 AI / 人类；用途：替代重复调研，决定"造 vs 抄"。

## 一句话结论

**80% 核心逻辑已被 Resin (Go, 1732★) 生产验证完成。保留 mihomo 订阅编译 + Tauri 壳 + 进程路由三大差异化能力，其余直接拥抱 Resin，省 3-6 个月重复造轮子。**

## 你在造轮子吗 — 模块对照

| 你的模块 | 现有轮子 | 复用度 | 建议动作 |
|---|---|---|---|
| 节点调度 / P2C / TD-EWMA / 粘性 IP | Resin 100% 覆盖 | 高 | **直接集成 Resin 为核心内核**，弃自研 scheduler/ |
| SSE 会话锁 / 熔断 / 健康检查 | Resin + Comox 覆盖 | 高 | 迁移到 Resin Platform/Account 事件模型 |
| 多协议入口 (HTTP/SOCKS5/进程路由) | Resin 有 HTTP+SOCKS5 | 中 | 保留你的进程路由 entry/，其余委托 Resin |
| mihomo L4 侧车（订阅编译） | 你的强项 | — | **保留**，作为 Resin 下游出口 |
| 高并发连接池 | Pingora 共享池 | 低（>50K RPS 才需） | 中长期加 pingora feature，短期 reqwest 够用 |
| Tauri 桌面壳 + safety net | Ghost 成熟模式 | 中 | 参考 Ghost 启动握手/崩溃自愈/system tray |
| 跨平台打包 | Tauri 官方 tauri-action | 高 | 直接套官方矩阵 workflow |

## 5 个参考项目速查

### Resin (Resinat/Resin) — Go, 1732★, MIT
- 10万+ 节点；P2C + 域名感知 TD-EWMA；粘性锚定出口 IP（同 IP 多节点可互换）
- Platform + Account 双层隔离；每 Platform 独立可路由视图 + xsync.Map 租约表
- 热路径无锁（P2C + 原子）；冷路径全量重建可路由集合；TD-EWMA 分权威域名/普通站点 LRU
- 单二进制：go build -tags "with_quic with_wireguard with_grpc with_utls"
- 三种入口：HTTP 正向代理 / SOCKS5 / 反向代理（BaseURL 替换）
- 零侵入：可从 Authorization 头提取 Account 自动绑定 IP

### Pingora (Cloudflare / STOA 嵌入) — Rust, Apache-2.0
- 共享连接池替代 per-client pool；50K+ RPS 不耗尽；H2 global multiplexing
- STOA 选择 Embedded Connector（feature-gated pingora），单二进制零 sidecar
- <1K RPS 差异 0.5ms p95；>50K RPS 才体现优势
- 迁移成本 13 pts（STOA 估算）；需 cmake；CI 需预装
- 迁移模式：ProxyPhase trait 1:1 映射 ProxyHttp，仅代理路径切 PingoraPool.send_request()

### Ghost (Ghostsproxy) — Tauri + Go sidecar
- Rust Tauri 壳 + Go ghost-engine sidecar；HTTP/WS 通信
- safety net：sidecar 死后 Rust 调 OS API 关系统代理，不依赖 sidecar 存活
- 启动握手：sidecar 15s 内输出 JSON（api_port/proxy_port/token），超时退出
- system tray：每 3s 轮询后端状态，连续 3 次失败触发 safety net

### Comox AI Gateway — Go, 企业级 LLM 网关
- goroutine 万级并发 SSE；单二进制；GC 亚毫秒停顿
- Least-Latency 路由 + 模型回退 + 语义缓存（向量）+ 熔断器
- 印证高并发 SSE 必须 Go/Rust；无 SOCKS5/进程路由/粘性 IP（仅算法参考）

### Only1MCP — Rust (axum), 10k+ req/s, <5ms
- MCP 聚合网关；bb8 连接池 + DashMap 缓存；SSE/STDIO/HTTP 多传输
- 仅作 Rust 高并发模式参考；无代理池/出口 IP/订阅导入

## 推荐迁移路径（分阶段）

| 阶段 | 目标 | 工作量 | 风险 |
|---|---|---|---|
| P0 | scheduler/ 替换为 Resin Platform/Account；gateway/proxy.rs 转发到 Resin sidecar | ~2 周 | 低（Resin HTTP API 稳定） |
| P1 | SSE 粘性锁迁到 Resin 租约；补进程路由；Ghost 式 safety net | ~1 周 | 低 |
| P2 | Tauri + Resin sidecar 编排；CI/CD 矩阵 Win/Linux/macOS | ~1 周 | 中（sidecar 打包） |
| P3 | pingora feature gate 替换 reqwest；语义缓存/模型回退（Comox） | 持续 | 低 |

## 可直接复用的代码 / 配置源

- Resin 核心：github.com/Resinat/Resin（go build 得 resin 二进制，Tauri sidecar 托管）
- Resin 配置：RESIN_AUTH_VERSION=V1 + Platform/Account YAML（替代拓扑 JSON）
- Tauri CI：tauri-apps/tauri-action 官方矩阵 workflow
- Ghost safety net：disable_system_proxy_native() + sidecar 心跳
- Pingora 嵌入实现：stoa-platform/stoa#1801 PR

## 关键技术取舍已定

- 不强行 Rust SSE：高并发层走 Go (Resin / Comox 路径)
- 不抛 mihomo：保留订阅编译 + Clash YAML 编译能力，作为 Resin 下游
- 不自己实现 IP 粘性：用 Resin 出口 IP 锚定 + 租约表
- 不引入 Pingora 直至 >50K RPS：短期 reqwest pool_max_idle=0 足够


---

## 决策复审：A vs C（2026-07-29)

> 本节在上文「推荐迁移路径」之后追加；复审动机来自第一窗口的代码证据质疑——当前代码库是否真的 fork 了 Resin。
> 证据链：github.com/Resinat/Resin/languages API、commits API、exa 抓取的 README/Dockerfile、本仓库 crates/resin-core/src 实际 LoC。
> 全过程产物持久在 .omx/goals/autoresearch/go-vs-rust-path/{mission.json,rubric.md,ledger.jsonl,completion.json}。

### 路径定义

- **A**：fork Resin（github.com/Resinat/Resin），把其 Go 单二进制 `resin` 作为 Tauri sidecar 嵌入，Rust 壳只做 life-cycle 编排 + Ghost 安全网 + mihomo 控制；前端把 `Resin/webui` 改桌面包。
- **C**：保留当前 `crates/resin-core`，从零用 Rust 重写 P2C / TD-EWMA / lane / lease / Platform / Account，并在 Rust 生态里补上 sing-box 多协议 outbound 的对应物。

### 关键证据（可验证）

- Resin Go 代码量：**1,834,190 字节**（GitHub `/languages` API 返回），折算 ~30k LoC
- Resin TypeScript webui：**473,116 字节**，~10k LoC，React + Vite
- Resin 活跃度：最近提交 **2026-07-05**，Go 1.25，MIT
- Resin 构建产物：**单二进制 `resin`**，`webui/dist` 被 `go:embed` 打进二进制；构建 tag `with_quic with_wireguard with_grpc with_utls`
- 当前 `crates/resin-core/src` Rust：**1370 LoC**，只实现了 lane/lease/tdewma/platform 四个模块的「最外层 pattern」，无 axum 监听、无 sing-box outbound 集成、无 SQLite 状态持久化、无订阅编译器、无 geoip、无熔断
- 当前前端 `src/`：**1549 LoC** TS/TSX across 16 files，自研 React + ReactFlow

### 三维裁决

| Dimension (权重) | A | C | 胜方 |
|---|---|---|---|
| completion_difficulty (0.33) | 剩 ~3-4k LoC 粘合；Resin 生产路径已验证 | 需 ~25-35k LoC from-scratch + 不可绕过的 sing-box-Rust 替代子系统（生态硬缺口） | **A**，decisive |
| architecture_complexity (0.33) | 2 二进制 + HTTP-IPC（Resin 本就是单 port 2260 承载 API+Web+代理）+ Ghost 安全网 | 当前态简单，目标态需自研多协议 outbound 引擎（独立开源项目级子系统） | **A**，margin |
| long_term_maintenance (0.34) | 上游 MIT 托底 + 周期性 rebase drift；Resin V1 API 已稳定 | 永久 chasing sing-box 上游 + 自研子系统全责，无托底 | **A**，3/4 子轴 |

### 决策：**采用路径 A**

### A 路径已知代价与缓解

1. 二进制体积膨胀到 ~33MB：接受，代理软件常态 mihomo 同量级
2. sidecar 崩溃面新增：Ghost 模式 3s 轮询 + Resin `/health` REST + 关系统代理 safety-net；Ghost 已生产验证
3. 三平台初次 Go 工具链集成：直接拉 Resin 官方预编译二进制，不本地自建除非改 `vendor/resin` 源
4. IPC 纪律：admin token 留 Rust 侧，webview 走 Rust 中继 IPC，沿用 AGENTS §7.6 现成规矩

### 与原任务指令的对齐

原任务 #2 字面「复用 Resin 的 webui 代码：把 Resin/webui/ 整个搬进 Tauri src/」，被 exa 抓取证据证实：`Resin/webui/` 确实是现成 React+TS+Vite 项目。该任务前提成立，第一窗口误判为「前提不成立」并走偏到 C；本次复审修正回 A。


---

## 路线纠正结论 (2026-08-05, ADR-0012)

### 前提交翻

原 P24-A4-3 interceptor 架构假设：shell 能从 HTTPS 代理流中读取 Authorization
header 来识别 (api_key, upstream v1 endpoint) 组合。

**事实**：HTTPS 上游加密后，代理层只看到 CONNECT 隧道字节，Authorization header
在 TLS 信封内，永远不可见。此前提被推翻。

### 正确路线

软件定位：agent -> AI 网关 (omniroute/litellm) -> **本软件 (代理网络层)** -> 上游 v1

唯一可行的身份机制是 **port-based**：AI 网关在本软件配置 per-key (或 per-key-group)
socks5/http 代理端口。每个端口 = 一个身份。shell 根据入站端口号注入
X-Resin-Account，不需要任何 header 解析。

### 架构 (Path A — 薄壳，不 fork Resin)

1. 多端口 socks5/http listener (tokio TcpListener + 协议检测)
2. Port -> (platform, account) 映射表 (SQLite, 复用 DbPool)
3. X-Resin-Account 注入 (port-based, 非 header-based)
4. Resin sidecar 不变 (P2C + TD-EWMA + sticky exit IP + SSE lease)
5. 模块化策略决策层 (可插拔: 测活/延时/带宽/质量/协议权重/IP信誉)
6. AI 流感知模块 (SSE/WebSocket, 独立板块)
7. hotswap-config (白盒配置层, 原子备份, 热重载)

### 删除的死代码 (ADR-0014)

- interceptor.rs (axum proxy_handler + route_id 注入)
- lane.rs route_id + normalize_auth (8 cargo tests)
- db.rs observed_keys 表 (DbPool 基础设施保留)
- ADR-0003, ADR-0011 SUPERSEDED

### 保留的

- Resin sidecar lifecycle, Ghost safety net, Subscriptions CRUD, Node pool,
  Backup/config, i18n/tray/settings — 全部不变

### 项目改名

ai-api-route -> **EgressAPIKEY** (ADR-0013, 无撞名, 5 源审计)

### 一句话结论 (更新)

"Process/Account/订阅管理直接拥抱 Resin...保留 mihomo 订阅编译 + Tauri 壳 +
进程路由三大差异化" — 此结论不变。新增：薄壳多端口转发器 (port=identity) 作为
核心差异化，策略层 + AI 流感知 + IP 信誉作为模块化扩展。
