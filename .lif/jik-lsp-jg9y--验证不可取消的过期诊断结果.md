---
# jik-lsp-jg9y
title: 验证不可取消的过期诊断结果
status: completed
type: task
priority: deferred
created_at: 2026-11-02T08:52:47Z
updated_at: 2026-11-02T09:20:10Z
---

原型端到端测试覆盖较新保存取消进行中的子进程，但尚未注入不可取消的旧结果来强制验证代次门控的逆序完成场景；增加可控的检查执行器及确定性测试。

## Summary of Changes

将结果接收与代次、epoch、shutdown 校验集中到持锁的 State::accept_result；使用 oneshot 强制较新结果先接收、不可取消的旧结果后到，验证旧结果不得替换，并覆盖失效和关闭校验。全量测试及 clippy 通过。
