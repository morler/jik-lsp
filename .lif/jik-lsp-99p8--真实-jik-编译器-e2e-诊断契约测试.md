---
# jik-lsp-99p8
title: 真实 jik 编译器 e2e 诊断契约测试
status: completed
type: task
priority: high
created_at: 2026-11-04T03:21:31Z
updated_at: 2026-11-04T07:12:57Z
parent: jik-lsp-rzv8
---

现状：tests/lifecycle.rs:87 与 tests/diagnostic_concurrency.rs 全部用 shell 假脚本 printf 手写报文；真实编译器只有 src/check.rs:217 一处单元测试。
做法：新增 tests/real_compiler.rs，在 ../jik/jik 存在时运行（否则 skip），用真实 jik check 触发：语法错误、类型错误、区域错误、warning 级别、多诊断叠加、含冒号/空格/非 ASCII 的路径、UTF-8 中文行内的列号。
验收：至少 5 个真实报文用例断言 line/byte_column/message/warning 与零长度 range 位置正确；无编译器时测试明确 skip 而非静默通过。

## Summary of Changes

新增 tests/real_compiler.rs：6 项契约测试直接驱动 ../jik/jik（可用 JIK_LSP_TEST_COMPILER 覆盖，缺失时跳过），固定真实报文语义——UTF-8 字节列需换算 UTF-16、解析错误逐条报告而语义错误遇首个未定义符号即停、含空格冒号路径定位、缺 main 落在文件末尾后一行、合法源码静默。顺带确认'干净检查不发布消息'是 LSP 缺省语义而非缺陷，测试以有界等待断言无发布。全套 31 项测试通过，CI run 41790516881 三平台全绿。
