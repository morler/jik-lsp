---
# jik-lsp-qllk
title: 建立 GitHub Actions CI 门禁
status: todo
type: task
priority: high
created_at: 2026-11-04T03:21:31Z
updated_at: 2026-11-04T03:21:31Z
parent: jik-lsp-u8g2
---

现状：仓库无 .github 目录，fmt/clippy/test 全靠手工。
做法：.github/workflows/ci.yml，矩阵 ubuntu/macos/windows + stable Rust，步骤 cargo fmt --check、cargo clippy --all-targets -- -D warnings、cargo test。真实编译器用例在 CI 内跳过（无 ../jik）。
验收：PR 与 push 到 main 都触发；三平台全绿；失败时能定位到具体步骤。
