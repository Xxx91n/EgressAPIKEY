# Strategy Single Source of Truth: strategyConfig JSON over direct Resin PATCH

Q5 决策：画布拖线改策略不再直接 `ipcPlatformUpdate` PATCH Resin，改为更新 strategyConfig JSON -> `ipcStrategyConfigPut` -> `ipcStrategyApply` -> 改 Resin。白盒配置文件是所有策略写入的唯一管道，Resin sidecar 是最终权威源但不是写入入口。

Rejected alternative: 直接 PATCH Resin（绕过 JSON 文件）。违反用户心智模型"所有配置都在白盒配置文件中能修改"。

Consequence: 画布拖线路径多一跳（JSON -> apply -> Resin），但确保白盒配置文件始终是最新的，用户可以直接编辑 JSON 文件改策略。
