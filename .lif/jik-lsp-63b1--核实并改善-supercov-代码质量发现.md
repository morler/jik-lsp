---
# jik-lsp-63b1
title: 核实并改善 Supercov 代码质量发现
status: completed
type: task
priority: normal
created_at: 2026-11-02T09:22:30Z
updated_at: 2026-11-02T09:27:13Z
---

核实方法长度、嵌套与复杂条件；仅做有实际可读性收益的重构，并运行回归验证。

核实 server.rs 的保存处理确实混合职责，已拆分后台检查与诊断转换，抽取文件版本条件；document.rs 的 CRLF 边界条件已命名并简化。check.rs 生产代码中未找到值得抽象的复杂布尔条件（工具未提供具体行号），保留原样。全量测试、clippy、fmt 通过；重新评分从 6.8 到 7.0，server.rs 仍标记长方法和复杂条件，需继续评估和拆分。

## Summary of Changes

将保存后的后台任务、编译器输出解码、诊断转换及源文件读取拆分，简化文件版本与 CRLF 判断；补充 stdout/stderr 非 UTF-8 端到端测试。20 项测试、clippy 和 fmt 通过；Supercov 评分 6.8→7.1。余下长方法、god_class 与 check.rs 复杂条件为模型判断，未为消除标签引入更多间接层。
