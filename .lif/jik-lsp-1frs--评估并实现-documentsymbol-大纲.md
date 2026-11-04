---
# jik-lsp-1frs
title: 评估并实现 documentSymbol 大纲
status: todo
type: task
priority: low
created_at: 2026-11-04T03:21:32Z
updated_at: 2026-11-04T03:21:44Z
parent: jik-lsp-nql1
---

先评估：jik tran --dump-ast 输出格式是否稳定可用作符号来源（../jik/jik help tran 已确认 --dump-ast 存在）。
做法：评估结论先写入本项，再决定基于 AST 转储还是自行轻量扫描实现函数/类型/全局符号大纲。
验收：评估结论落地；若实现，大纲可被编辑器展示且不阻塞主线程。
