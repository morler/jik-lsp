---
# jik-lsp-qllk
title: 建立 GitHub Actions CI 门禁
status: completed
type: task
priority: high
created_at: 2026-11-04T03:21:31Z
updated_at: 2026-11-04T03:30:58Z
parent: jik-lsp-u8g2
---

现状：仓库无 .github 目录，fmt/clippy/test 全靠手工。
做法：.github/workflows/ci.yml，矩阵 ubuntu/macos/windows + stable Rust，步骤 cargo fmt --check、cargo clippy --all-targets -- -D warnings、cargo test。真实编译器用例在 CI 内跳过（无 ../jik）。
验收：PR 与 push 到 main 都触发；三平台全绿；失败时能定位到具体步骤。

## Summary of Changes

新增 .github/workflows/ci.yml：rustfmt、clippy -D warnings、三平台 cargo test 矩阵，push 到 main / PR / dispatch 触发，同 ref 并发取消。README 记录覆盖范围。首次运行暴露并修复了 lif 项 jik-lsp-p1b4 文件名非法 UTF-8 导致 macOS checkout 失败的问题（67f61e0）；修复后 run 41767383267 五个 job 全绿（ubuntu/macos/windows + clippy + rustfmt）。
