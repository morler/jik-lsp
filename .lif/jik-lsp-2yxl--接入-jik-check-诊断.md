---
# jik-lsp-2yxl
title: 接入 jik check 诊断
status: in-progress
type: task
priority: normal
created_at: 2026-11-02T06:54:28Z
updated_at: 2026-11-02T07:10:33Z
---

实现编译器调用、诊断解析及保存后发布，并验证失败与并发安全。

已实现并测试诊断文本解析；尚需实现进程执行、配置、LSP 发布和过期诊断处理。

已增加受超时和总输出大小限制的 jik check 子进程封装及真实编译器契约测试；尚未接入服务端。取消测试曾偶发等待子进程启动超时，随后重复 30 次通过，原因未确定。
