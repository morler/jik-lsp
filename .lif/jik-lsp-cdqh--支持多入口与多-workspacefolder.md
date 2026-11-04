---
# jik-lsp-cdqh
title: 支持多入口与多 workspaceFolder
status: todo
type: task
priority: normal
created_at: 2026-11-04T03:21:32Z
updated_at: 2026-11-04T03:21:44Z
parent: jik-lsp-oueg
---

现状：state.entry 是单个 Option<PathBuf>，多模块项目必须为每个项目各起一个实例。
做法：允许 entryPath 为数组或按 workspaceFolder 配置，检查任务按入口分桶，诊断互不覆盖。
验收：两个入口文件的诊断可同时存在；单入口行为不变；有回归测试。
