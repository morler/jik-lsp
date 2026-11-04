---
# jik-lsp-ivp1
title: 利用 span 提供 relatedInformation 与诊断细节
status: todo
type: feature
priority: low
created_at: 2026-11-04T03:21:32Z
updated_at: 2026-11-04T03:21:32Z
parent: jik-lsp-v137
blocked_by:
    - jik-lsp-xch7
---

做法：JSON 诊断的 detail/related 映射为 Diagnostic.related_information，重复错误去重。
验收：一条含相关位置的诊断在客户端可跳转到关联位置。
