---
# jik-lsp-99p8
title: 真实 jik 编译器 e2e 诊断契约测试
status: todo
type: task
priority: high
created_at: 2026-11-04T03:21:31Z
updated_at: 2026-11-04T03:21:31Z
parent: jik-lsp-rzv8
---

现状：tests/lifecycle.rs:87 与 tests/diagnostic_concurrency.rs 全部用 shell 假脚本 printf 手写报文；真实编译器只有 src/check.rs:217 一处单元测试。
做法：新增 tests/real_compiler.rs，在 ../jik/jik 存在时运行（否则 skip），用真实 jik check 触发：语法错误、类型错误、区域错误、warning 级别、多诊断叠加、含冒号/空格/非 ASCII 的路径、UTF-8 中文行内的列号。
验收：至少 5 个真实报文用例断言 line/byte_column/message/warning 与零长度 range 位置正确；无编译器时测试明确 skip 而非静默通过。
