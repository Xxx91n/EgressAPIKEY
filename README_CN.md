# EgressAPIKEY

> 原名 **ai-api-route**。按 ADR-0013 改名。

**面向 AI API Key 的特化代理池，配有拓扑画布。**

一款桌面应用（Tauri 2 + React 19），为 AI API Key 提供 L7 代理网关。每个暴露端口对应一个 (平台, 账户) 身份对，Resin 原生按端口绑定粘性出口 IP，确保不同端口走不同出口 IP —— 解决同域同 IP 碰撞这一 AI API Key 池的常见痛点。

[English](README.md)

## 快速开始

```powershell
# 安装依赖
pnpm install

# 下载 mihomo 侧车程序
powershell -File scripts/download-mihomo.ps1

# 启动开发环境
pnpm tauri dev
```

## 核心设计

- **端口即身份** —— 每个暴露端口对应一个 (平台, 账户) 对，Resin 原生按端口绑定粘性 IP（ADR-0012/0014/0015）
- **每请求新建 TCP** —— `pool_max_idle_per_host(0)` 确保每次请求都是新连接、新 IP
- **SSE 会话粘性** —— 流式传输期间锁定节点，断开后自动切换
- **零适配侵入** —— OmniRoute 用户只需改一处代理地址；客户端代码无需修改
- **备份安全** —— 备份写入用户专属 app data 目录（不落共享系统临时目录），且上传路径有目录越界防护，防止被攻陷的 webview 借 WebDAV 上传路径窃取任意文件

## 无头服务器

同一套控制台（平台 / 订阅 / 节点 / 拓扑画布）可不安装 Tauri 桌面壳，直接以浏览器访问，适用于 Linux 服务器、Docker 容器或远程 VPS：

```bash
npm install -g @egressapikey/server
egressapikey-server
# → 在任意浏览器打开 http://127.0.0.1:14200/
```

launcher 会拉起 Rust `egressapikey-headless` 二进制，它启动 Resin Go 侧车、托管预构建的 React SPA，并将 `/api/v1/*` 与 `/metrics/*` 反代到本地 Resin 控制面 —— admin bearer 由服务端注入，浏览器永远看不到该令牌。SSE / WebSocket 流通过 `Body::from_stream` 透传。

- 文档：[`docs/how-to/HEADLESS_DEPLOYMENT.md`](docs/how-to/HEADLESS_DEPLOYMENT.md)（systemd unit、环境变量表、TLS 反代）与 [`docs/how-to/HEADLESS_RUNBOOK.md`](docs/how-to/HEADLESS_RUNBOOK.md)（日志、健康检查、优雅退出、故障排查）
- 决策记录：[`docs/adr/0043-headless-server-build-separation.md`](docs/adr/0043-headless-server-build-separation.md)（源码搬迁 + `required-features = ["headless"]` 门禁 + CI 任务拆分）
- 发布产物：`release/<os>-backend/` —— 自包含目录，`egressapikey-headless` + `dist/` + `resin` 同级

## 平台与 IP 通道路由

**拓扑** 标签页是三列 key→出口 画布（入口代理端口 → 平台 → IP 通道）：
- **A：入口代理端口** —— 概念上的 Resin 转发代理入口；默认与每个平台始终连接。
- **B：平台** —— 每个平台对应画布上的一个节点。平台到节点组的连线表示该平台的 `region_filters` 包含该区域。拖动建立连线即向 Resin 侧热 PATCH 平台的 `region_filters`；删除连线即移除该 region。每次拓扑拖动都会自动先做一次备份（防呆）。
- **C：IP 通道 / 节点** —— 每个 region 分组对应一个节点，显示健康/总数及可路由节点数。

**平台** 标签页是双栏 key→平台 界面：
- **左栏**：候选 key 组合（上游 v1 端点 + API key），本地存储在 `settings.json#keyCandidates` —— 在激活前从不发送给 Resin。
- **右栏**：实时 Resin 平台 + 每平台租约。从左栏拖动 key 到右栏空白处即创建 `auto-{uid}` 独立平台（POST /platforms，策略 `BALANCED`）；拖到已有平台上即附加该 key。实线边框卡片是自动独立平台，虚线边框卡片是手动平台。每平台的 `allocation_policy` 是个 5 档出口策略选择器（随机/顺序 → `BALANCED`，延时 → `PREFER_LOW_LATENCY`，质量 → `PREFER_IDLE_IP`）。

**节点** 标签页以按订阅源折叠的树形展示（clash-verge-dev 模式）。每个订阅可展开显示其节点，含实时延时（绿 <200ms / 黄 200-500ms / 红 >500ms / 灰超时）、`display_tag`、`region`、健康状态，并附协议权重参考卡（SSE 适用性：http/socks5/vmess=1.0，shadowsocks=0.7，hysteria2/tuic/wireguard=0.1）。平台标签页承载策略引擎面板（ADR-0022）：A 类策略选择哪些 IP 进入平台（手动 / 地区 / 质量分 / 订阅源，受自动探活门控），B 类策略选择端口如何选出口 IP（随机 / 轮询 / 低延时）。白盒配置 `egressapikey-strategy.json`。

**诊断 (Diagnostics)** 标签页是完整的诊断中心（唯一入口，与设置页无冗余）：sidecar 状态卡片（端口/模式/PID/healthz/IPC 延时）、防火墙状态（跨平台：Windows Get-NetFirewallProfile / Linux systemctl-ufw-firewalld / macOS pfctl，均有 5s 超时 + CREATE_NO_WINDOW）、请求日志表（可配置自动轮询，默认 5s，1s-60s 范围）、出口 IP 探测（HTTP+SOCKS5 到 1.1.1.1/cdn-cgi/trace）、端口健康检查（TCP 连接延时）、sidecar 日志缓冲区、日志目录按钮。替代原先拥挤的 Settings > Diagnostics 面板（T7 重构）。

## 架构

```
+:端      React 19 + ReactFlow 12 + Zustand 5 + Tailwind CSS
              |
              | Tauri IPC
              v
后端      Rust (tokio + axum + reqwest + rusqlite)
              |
              | HTTP proxy, pool_max_idle=0
              v
代理心    mihomo 侧车子进程（REST API 控制）
```

更多细节见 [架构文档](docs/architecture/ARCHITECTURE.md)。

## 技术栈

| 层 | 技术 |
|---|------|
| 桌面壳 | Tauri 2 (Rust) |
| 前端 | React 19, TypeScript, ReactFlow 12, Zustand 5, Tailwind CSS |
| 后端 | Rust (tokio, axum, reqwest, rusqlite) |
| 代理核心 | Resin Go 侧车 v1.2.0 (github.com/Resinat/Resin) |

## 文档

| 文件 | 用途 |
|------|------|
| `docs/architecture/MEMORY_REUSE_DECISION.md` | 压缩研究记忆——首先阅读 |
| `docs/architecture/ARCHITECTURE.md` | 分层架构、数据流、技术栈、回退方案 |
| `docs/history/phases/PROJECT_PLAN.md` | 分阶段交付（P0–P9）及成功标准 |

## 开发

```bash
# 类型检查
pnpm tsc --noEmit

# Rust 代码检查
cargo check --manifest-path src-tauri/Cargo.toml

# 代码图重建索引
npx codegraph sync

```

## 许可

GPL-3.0-or-later
