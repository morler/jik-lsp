---
# jik-lsp-uxgz
title: 无法定位的诊断降级为文件级诊断
status: completed
type: bug
priority: normal
created_at: 2026-11-04T03:21:31Z
updated_at: 2026-11-04T07:40:40Z
parent: jik-lsp-rzv8
---

现状：src/server.rs:648-655 在 convert_diagnostics 里遇到 compiler_position 失败只 log warning 并 continue，错误对用户彻底消失。
做法：降级为该文件的文件级诊断（range 取 0:0 或整文件），message 保留原文并标注「位置未定位」。
验收：构造无法定位的报文时客户端仍收到 Diagnostic 而非仅日志；现有日志保留以便排错。

## Summary of Changes

convert_diagnostics 遇到位置无法映射时不再 continue 丢弃：改为下发 range 0:0 的文件级诊断，消息保留原文并追加「position not located」及编译器原始行号与字节列，同时保留日志警告。新增集成测试 unlocatable_diagnostic_falls_back_to_the_start_of_the_file 覆盖行号越界与列越界，并确认日志先于发布到达的顺序。README 同步说明降级行为。全套 32 项测试通过，CI run 41793736891 全绿。
