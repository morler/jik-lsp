---
# jik-lsp-s79h
title: jik check 增加 --json 结构化诊断输出
status: todo
type: task
priority: normal
created_at: 2026-11-04T03:21:31Z
updated_at: 2026-11-04T03:21:31Z
parent: jik-lsp-v137
---

归属仓库：../jik（不是本仓库）。
现状：诊断只由 ../jik/src/jik/diag.jik:63 起 append_message 渲染成人可读文本，span 结束位置被丢弃，LSP 只能拿到零长度点（src/server.rs:634）。
做法：--json 输出每条诊断含 path、span.start/end（行、UTF-8 字节列）、level、summary、detail、related 位置。
验收：jik check --json 在 jik 仓库自测通过；字段稳定并写入 docs/cli.md；无 --json 时输出不变。
