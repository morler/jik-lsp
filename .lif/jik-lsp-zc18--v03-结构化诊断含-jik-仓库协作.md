---
# jik-lsp-zc18
title: v0.3 结构化诊断（含 jik 仓库协作）
status: todo
type: milestone
priority: normal
created_at: 2026-11-04T03:21:31Z
updated_at: 2026-11-04T03:21:31Z
---

目标：摆脱对人可读文本的前缀匹配，让 LSP 拿到带 span 起止的结构化诊断。
完成标准：jik check 提供 JSON 输出；LSP 消费 JSON 产生带真实范围与 relatedInformation 的诊断；文本解析路径保留为回退。
