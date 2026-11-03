---
# jik-lsp-a5k6
title: 修复 ocr 评审发现的并发与重检问题
status: in-progress
type: task
created_at: 2026-11-03T03:14:03Z
updated_at: 2026-11-03T03:14:03Z
---

按 ocr 评审意见修正：回滚无收益的持锁复查、注释与实现不一致、generations 无界增长、保存与监听重复触发编译、watcher 扇出 O(N²) 发布，并补充 entryPath 分支测试。
