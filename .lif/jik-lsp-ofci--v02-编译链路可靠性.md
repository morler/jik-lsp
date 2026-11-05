---
# jik-lsp-ofci
title: v0.2 编译链路可靠性
status: completed
type: milestone
priority: high
created_at: 2026-11-04T03:21:31Z
updated_at: 2026-11-05T02:13:01Z
---

目标：让诊断链路在真实编译器、真实平台、真实 CI 上被证明可靠，并把「无法解析/无法定位」的错误降到最低。
完成标准：真实 jik 编译器的 e2e 契约测试常驻；CI 在 Linux/macOS/Windows 跑 fmt+clippy+test；无法定位的诊断有降级路径；配置错误在 initialize 时立即报错。

## Summary of Changes

四项完成标准均已达成：真实 jik 编译器 e2e 契约测试常驻；CI 在 Linux/macOS/Windows 跑 fmt+clippy+test 并追加 MSRV 1.85 检查；无法定位的诊断有文件级降级路径且标注 position not located；initialize 阶段即时校验 jikPath 与 entryPath。v0.2.0 已打 tag 并产出三平台预编译二进制 Release。跨平台真实覆盖（jik-lsp-3gfu）按用户决定暂缓，独立保留在里程碑之外。
