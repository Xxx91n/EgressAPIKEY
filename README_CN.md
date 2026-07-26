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

## 架构

```
+:端      React 19 + ReactFlow 12 + Zustand 5 + Tailwind CSS
              |
              | Tauri IPC
              v
后端      Rust (tokio + axum + reqwest + petgraph + rusqlite)
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
| 后端 | Rust (tokio, axum, reqwest, petgraph, rusqlite) |
| 代理心 | mihomo (侧车, REST API) |

## 文档

| 文件 | 用途 |
|------|------|
| `docs/MEMORY.md` | 压缩研究记忆——首先阅读 |
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

MIT
