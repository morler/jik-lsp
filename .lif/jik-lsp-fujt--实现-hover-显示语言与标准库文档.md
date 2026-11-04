---
# jik-lsp-fujt
title: 实现 hover 显示语言与标准库文档
status: todo
type: task
priority: low
created_at: 2026-11-04T03:21:32Z
updated_at: 2026-11-04T03:21:44Z
parent: jik-lsp-nql1
---

数据源：../jik/docs/help-index.txt 与 jik help <topic> 输出（已验证可用）。
做法：按光标位置取标识符，查 help 索引，返回 Markdown hover；未命中返回空而非报错。
验收：内置类型与标准库符号 hover 有内容；无命中无副作用；有单元测试。
