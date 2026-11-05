---
# jik-lsp-u8g2
title: CI 与发版自动化
status: completed
type: epic
priority: high
created_at: 2026-11-04T03:21:31Z
updated_at: 2026-11-05T02:12:34Z
parent: jik-lsp-ofci
---

把 fmt/clippy/test 变成强制门禁，并产出可下载的发布二进制。

## Summary of Changes

fmt/clippy/MSRV 1.85/三平台测试已作为 CI 门禁常驻（.github/workflows/ci.yml），v0.2.0 已打 tag 并由 .github/workflows/release.yml 产出三平台预编译二进制挂到 GitHub Release，release 页面资产可下载并直接作为 jikPath 使用。
