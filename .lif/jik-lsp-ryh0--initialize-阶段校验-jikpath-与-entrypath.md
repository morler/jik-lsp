---
# jik-lsp-ryh0
title: initialize 阶段校验 jikPath 与 entryPath
status: todo
type: task
priority: normal
created_at: 2026-11-04T03:21:31Z
updated_at: 2026-11-04T03:21:31Z
parent: jik-lsp-rzv8
---

现状：src/server.rs:199-207 只校验 entryPath 是否绝对路径；jikPath 不存在、非可执行、entryPath 指向不存在的文件都要等到每次保存才失败并重复打日志。
做法：initialize 时做一次存在性/可执行性检查，失败则返回明确的 invalid_params 或初始化后一次性 ERROR 日志 + 可读提示。
验收：三类错误各有对应测试与明确文案；错误不会在每次保存时重复刷屏。
