---
# jik-lsp-a5k6
title: 修复 ocr 评审发现的并发与重检问题
status: completed
type: task
priority: normal
created_at: 2026-11-03T03:14:03Z
updated_at: 2026-11-03T03:19:39Z
---

按 ocr 评审意见修正：回滚无收益的持锁复查、注释与实现不一致、generations 无界增长、保存与监听重复触发编译、watcher 扇出 O(N²) 发布，并补充 entryPath 分支测试。

## Summary of Changes

回滚无收益的持锁复查；start_check 与调用方职责分离、watcher 单次发布；关闭文档清理 per-entry 状态；entryPath 下跳过自身保存回声；新增 entryPath 监听测试。
