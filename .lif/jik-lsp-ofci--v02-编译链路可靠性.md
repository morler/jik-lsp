---
# jik-lsp-ofci
title: v0.2 编译链路可靠性
status: todo
type: milestone
priority: high
created_at: 2026-11-04T03:21:31Z
updated_at: 2026-11-04T03:21:31Z
---

目标：让诊断链路在真实编译器、真实平台、真实 CI 上被证明可靠，并把「无法解析/无法定位」的错误降到最低。
完成标准：真实 jik 编译器的 e2e 契约测试常驻；CI 在 Linux/macOS/Windows 跑 fmt+clippy+test；无法定位的诊断有降级路径；配置错误在 initialize 时立即报错。
