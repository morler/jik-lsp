---
# jik-lsp-uxgz
title: 无法定位的诊断降级为文件级诊断
status: in-progress
type: bug
priority: normal
created_at: 2026-11-04T03:21:31Z
updated_at: 2026-11-04T07:36:03Z
parent: jik-lsp-rzv8
---

现状：src/server.rs:648-655 在 convert_diagnostics 里遇到 compiler_position 失败只 log warning 并 continue，错误对用户彻底消失。
做法：降级为该文件的文件级诊断（range 取 0:0 或整文件），message 保留原文并标注「位置未定位」。
验收：构造无法定位的报文时客户端仍收到 Diagnostic 而非仅日志；现有日志保留以便排错。
