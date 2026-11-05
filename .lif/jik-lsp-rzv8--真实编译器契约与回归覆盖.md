---
# jik-lsp-rzv8
title: 真实编译器契约与回归覆盖
status: completed
type: epic
priority: high
created_at: 2026-11-04T03:21:31Z
updated_at: 2026-11-05T02:13:01Z
parent: jik-lsp-ofci
---

用真实编译器与真实错误报文建立契约测试，消除「靠假脚本假设报文格式」的死角。

## Summary of Changes

真实编译器 e2e 契约测试常驻 tests/real_compiler.rs（6 项，含 UTF-8 字节列转 UTF-16、EOF 处缺 main、含空格与冒号的路径、解析错误逐条报告而语义错误遇首个未定义符号即停、有效源码不发布诊断）；无法定位的诊断已降级为文件级诊断而非丢弃；initialize 阶段校验 jikPath 与 entryPath。Windows 真实覆盖已由用户决定暂缓并移出本史诗（jik-lsp-3gfu 独立保留）。
