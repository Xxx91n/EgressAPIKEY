# 我改了配置为什么没生效(排障表)

> Audience: EgressAPIKEY users. The authoritative check is the Effective
> Config view (sidebar entry 生效配置 / Effective Config). Every scenario
> below ends with "open that view and press 立即复验 (re-check)" — the
> three-state badges there are the fact, this table is the interpretation.
> Contract: ADR-0054 (reconciliation loop) + ADR-0051 (snapshot semantics).

## 快速路径

1. Open the Effective Config view from the sidebar.
2. Press the manual re-check button (立即复验) once — it re-reads all three
   layers (whitebox strategy, whitebox ports, Resin runtime) and re-merges.
3. Read the badge per platform/port:

| Badge | Meaning | Next step |
| --- | --- | --- |
| 一致 consistent | whitebox and Resin runtime agree | Nothing to do |
| 漂移 divergent | both sides readable but disagree | See scenario 1 or 2 |
| 缺失 missingOnResin | entity absent on the Resin side | See scenario 2 or 3 |
| 已知 known (grey) | drift exists but you marked it acknowledged | Revoke the exemption in the whitebox if unintended |

## 场景 1:改了 strategy 没生效

**症状**:在 GUI 或 `egressapikey-strategy.json` 里改了平台 region/A-class,
画布或请求行为没变化;Effective Config 里该平台显示 漂移 divergent。

**原因**:strategy 白盒的写入(`strategy_config_put`)只更新 L2 文件;
把它推向 Resin 运行时的是**另一步显式的 apply**(strategy_apply / 深度
region 编辑 /「同步到期望态」)。只改文件不 apply,L3 不会自己变。

**修复**:

- 打开 Effective Config,点「同步到期望态」:预演弹窗会列出将要 PATCH 的
  平台,确认后执行 strategy_apply 并自动复验(单向:白盒必胜)。
- 或者继续用画布的拖拽编辑(它内部走同一个 apply 入口)。

**如果 apply 后仍 divergent**:看错误提示(ADR-0045 类型化错误);常见是
sidecar 未运行——先到诊断页确认 sidecar 状态再重试。

## 场景 2:改了 ports 没生效

**症状**:新增/启用了入口端口,但上游网关连不上;Effective Config 里该
端口显示 缺失 missingOnResin。

**原因**:端口白盒(`egressapikey-ports.json`)是唯一事实源,但 Resin
监听器是派生状态。写入白盒后需要把它 re-assert 到 Resin(正常路径是
创建端口时的自动 POST;sidecar 重启后由启动路径 restore 补)。

**修复**:

- 确认 sidecar 正在运行(缺失徽标在 sidecar 下线时是中性灰,不算漂移)。
- 点「同步到期望态」:reconcile 会按白盒重新 POST 缺失的 endpoint
  (409=已存在视为满足),完成后自动复验。
- 仍然失败则看诊断页 sidecar 日志(环形缓冲 500 行)与该端口的健康检查。

## 场景 3:重启后回退了

**症状**:重启应用后发现某个改动「消失了」或回到了旧值。

**原因与区分**:

- **L3 是派生层**:sidecar 重启不保存运行时状态;启动时从白盒 restore。
  你如果只改了运行时(例如在 Resin API 层面),重启后被白盒覆盖是**设
  计行为**,不是回退。
- **白盒是唯一事实源**:白盒文件的改动重启后一定还在。如果你改的是白盒
  而重启后丢了,说明那次写入没有走白盒写入口(这算 bug,请提 issue)。
- **divergentSince 时间戳重启清零**:漂移计时的记忆是进程内的,重启后
  重新计时——时间戳变小不是数据回退。

**修复**:在 Effective Config 里对照 期望态|实况 两列,确认你想保留的值
到底写在白盒里没有;没有就回编辑器改白盒(GUI 或 JSON),再走场景 1/2 的
apply/reconcile。

## 场景 4:通知为什么只弹一次

**症状**:系统通知「检测到配置漂移」出现过一次,之后明明还在漂移却不再
弹。

**原因(设计行为,ADR-0054 §E)**:托盘通知是**每进程一次**的状态机:

- 首次出现「未豁免漂移」时弹一次,同进程内不重复弹(不打循环)。
- 归零后重新武装:当一次快照显示**零条未豁免漂移**时,状态机重新武装,
  下一轮新漂移会再弹一次。
- 已知豁免(acknowledged)条目永不触发通知——它们在视图里是灰色「已知」
  徽标,但不算「未豁免漂移」。
- sidecar 下线时不弹(没有运行时可比对,缺失不算漂移)。
- 进程重启后状态机重新武装(与 divergentSince 重置同语义)。

**想让新漂移再弹一次**:把当前漂移处理到归零(点「同步到期望态」或修正
白盒),或对已知漂移条目做豁免;之后的新漂移会重新触发通知。

## 场景 5:配了进程路由,目标进程的流量没走代理

**症状**:在「进程路由」页添加了 `xxx.exe → :17990`,但该进程的流量并未经由
所选出口;Effective Config 里路由条目照常显示 一致 consistent。

**原因(设计行为,ADR-0055 D3)**:进程路由是**带漂移告警的声明式备忘**,不是
流量拦截器——规则只记录「进程名 → 入口端口」的意图并核对目标端口的存活度,
壳层不会自动接管该进程的连接。Resin 没有进程组 API,一条路由的存活度 = 它
指向的入口端口的存活度(端口有监听器即 consistent)。

**修复**:让目标进程自己的代理设置指向所选端口——入口地址
`127.0.0.1:<端口>`(HTTP_PROXY/HTTPS_PROXY 环境变量、应用内代理设置或系统
代理)。指向之后该进程的流量才会经 Resin 出口;路由条目的三态仍按端口存活度
报告,不反映该进程是否真的在用代理。

**升级路径(留档)**:若 Resin 上游未来支持进程组,快照缝已按「L3 侧归一化
名称集合」塑形(`ProcessRouteSnapshot` / `merge_routes`,snapshot.rs 的
process-group echo 注释),届时把数据源从端口监听器回显换成进程组注册表即可
完成无痛升级的半执行;真正的进程级流量拦截属独立 round,与 Resin fork 决策点
同级(D-005)。

## 相关文档

- 生效配置视图契约:ADR-0051(authoritative snapshot)
- 调和闭环(单向 reconcile / 版本化 / 通知一次):ADR-0054
- 配置权威三层模型:`docs/architecture/ARCHITECTURE.md` § Config Authority
- 白盒回滚:生效配置视图内的历史区(每次写入自动备份,保留最近 10 份)
- 进程路由 vs 账号头规则:`docs/architecture/PROCESS_ROUTE_VS_HEADER_RULES.md`
