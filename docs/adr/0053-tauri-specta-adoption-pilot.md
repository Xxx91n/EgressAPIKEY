# ADR-0053: tauri-specta 采纳试点 — 仅类型层采纳,分期迁移

Status: PROPOSED (检查点 B 待用户拍板 — 三选一结论在 §Decision,推荐方向已按 handoff 默认先行)
> Date: 2026-08-30
> Ticket: architecture-recovery 09 (tauri-specta-pilot)
> Related: ADR-0045 (IPC 错误契约 — IpcError 判别联合由此进入 bindings.ts)、
> ADR-0043 (headless 双模 — 生成的 runtime 包装不能替代 ipc.ts,见 §Consequences)、
> AGENTS.md §7.5/§7.6 (边界校验与 IPC 清单守卫)、ADR-0052 (票 10 已占用该编号)

## Context

类型契约今天有三个手抄源:Platform/Account/NodeInfo 类型在
`src/store/appStore.ts`、`src/lib/ipc.ts`、`crates/resin-core/src/platform.rs`
三处平行维护(2026-08-29 调研实测);31 个 IPC 返回 unknown;Rust 侧重命名
或改字段时 TS 侧编译期零感知,漂移只能在运行时暴露。

2026-08-29 atomcode 调研(18 信源)+ 本票实施期间源码复核,生态事实:

- **tauri-specta 2.0.0-rc.25**(2026-05-08)是 v2 最新发布;stable 无时间表;
  docs.rs 对 rc.25 构建失败(文档停在 rc.21);main 分支已用未发布的
  specta rc.26-dev。生态规模 ~17.7 万月下载、785★、Handy 30.6k★ 生产使用。
- 三件套必须 `=` 锁版本(tauri-specta =2.0.0-rc.25 + specta =2.0.0-rc.25 +
  specta-typescript =0.0.12),beta 期不用 `=` 会在未来更新时破坏。
- 命令参数 ≤10(crates.io 官方描述;tauri 维护者口径 ~16,保守按 10)。
  本仓 `port_upsert` 7 个非 State 参数,达标;`port_toggle` 2 个。
- **specta-typescript 0.0.12 没有 bigint 导出策略旋钮**(那是后续版本的
  `.bigint(BigIntExportBehavior)` API),且默认**禁止** u64/i64/usize 导出
  (`bigint_forbidden` 编译期错误)。唯一出路是字段级
  `#[specta(type = u32)]` 覆盖。
- 默认 `ErrorHandlingMode::Result`:前端生成
  `typedError<T,E>` 包装(`{status:"ok",data}|{status:"error",error}`)。

### 试点实施中实测的坑(调研未覆盖,源码+链接器证据定因)

1. **comctl32 v6 manifest 坑**:凡单型态化 `Builder<R>` 的测试二进制都会
   静态拉入 muda/tao 的 comctl32 v6 导入(`SetWindowSubclass`、
   `TaskDialogIndirect`);tauri_build 只给 bin 目标嵌 SxS manifest,
   lib 测试 harness 没有 → 加载器绑到 comctl32 v5 →
   `STATUS_ENTRYPOINT_NOT_FOUND (0xC0000139)`,测试进程启动即死、零输出。
   定因证据:新旧测试 exe 导入表 diff(新:comctl32/user32/gdi32/shell32;
   旧:无)、无 manifest、直接运行 exit 127。
   **解法**:导出测试放集成测试(`[[test]]` target),build.rs 用
   `cargo:rustc-link-arg-tests=/MANIFEST:EMBED` + `/MANIFESTINPUT` 注入
   manifest(src-tauri/build.rs);lib 单测 harness 不再单型态化 Builder。
2. **MockRuntime 路线**:tauri::test::MockRuntime(tauri 自家无头测试运行时)
   单型态化导出测试,不开真窗口;生产 invoke_handler(采纳后)才用 Wry。

## Decision(三选一,检查点 B)

**推荐:选项 2 — 仅类型层采纳,分期按域迁移**(本票已按此落地试点)。

| 选项 | 内容 | 结论 |
|---|---|---|
| 1 全量采纳 | 65+ 命令全注解,`collect_commands!` 替换 `generate_handler!`,前端全量切 bindings 调用 | 不推荐。rc 版宏一次替换全注册表,与 headless 双模(ADR-0043:ipc.ts 的 trace_id 注入 + CMD_TO_HTTP 浏览器回退)冲突面一次铺满;回滚粒度是全部 65 命令 |
| **2 仅类型层采纳(推荐)** | 命令逐域加 `#[specta::specta]` + 类型 derive,bindings.ts 入库;前端**只 import 生成类型**,运行时调用留在 ipc.ts(保留 trace_id + headless CMD_TO_HTTP) | **试点已验证可行**(本票)。类型契约单源化,零运行时行为变化 |
| 3 放弃 | 理由留档防重提 | 不必要:rc.25 在本仓真实编译通过且生成物正确,痛点真实(三处手抄) |

**为什么选项 2 的"只 import 类型"是对的**:生成的 runtime 包装
(`commands.gatewaySnapshot()`)绕开 ipc.ts 的 trace_id 注入(ADR-0026)
与 headless 浏览器回退(ADR-0043 Q2=A),直接采纳会把这两个特性打断。
类型 import 是零风险的;runtime 切换必须先解决双模问题(后续票)。

### 试点范围(已落地,3 命令,避开 strategy 域)

- `gateway_snapshot`(纯读快照,返回 LaneSnapshot)— platform.rs
- `set_log_level`(参数从 String 收紧为 `LogLevel` 枚举,wire 不变) — settings.rs
- `port_toggle`(错误路径丰富,返回 PortMapping + IpcError) — ports.rs

IpcError(resin-core/ipc_error.rs)与 PortMapping(resin-core/db.rs)加
`cfg_attr(feature = "specta", derive(specta::Type))`,resin-core 侧
specta 为 optional 依赖,headless-only 构建零成本。

### 分期迁移建议(后续票清单,采纳确认后立项)

1. **diagnostics 域**(5 命令,返回多为 serde_json::Value → 先定型再注解)
2. **backup 域**(5 命令)
3. **ports 域剩余 15 命令**(port_upsert 7 参达标)
4. **platform 域剩余 22 命令**(LaneSnapshot 已通,大头是 PlatformFull/NodeInfo
   三处手抄类型的核心收益区)
5. **strategy 域**(5 命令;与票 10 的 StrategyService 深模块收口后做)
6. **settings 域剩余 10 命令**(多为轻量)

每批一个 commit:Rust 注解 + cargo test 再生成 + ipc.ts import 切换 +
ipc-manifest-check 仍绿。全部完成后评估是否把 65 条 `generate_handler!`
换成 `builder.invoke_handler()`(那时才有理由动注册表)。

## Consequences

- bindings.ts 入库提交;再生成是确定性单命令
  (`cargo test -p egressapikey-app --test bindings_export`),CI 可加
  drift 检查(diff 再生成结果,或直接跑该测试——它每次重写后断言内容)。
- 构建顺序约束(验收清单第 3 项):bindings 生成(cargo test)**先于**
  前端编译(tsc);类型漂移时 tsc 直接红,这正是想要的守卫。CI 无需
  新前置步骤——verify-build.sh 的顺序(cargo test → tsc)天然满足;
  若未来把 bindings 排除出版本库,才需要 CI 前置生成步骤。
- usize/u64 字段需 `#[specta(type = u32)]` 覆盖(LaneSnapshot 4 字段已加);
  若未来某类型携带真 64 位值(>2^32),覆盖成 u32 会在生成层截断**类型**而非
  运行时值——迁移各域时逐字段核对。
- `set_log_level` 参数收紧为枚举是试点里唯一的行为面变化:越界值从
  "Rust 命令内 match 返回 Err" 变为 "serde 反序列化失败(Tauri 层报错)"。
  wire 格式不变(lowercase);前端 ipcSetLogLevel 的 TS 侧守卫保留未动。
- rc 风险敞口:三件套 `=` 锁死;升级到 stable(无时间表)时需再评估。
  docs.rs 无 rc.25 文档这一事实已被"本仓编译+生成物断言"补强。
- comctl32 v6 manifest 注入(build.rs)对 CI 的 linux/macos 构建零影响
  (`CARGO_CFG_TARGET_ENV` 只在 msvc 生效)。
- ipc-manifest 守卫(票 03)与本方案正交:它守"命令名=注册表",
  bindings 守"类型=契约",两层都过才算对齐。

## Verification(本票证据)

- cargo test -p egressapikey-app --test bindings_export → 1 passed
  (再生成 bindings.ts + 断言 7 个契约符号在档)
- cargo test -p egressapikey-app --lib → 91 passed(含 LogLevel 2 个新测试)
- cargo test -p resin-core --lib → 145 passed
- vitest 17 文件 305/305(含 ipc.test.ts 3 个试点回归)
- npx tsc -b → 零错误(ipc.ts 删除两处手抄、import 生成类型)
- verify-build.sh → EXIT 0
- P23:chunk Bbc8jnaQ 在 staged exe offset 11336018 命中;冒烟
  TITLE=EgressAPIKEY / HWND≠0 / WS 34MB / stderr 0 字节
- main.rs generate_handler 注册表零改动(git diff 无 main.rs)

> **Closeout note (2026-08-30, brain review):** Checkpoint B went unanswered at round closeout;
> per the tracker governance default (README §3.2) the recommended Option 2 (type-layer adoption)
> was executed and landed on origin/main (commit 0c24d56). This ADR stays PROPOSED until the
> user confirms. Reversal window: revert commit 0c24d56 on a new branch (deps + bindings.ts +
> build.rs; runtime behavior is unchanged either way — type imports only). If confirmed, flip
> Status to ACCEPTED and follow the phased migration list in §Decision.
