---
# jik-lsp-xch7
title: LSP 消费 JSON 诊断并生成真实 range
status: todo
type: task
priority: normal
created_at: 2026-11-04T03:21:31Z
updated_at: 2026-11-04T03:21:31Z
parent: jik-lsp-v137
blocked_by:
    - jik-lsp-s79h
---

做法：新增 JSON 解析路径，range 用 span 起止（UTF-8 字节列经编码转换，文档变更仍用 UTF-16）；保留 src/diagnostics.rs 文本解析作为旧版本编译器的回退，按 --json 支持与否自动探测。
验收：真实 span 的多字符错误在编辑器里高亮整段而非零宽点；旧编译器仍可工作；两套解析均有测试。
