---
# jik-lsp-180m
title: 发布 tag 与预编译二进制
status: completed
type: task
priority: low
created_at: 2026-11-04T03:21:31Z
updated_at: 2026-11-05T02:12:07Z
parent: jik-lsp-u8g2
---

现状：git tag 为空，remote 为 github.com/morler/jik-lsp，无 release 产物。
做法：到 v0.2.0 打 tag，CI 产出 Linux/macOS/Windows 三平台二进制并挂到 release。
验收：release 页面可下载并可直接作为 jikPath 使用。

## Summary of Changes

新增 .github/workflows/release.yml：推送 v* tag 时在 ubuntu/macos/windows 三平台构建 release 二进制，打包为 jik-lsp-<tag>-<platform>.tar.gz（含 README/LICENSE）并附到 GitHub Release；手动触发只产出运行产物。Cargo.toml/Cargo.lock 版本升到 0.2.0，.gitignore 增加 /dist/，README 补充发布说明。修复 publish 任务缺少 checkout 导致 gh 报 fatal: not a git repository，改为显式 --repo。已推 main（37c16a0）并打 v0.2.0 tag；CI 与 Release 均 success，Release 页面三个平台包可下载，Linux 包解包后二进制 initialize 握手返回 serverInfo.version=0.2.0。
