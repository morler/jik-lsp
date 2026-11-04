---
# jik-lsp-3gfu
title: Windows 下的编译器替身与并发测试覆盖
status: todo
type: bug
priority: high
created_at: 2026-11-04T03:21:31Z
updated_at: 2026-11-04T03:21:31Z
parent: jik-lsp-rzv8
---

现状：tests/diagnostic_concurrency.rs:1 整个文件 #![cfg(unix)]，tests/lifecycle.rs:84 的编译器测试也只在 unix 运行，Windows 上诊断链路零覆盖。
做法：用 Rust 编译期生成的替身程序（或 tests/ 下的小 bin target）替换 #!/bin/sh printf 脚本，让替身由环境变量决定输出报文与退出码。
验收：除符号链接/设备号相关用例外，其余测试在 Windows 上运行；CI 的 windows-latest job 全绿。
