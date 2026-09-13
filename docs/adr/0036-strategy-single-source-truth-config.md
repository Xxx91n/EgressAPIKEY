# Strategy Single Source of Truth: strategyConfig JSON over direct Resin PATCH
Status: ACCEPTED


Q5 决策：画布拖线改策略不再直接 `ipcPlatformUpdate` PATCH Resin，改为更新 strategyConfig JSON -> `ipcStrategyConfigPut` -> `ipcStrategyApply` -> 改 Resin。白盒配置文件是所有策略写入的唯一管道，Resin sidecar 是最终权威源但不是写入入口。

Rejected alternative: 直接 PATCH Resin（绕过 JSON 文件）。违反用户心智模型"所有配置都在白盒配置文件中能修改"。

Consequence: 画布拖线路径多一跳（JSON -> apply -> Resin），但确保白盒配置文件始终是最新的，用户可以直接编辑 JSON 文件改策略。

---

## Addendum: Read-side specification (ADR-0039 SS2)

The *read* side of the strategyConfig pipeline (how the GUI canvas consumes the JSON) is now specified by [ADR-0039](0039-canvas-v3-fold-center-strategy-sync.md) SS2. Key points:

- `TopologyView.tsx` `sync()` merges ALL `PlatformStrategy` fields (`a_class`, `b_class`, `manual_nodes`, `subscriptions`, `top_n`) from `cfgRaw.platforms` into the `PlatformFull` interface, not just `regions`.
- `aClassLabel` is computed via a switch on `p.aClass` (not a `region_filters.length` ternary), so `subscription` and `quality` strategies render correct badge text.
- `bClassLabel` prefers `p.bClass` (strategyConfig) over `mapResinToShell(p.allocation_policy)` (Resin enum).

The *write* side (`strategy_config_put` + `strategy_apply` -> PATCH Resin `regex_filters` / `region_filters` / `allocation_policy`) remains unchanged. This addendum does not reverse the write-side decision; it only specifies the read-side contract that was previously implicit.
