# Handoff — Path A 实施移交（2026-07-29）

> 接手 agent 必读：
> 1. AGENTS.md（项目宪法，先读后做）
> 2. docs/MEMORY_REUSE_DECISION.md（含 A vs C 决策复审，决策=A）
> 3. docs/ARCHITECTURE.md、docs/PROJECT_PLAN.md
> 4. 本文件
> 接手后第一动作：`codegraph sync .` 再做任何代码读。

## 上一窗口交付的成果

1. **决策复审固化到 docs/MEMORY_REUSE_DECISION.md**：A vs C 三维裁决，采用**路径 A**——fork Resin Go 二进制作为 Tauri sidecar 嵌入，Rust 壳仅做生命周期编排 + Ghost 安全网 + mihomo 控制。
2. 6 个用户报告 bug 已修并推送（commits 4bb143a/a183287/fd0090d + 本窗口 fe6cc33）。
3. AGENTS.md §11 已写入「runtime wiring state（post-#6 phase 1 sync）」，明确标注 **Resin 代理运行时尚未实现**——这是 phase 4 要接手的目标。
4. 评估全过程产物持久在 `.omx/goals/autoresearch/go-vs-rust-path/{mission.json,rubric.md,ledger.jsonl,completion.json}`，可审计。

## 当前架构盘点（codegraph 已 sync，9 文件已变更）

### Rust 侧
- `crates/resin-core/src/lib.rs`：导出 `CoreConfig{lanes,bind,mihomo_api,mihomo_secret}`，`DEFAULT_LANES=10`、`MAX_LANES=50`、`sanitize_lanes()`。这是**重写残留**，phase 4 将大幅砍除。
- `crates/resin-core/src/gateway.rs`：`GatewayState` 状态机（reserve/release/evict_lane/record_latency/snapshot）。**无 axum listener、无 SSE forward 实现**。phase 4 砍为「Forward client wrapper」。
- `crates/resin-core/src/{lease,tdewma,platform,lane,mihomo}.rs`：1370 LoC 从零重写。phase 4 保留 mihomo.rs（mihomo 控制 wrapper），其余弃用或仅保留数据传输结构（DTO）。
- `src-tauri/src/main.rs`：`.setup()` 读取 settings.json 注入 CoreConfig，trace + drop。**未启动 sidecar**。
- `src-tauri/src/commands/mod.rs`：14 个 `#[tauri::command]`（见下方清单）。全部委托给 in-process kernel，未委托给 Resin。phase 4：大部分改为 forward → Resin REST。
- `src-tauri/src/tray.rs`：build_tray + apply_labels（i18n）、`tray_refresh_labels` 命令。

### 前端侧
- `src/lib/ipc.ts`：12 个类型化 IPC 包装（gateway/platform/account/tray）。
- `src/views/SettingsView.tsx`：调 `tray_refresh_labels`。Save 按钮、Network Save footer。
- `src/views/TopologyView.tsx`：调 `ipcGatewaySnapshot`，每 5s 轮询，rebuild lanes。
- `src/views/PlatformsView.tsx`：**已对接 IPC**（5 个 platform/account）。
- `src/views/SubscriptionsView.tsx`：**0 invoke()**，纯 local Zustand。
- `src/views/ProcessRouteView.tsx`：**0 invoke()**，纯 local Zustand。
- `src/store/appStore.ts`：Zustand store 集中所有 UI 状态。
- `src/locales/`：18 个 locale × 60 keys。

### Resin sidecar 接入缺口（phase 4 核心交付）
- ❌ 无 `vendor/resin/` 或 git submodule，`resin` 二进制不存在
- ❌ 无 tauri-plugin-shell sidecar 配置（`tauri.conf.json` 无 `bundle.externalBin`）
- ❌ 无 Resin 健康握手（Ghost 模式 15s JSON 带 api_port/proxy_port/token）
- ❌ 无 3s 轮询的 safety-net（sidecar 挂了关系统代理）
- ❌ admin token 未保存在 Rust-only trust 侧（端到端 token 链路未建立）
- ❌ 前端无 Resin/webui 搬入迹象

## 已知风险与遗留（非 phase 4 但需要跟踪）

- `crates/resin-core/src/mihomo.rs` `MihomoController::new` 用 `debug_assert!` 做回环校验，**release build 被剥除** → 潜在 SSRF 洞。修复：phase 4 重写 mihomo.rs 为「向 Resin REST 转发」，原 SSRF 守卫改 `if !is_loopback(...) return Err`。
- `cargo test -p ai-api-route-app` 在本机崩 `STATUS_ENTRYPOINT_NOTUSED`（Tauri 链接器，环境非代码问题）。CI 依赖 `cargo build` + `cargo test -p resin-core` + `vitest`。
- debug build 的 `generate_context!` 用 devUrl，复制 release exe 才可用，debug exe 双击会 ERR_CONNECTION_REFUSED。

## Phase 4：路径 A 实施的精细化 Spec

### 目标序列（建议按序交付，每个目标即为一个可验证 milestone）

**G1：vendor Resin binary + sidecar 骨架（P0，阻塞其余）**
- spec：
  - `.gitmodules` add `vendor/resin`，指向 `github.com/Resinat/Resin@<最新 tag>`；或预编译二进制路径 `.sidecar/resin-<target>.exe|bin`
  - `src-tauri/tauri.conf.json` 加 `bundle.externalBin`，三 target：`x86_64-pc-windows-msvc`、`aarch64-apple-darwin`+x86_64-apple-darwin、`x86_64-unknown-linux-gnu`
  - `src-tauri/src/sidecar.rs` 新增：`boot_resin(app: &AppHandle) -> Result<Handle>`，拉起 sidecar，15s 内读取 stdout JSON `{api_port, proxy_port, token}`（Ghost 握手协议），超时 fail-fast
  - `main.rs .setup()` 改：读 settings.json → CoreConfig 之外，**额外** `let _ = boot_resin(app.handle())?;`；cfg 不再 drop
  - acceptance：`cargo build -p ai-api-route-app` 绿；release exe 双击能拉起 resin sidecar（cmd line 见 `resin.exe` 子进程），主窗口 15s 内可见；失败路径有明示 error dialog
  - test：`e2e/sidecar_boot.spec.ts` 用 playwright 拉起 release build，断言主窗口存在 + sidecar 进程出现

**G2：Forward client wrapper + IPC 转发**
- spec：砍掉 `crates/resin-core/src/{lane,lease,tdewma,platform}` 4 模块的自实现（保留 `mihomo.rs`），新建 `crates/resin-core/src/resin_client.rs`：
  - `ResinClient { base: Url, token: String }`，构造自 boot 握手的 token + api_port
  - `async fn proxy_request(&self, req: ReqBuilder) -> Result<Response>`：转发到 Resin 的 forward proxy port（`proxy_port`）
  - `async fn admin(&self, path) -> Result<JsonValue>`：调 Resin REST（platform/account/lease）
- `src-tauri/src/commands/mod.rs` 14 个 command 改写：
  - `platform_add/remove/list/snapshot`、`account_add`、`account_bind_ip`、`gateway_select_account` → 直接 forward 到 `ResinClient::admin`（不经过 `SharedRegistry`）
  - `gateway_reserve/release/evict_lane/record_latency` → Resin 的 forward proxy 自然走它的 sticky 机制，这些命令**保留 IPC 但实现变成 no-op 或 status echo**（站点 UI 不依赖，能绕则绕）
  - `gateway_snapshot` → 调 Resin `/api/v1/stats`（或新 window 用 `/api/v1/platform/<name>/snapshot`）
  - `get_config_dir/get_log_dir`、`tray_refresh_labels` 保留不变
- acceptance：前端 `PlatformsView` reality check：add platform → Resin SQLite 出现一行；侧栏计数与 Resin admin API 一致
- test：`e2e/platform_round_trip.spec.ts` add/list/remove 闭环；`crates/resin-core/tests/resin_client.rs` 用 mockito 测 client

**G3：Ghost 安全网 + 健康轮询**
- spec：扩 `src-tauri/src/sidecar.rs`：
  - `spawn_health_poll(app: AppHandle, client: ResinClient)`：3s 调 `/health`，连续 3 次失败：关闭系统代理（OS API）、托盘转红、弹 notification
  - 不在 `ResinClient` 内部维护，由 Tauri 调度；关代理的 OS API：Windows=注册表 `ProxySettings` + `WinINet`、macOS=`networksetup`、linux=GNOME gsettings（shell 调用，走 `tauri-plugin-shell`）
- acceptance：kill `resin.exe` 后 3 秒托盘变红 + 系统代理被关；再启动 sidecar，5s 内托盘转绿
- test：`e2e/safety_net.spec.ts` 模拟 kill（powerShell stop-process）后断言托盘状态

**G4：前端搬入 `Resin/webui` + 桌面化**
- spec：`vendor/resin/webui` → `src/resin-views/`（保留 React/Vite/TS）。布局嵌入 Tauri webview 中分页作为子 view：
  - 把 Resin 3 个主页面（Platform/Account、Subscription、Lease）作为 `<ResinFrame>` 子组件引入
  - 你已有的 TopologyView（ReactFlow 拓扑画布）作为独立的「Topology」tab 与 Resin views 并排
  - 系统托盘、i18n、theme 继续复用你已有的 `src-tauri/src/tray.rs` 和 `src/locales/`
  - 保留你的进程路由（Resin 无此功能）作为 `ProcessRouteView`，IPC 委托给 ResinClient.admin 创建 account header rule
- acceptance：用户在 5 个 tab 之间切换：Topology（你的画布）/ Platforms（Resin 平台页）/ Subscriptions（Resin 订阅页）/ ProcessRoute（你独有）/ Settings
- test：`e2e/tab_switch.spec.ts`，每个 tab 渲染非空

**G5：CI/CD matrix + release pipeline**
- spec：`.github/workflows/ci.yml` 加 build job：`with_quic with_wireguard with_grpc with_utls` tag，三平台产出 `resin-<target>`，作为 tauri-action externalBin 资源
  - 三 target：`windows-gui`、`linux-gui`、`macos-gui-arm64`+`macos-gui-x86_64`
  - 便携版本统一命名 `ai-api-route.exe|bin`
  - 后端 only 版本：Resin 单独 prebuilt binary + Rust headless wrapper（可选，是否保留看你）
- acceptance：CI 跑通，`release/` 出现 `windows-gui/ai-api-route-setup.exe` + `windows-gui/ai-api-route.exe`、`linux-gui/ai-api-route.deb`+AppImage、`macos-gui-arm64/ai-api-route.dmg`、`macos-gui-x86_64/ai-api-route.dmg`
- test：`scripts/verify-build.sh` 加 sidecar 二进制存在性守卫

### 次序与依赖
- G1 阻塞 G2/G3/G4
- G2 阻塞 G4（前端要 Resin REST 才能读数据）
- G3 可与 G2 并行
- G4 可与 G5 并行（G5 是 packaging，G4 是 UI）
- **第一个接手 agent 只需做 G1，做完即 push，下一个 G2 agent 接手**

### 不要做（明确禁区）
- 不要在 phase 4 内重写任何 sing-box outbound 协议栈（这是 C 路径的陷阱，A 路径托付给 Resin）
- 不要把 `resin` 二进制打进 webview 之外的位置（Tauri externalBin 是唯一允许的 sidecar 路径）
- 不要把 admin token 暴露给前端 IPC（`ResinClient` 只在 Rust 侧持有）
- 不要动你已经修好的 6 个 bugs
- 不要把 `crates/resin-core` 整个 crate 删掉——保留 mihomo.rs + resin_client.rs，其余文件保留为 DTO 数据类（Platform/Account/LaneSnapshot 的 struct 定义，避免影响 14 个 command 的签名），实际改实现到 forward

## 交接提示词（直接粘给下一个 Codex 窗口）

```
始终遵循 AGENTS.md，使用 ctx_*，必要时用 1mcp 的 exa/perplexity 联网搜索，不要产生幻觉推理。Ponytail full 模式。

背景在我的 ai-api-route 项目（D:\Aworker\ai-api-route）。决策依据见 docs/MEMORY_REUSE_DECISION.md，已定 A：fork Resin Go 二进制作为 Tauri sidecar。

你接 G1：vendor Resin binary + sidecar 骨架。详细 spec 见 docs/HANDOFF_PATH_A.md G1 段。

做完：
1. `git add` + commit `P4-G1: vendor Resin + sidecar boot skeleton`，push
2. `codegraph sync .`
3. 完成后总结剩余 G2/G3/G4/G5 状态给下一个窗口
4. 若 G2/G3 显然可顺手做，告知我后继做，但 G1 必须独立可验证的 commit

约束：不要动 crates/resin-core/src/mihomo.rs 之外的自实现 lane/lease/tdewma/platform；不要在 webview 暴露 admin token；按 AGENTS §8 不起子代理。
```

## 产物位置

- 本文件：`docs/HANDOFF_PATH_A.md`
- 决策复审固化为：`docs/MEMORY_REUSE_DECISION.md ## 决策复审：A vs C`
- 评估过程产物：`.omx/goals/autoresearch/go-vs-rust-path/`
