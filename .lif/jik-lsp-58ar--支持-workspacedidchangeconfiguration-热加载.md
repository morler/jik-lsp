---
# jik-lsp-58ar
title: 支持 workspace/didChangeConfiguration 热加载
status: todo
type: task
priority: normal
created_at: 2026-11-04T03:21:32Z
updated_at: 2026-11-04T03:21:44Z
parent: jik-lsp-oueg
---

现状：src/server.rs:181-229 只在 initialize 读一次 initializationOptions，之后不可变。
做法：实现 did_change_configuration，复用同一套校验（含 jikPath 可执行性、entryPath 绝对路径），变更后使现有诊断失效并重检。
验收：改配置无需重启即可生效；非法配置被拒绝且保留旧配置；有回归测试。
