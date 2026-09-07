# EgressAPIKEY

**面向 AI 网关与上游服务商的多端口 socks5/http 转发器 —— 为 AI API Key 提供粘性出口 IP 路由，并配有拓扑画布。**

[![License](https://img.shields.io/github/license/Xxx91n/EgressAPIKEY?style=flat-square)](https://github.com/Xxx91n/EgressAPIKEY/blob/main/LICENSE) [![CI](https://img.shields.io/github/actions/workflow/status/Xxx91n/EgressAPIKEY/ci.yml?style=flat-square&label=CI)](https://github.com/Xxx91n/EgressAPIKEY/actions/workflows/ci.yml) [![Release](https://img.shields.io/github/v/release/Xxx91n/EgressAPIKEY?style=flat-square)](https://github.com/Xxx91n/EgressAPIKEY/releases)

[English](README.md) | 简体中文

> AI / automation agents：机器可读的仓库地图见 [llms.txt](llms.txt)。
> 原名 **ai-api-route** —— 按 [ADR-0013](docs/adr/0013-project-rename-egressapikey.md) 改名。

## 是什么与为什么

EgressAPIKEY 是一款桌面应用（Tauri 2 + React 19），位于你的 AI 网关（OmniRoute / LiteLLM / CLIProxy）与上游 OpenAI 兼容 v1 服务商之间。它暴露多个 socks5/http 入口端口；每个端口即一个（平台，账户）身份对的标识，Resin Go 侧车保证每一对都拥有独立的粘性出口 IP —— 除非你明确允许，一个 AI API Key 绝不与其他 Key 共享出口 IP。

单一统一代理无法为 HTTPS 上游携带按 Key 的身份（CONNECT 隧道对目的地不可见），因此多端口才是正确的身份机制（[ADR-0012](docs/adr/0012-route-correction-thin-shell-multi-port.md)）。壳侧后端核心是 `crates/resin-core`（Rust：tokio、reqwest、rusqlite）；代理引擎是 Resin 侧车 v1.2.0，仅通过环回 REST 接缝访问（构建期由 `scripts/fetch_resin.sh` 获取）。

## 下载

**桌面版** —— 安装包（MSI / NSIS / deb / AppImage / dmg）与每平台一个免安装、开箱即用的可执行文件发布在 [Releases](https://github.com/Xxx91n/EgressAPIKEY/releases) 页面。目前尚无 tagged release —— 产物将随首个 tagged release 一并发布。

**无头服务器** —— 同一套控制面（平台 / 订阅 / 节点 / 拓扑），可在任意浏览器经 `http://127.0.0.1:14200` 访问，适用于 Linux 服务器、Docker 或远程 VPS。admin bearer 令牌由服务端注入，浏览器永远看不到它。`@egressapikey/server` npm 启动器**未发布到 npm** —— 请从源码运行：

```bash
git clone https://github.com/Xxx91n/EgressAPIKEY.git
cd EgressAPIKEY
pnpm install && pnpm build
cargo build --release -p egressapikey-app --bin egressapikey-headless --features headless
bash scripts/fetch_resin.sh   # 将侧车二进制固定拉取到 src-tauri/binaries/（或用 Go 构建 resin/）
target/release/egressapikey-headless --dist dist --binary-dir src-tauri/binaries --no-browser
```

Windows 下二进制为 `target\release\egressapikey-headless.exe`。部署布局（systemd unit、Docker、环境变量表、TLS 反代）：[docs/how-to/HEADLESS_DEPLOYMENT.md](docs/how-to/HEADLESS_DEPLOYMENT.md) · 运维（日志、健康、停机）：[docs/how-to/HEADLESS_RUNBOOK.md](docs/how-to/HEADLESS_RUNBOOK.md)

## 特性

- **入口端口即身份** —— 每端口对应一个（平台，账户）对；Resin 原生绑定粘性出口 IP
- **SSE 会话粘性** —— 流式响应锁定其节点直至完成，失败时自动切换
- **每请求 TCP 新建** —— `pool_max_idle_per_host(0)` 让每次请求都走全新连接
- **策略引擎** —— A 类策略决定哪些 IP 进入平台（地区 / 质量 / 订阅源），B 类策略决定端口如何选出口（随机 / 轮询 / 低延时）
- **拓扑画布** —— 拖拽连线即向在线侧车热更新各平台的 region 过滤器
- **零适配** —— 把你的网关指向一个入口端口即可；客户端代码无需改动
- **无头孪生** —— 完整的 GUI 控制面经 HTTP 提供，无需桌面壳

## 截图

<!-- 占位：真实的拓扑画布 / 平台页截图是用户提供的资产，刻意不代生成。 -->
<!-- 建议投放：全宽 PNG（<=1280px），拓扑画布一张、平台双栏一张。 -->

_截图待补 —— 此位预留。_

## 快速开始（开发）

前置要求：Node.js 20+ 与 pnpm、Rust stable，以及所在平台的 webview 依赖（[Tauri v2 前置要求](https://tauri.app/start/prerequisites/) —— Linux 需要 `webkit2gtk-4.1` 等）。

```bash
git clone https://github.com/Xxx91n/EgressAPIKEY.git
cd EgressAPIKEY
pnpm install
pnpm tauri dev
```

## 文档

| 文档 | 用途 |
| --- | --- |
| [docs/architecture/ARCHITECTURE.md](docs/architecture/ARCHITECTURE.md) | 分层、数据流、配置权威（L1/L2/L3）、技术栈 |
| [docs/adr/](docs/adr/) | 架构决策记录（编号、只追加） |
| [docs/how-to/HEADLESS_DEPLOYMENT.md](docs/how-to/HEADLESS_DEPLOYMENT.md) | 无头服务器部署（systemd、Docker、TLS） |
| [docs/how-to/HEADLESS_RUNBOOK.md](docs/how-to/HEADLESS_RUNBOOK.md) | 无头服务器运维（日志、健康、停机、故障排查） |
| [docs/how-to/RELEASE.md](docs/how-to/RELEASE.md) | 发布流水线：CI 矩阵、产物分组 |

## 合规声明

> **合规声明** —— 本项目仅用于合法用途：路由你自有的 API Key、个人自动化、研究与学习。你有责任自行遵守所在司法辖区的全部适用法律法规及上游服务商服务条款。作者对任何滥用行为不承担责任。

## 参与贡献

欢迎提交 Issue 与 Pull Request —— 请到 [GitHub Issue](https://github.com/Xxx91n/EgressAPIKEY/issues)。首次提交 PR 前，请先阅读 [AGENTS.md](AGENTS.md)（仓库约定：i18n 全键覆盖、每个行为必须有测试、许可证字段纪律）与[架构总览](docs/architecture/ARCHITECTURE.md)。包含桌面开发环境搭建的独立 `CONTRIBUTING.md` 已列入路线图。

## 第三方声明

Resin Go 侧车（v1.2.0，上游 [github.com/Resinat/Resin](https://github.com/Resinat/Resin)）携带两层许可证值：**声明 MIT**（依其自身 `LICENSE`），而其编译依赖树经 `github.com/sagernet/sing-box v1.12.21`（钉在 `resin/go.mod`）传递 **GPL-3.0-or-later** 义务。壳与侧车是独立进程，仅通过环回 REST 接缝交互（单纯聚合）。完整清单与引用：[THIRD_PARTY.md](THIRD_PARTY.md) · 决策记录：[ADR-0067](docs/adr/0067-license-layering-provenance.md) · 完整许可证文本：[LICENSE](LICENSE)。

## 许可

GPL-3.0-or-later
