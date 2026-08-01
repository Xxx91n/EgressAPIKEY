# AI API Route · AI API 路由

**面向 AI API Key 的特化代理池，配有拓扑画布。**

一款桌面应用（Tauri 2 + React 19），为 AI API Key 提供 L7 代理网关。每个 Key 哈希到一条车道，每次请求通过 mihomo 新建一条 TCP 连接，确保不同 Key 走不同出口 IP —— 解决同域同 IP 碰撞这一 AI API Key 池的常见痛点。

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

- **N 车道**（默认 10，上限 50）—— Key 哈希后分配到车道，而非 1:1 端口映射
- **每请求新建 TCP** —— `pool_max_idle_per_host(0)` 确保每次请求都是新连接、新 IP
- **SSE 会话粘性** —— 流式传输期间锁定节点，断开后自动切换
- **零适配侵入** —— OmniRoute 用户只需改一处代理地址；客户端代码无需修改
- **备份安全** —— 备份写入用户专属 app data 目录（不落共享系统临时目录），且上传路径有目录越界防护，防止被攻陷的 webview 借 WebDAV 上传路径窃取任意文件
## 平台与 IP 通道路由

**拓扑** 标签页是三列 key→出口 画布（入口代理端口 → 平台 → IP 通道）：
- **A：入口代理端口** —— 概念上的 Resin 转发代理入口；默认与每个平台始终连接。
- **B：平台** —— 每个平台对应画布上的一个节点。平台到节点组的连线表示该平台的 `region_filters` 包含该区域。拖动建立连线即向 Resin 侧热 PATCH 平台的 `region_filters`；删除连线即移除该 region。每次拓扑拖动都会自动先做一次备份（防呆）。
- **C：IP 通道 / 节点** —— 每个 region 分组对应一个节点，显示健康/总数及可路由节点数。

**平台** 标签页是双栏 key→平台 界面：
- **左栏**：候选 key 组合（上游 v1 端点 + API key），本地存储在 `settings.json#keyCandidates` —— 在激活前从不发送给 Resin。
- **右栏**：实时 Resin 平台 + 每平台租约。从左栏拖动 key 到右栏空白处即创建 `auto-{uid}` 独立平台（POST /platforms，策略 `BALANCED`）；拖到已有平台上即附加该 key。实线边框卡片是自动独立平台，虚线边框卡片是手动平台。每平台的 `allocation_policy` 是个 5 档出口策略选择器（随机/顺序 → `BALANCED`，延时 → `PREFER_LOW_LATENCY`，质量 → `PREFER_IDLE_IP`）。

**节点** 标签页展示实时节点池快照，并附协议权重参考卡（SSE 适用性：http/socks5/vmess=1.0，shadowsocks=0.7，hysteria2/tuic/wireguard=0.1）和出口策略引导卡（指向拓扑画布中绑定每平台 `allocation_policy`）。GUI 不自造每节点策略选择器——Resin v1.1.2 没有该端点，故策略选择器绑定的是真实的平台 `allocation_policy` 字段。


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

更多细节见 [架构文档](docs/ARCHITECTURE.md)。

## 技术栈

| 层 | 技术 |
|---|------|
| 桌面壳 | Tauri 2 (Rust) |
| 前端 | React 19, TypeScript, ReactFlow 12, Zustand 5, Tailwind CSS |
| 后端 | Rust (tokio, axum, reqwest, rusqlite) |
| 代理心 | mihomo (侧车, REST API) |

## 文档

| 文件 | 用途 |
|------|------|
| `docs/MEMORY_REUSE_DECISION.md` | 压缩研究记忆——首先阅读 |
| `docs/ARCHITECTURE.md` | 分层架构、数据流、技术栈、回退方案 |
| `docs/PROJECT_PLAN.md` | 分阶段交付（P0–P9）及成功标准 |

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
