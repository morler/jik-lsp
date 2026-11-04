---
# jik-lsp-8dqv
title: 补齐 lint 配置与依赖版本约束
status: completed
type: task
priority: low
created_at: 2026-11-04T03:21:31Z
updated_at: 2026-11-04T08:42:13Z
parent: jik-lsp-u8g2
---

现状：Cargo.toml 无 [lints] 段，依赖用宽松的 "1"/"0.20"；tower-lsp 0.20 已滞后。
做法：加 [lints.rust]/[lints.clippy] 基线，评估升级 tower-lsp 的可行性（记录结论即可，不强制升级）。
验收：cargo clippy -D warnings 通过；升级评估结论写入本项正文。

## Summary of Changes

Cargo.toml 配置 Rust unsafe_code=forbid 与 Clippy all=warn；将直接依赖下限设为当前验证版本，并将 Cargo.lock 中不兼容 Rust 1.85 的 ICU 与 idna_adapter 回退到兼容版本。CI 使用 --locked 并增加 Rust 1.85 全目标 cargo check。验证：cargo fmt --all --check、stable cargo clippy --locked --all-targets -- -D warnings、cargo test --locked --all-targets --no-fail-fast（含本机真实 jik 编译器）、cargo +1.85.0 check --locked --all-targets 均通过。升级评估：原 tower-lsp 最新版本仍为 0.20.0，上游最近推送为 2024-08-15；tower-lsp-f 0.26.0 是活跃 fork，提供类似 API，但未声明 MSRV，切换需要评估协议类型兼容性并回归测试，当前不迁移。Rust 1.85 Clippy 的 zombie_processes 对测试线程闭包 return 误报，实际子进程由 Harness::drop 回收；最低版本 CI 检查编译，stable CI 负责 Clippy。
