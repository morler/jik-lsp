---
# jik-lsp-3gfu
title: Windows 下的编译器替身与并发测试覆盖
status: todo
type: bug
priority: deferred
created_at: 2026-11-04T03:21:31Z
updated_at: 2026-11-05T02:12:55Z
---

跨平台支持暂缓（用户 2026-11-04 决定：当前只做 Linux 环境适配）。

现状：tests/diagnostic_concurrency.rs:1 整个文件 #![cfg(unix)]；tests/lifecycle.rs:84 的编译器测试也只在 unix 运行。这在 Windows 上是零覆盖，但在 Linux-only 阶段不成问题——unix 条件本身已覆盖 Linux 与 macOS。

做法（恢复时）：用 Rust 编译期生成的替身程序（或 tests/ 下的小 bin target）替换 #!/bin/sh printf 脚本，让替身由环境变量决定输出报文与退出码。
验收（恢复时）：除符号链接/设备号相关用例外，其余测试在 Windows 上运行；CI 的 windows-latest job 全绿。

另注：CI 的 windows-latest job 目前仍保留并是绿的（编译通过+测试通过），只是并发/诊断链路的真实覆盖仅限于 unix 条件内的用例。

范围调整（收尾 v0.2 时）：用户已决定当前只做 Linux 适配，此项从 v0.2 的真实编译器契约史诗移出，作为独立的跨平台待办保留，恢复时再挂回相应里程碑。
