---
# jik-lsp-180m
title: 发布 tag 与预编译二进制
status: in-progress
type: task
priority: low
created_at: 2026-11-04T03:21:31Z
updated_at: 2026-11-05T01:55:46Z
parent: jik-lsp-u8g2
---

现状：git tag 为空，remote 为 github.com/morler/jik-lsp，无 release 产物。
做法：到 v0.2.0 打 tag，CI 产出 Linux/macOS/Windows 三平台二进制并挂到 release。
验收：release 页面可下载并可直接作为 jikPath 使用。
