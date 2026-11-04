---
# jik-lsp-ryh0
title: initialize 阶段校验 jikPath 与 entryPath
status: completed
type: task
priority: normal
created_at: 2026-11-04T03:21:31Z
updated_at: 2026-11-04T07:46:10Z
parent: jik-lsp-rzv8
---

现状：src/server.rs:199-207 只校验 entryPath 是否绝对路径；jikPath 不存在、非可执行、entryPath 指向不存在的文件都要等到每次保存才失败并重复打日志。
做法：initialize 时做一次存在性/可执行性检查，失败则返回明确的 invalid_params 或初始化后一次性 ERROR 日志 + 可读提示。
验收：三类错误各有对应测试与明确文案；错误不会在每次保存时重复刷屏。

## Summary of Changes

initialize 时一次性校验：显式 jikPath 必须存在且可执行（unix 查可执行位，含分隔符按给定路径、裸名称查 PATH）；entryPath 必须是存在的绝对路径；失败返回 invalid_params 并指明路径。省略 jikPath 时不校验，保留未装编译器也能启动的场景。新增 tests/initialize.rs 7 项；三个既有 entry 用例改为先建 entry 文件再启动 harness。全套 39 项通过，CI run 41794359224 全绿。
