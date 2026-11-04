---
# jik-lsp-8dqv
title: 补齐 lint 配置与依赖版本约束
status: todo
type: task
priority: low
created_at: 2026-11-04T03:21:31Z
updated_at: 2026-11-04T03:21:31Z
parent: jik-lsp-u8g2
---

现状：Cargo.toml 无 [lints] 段，依赖用宽松的 "1"/"0.20"；tower-lsp 0.20 已滞后。
做法：加 [lints.rust]/[lints.clippy] 基线，评估升级 tower-lsp 的可行性（记录结论即可，不强制升级）。
验收：cargo clippy -D warnings 通过；升级评估结论写入本项正文。
