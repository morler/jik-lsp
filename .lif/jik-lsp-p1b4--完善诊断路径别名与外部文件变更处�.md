---
# jik-lsp-p1b4
title: 完善诊断路径别名与外部文件变更处理
status: in-progress
type: task
priority: deferred
created_at: 2026-11-02T08:52:47Z
updated_at: 2026-11-04T02:03:36Z
---

原型已记录限制：符号链接等路径别名可能绕过打开文档匹配；检查前后快照无法检测期间改动后恢复、未打开文件变动或检查后到发布前的磁盘修改。设计可靠的文件身份和快照策略，并添加回归测试。

已实现规范路径和 Unix 设备号/inode 匹配诊断与打开文档，加入符号链接、硬链接端到端测试；已打开文件使用内容、身份及时间戳快照，在检查前后及发布前复查，覆盖修改后恢复。仍未完全解决：编译器未暴露完整依赖/读取版本，无法验证未打开依赖，最终检查与发布之间仍有竞态，时间戳精度也可能不足；需要编译器协作或隔离不可变输入快照。

进一步收窄发布前竞态窗口：获取状态锁后再复查已打开文件快照，避免等待状态锁扩大最终复查与发布之间的时间间隔。复查期间持锁可能延迟其他文档事件；外部文件仍可在复查后修改，未打开依赖仍无法验证。现有测试通过，尚无针对等待锁期间磁盘变更的确定性回归测试。

ocr 评审（会话 1bde634e）指出：把 read_open 复查移入状态锁无防护收益（并发变更已由 epoch/generation 拦截），却让文件 I/O 阻塞 did_open/change/save/watcher。已在 b3c865d 回滚，锁恢复为只包住 accept_result + publish。

收尾（本轮）：保存回声判定从「修改时间 <= 检查启动时间」改为内容比对——`State.started` 换成 `State.checking`，保存时在 `spawn_blocking` 中读取入口快照；watcher 事件只有在入口内容与快照一致（`same_identity` 加文本相同，忽略时间戳）时才视为回声，消除时间戳精度不足导致的误判。`CheckJob::check` 改为在解析编译器输出前先重读已打开文件，使「文件已变更」的告警不会被同一批过期输出引发的 jik 失败/非 UTF-8 日志覆盖。新增回归测试 `echo_of_own_save_does_not_start_second_check`（回声不触发第二次编译）与 `replaced_entry_with_same_content_still_rechecks`（内容变化且时间戳被回拨时仍重检）。未打开的依赖与最终复查到发布之间的竞态需要编译器协作或不可变输入快照，已在 README 中明确归因。


锁内 I/O 收尾（90b070e）：`start_check` 已拆成 `State::reserve_check`（锁内 bump generation、abort 旧任务、返回 `CheckTicket`）与 `Backend::begin_check`（回锁校验 stopped/epoch/generation 后才插入 `checking` 并 spawn `CheckJob`）。`did_save` 与 `did_change_watched_files` 均改为 reserve → publish → 释放锁 → `spawn_blocking` 读入口快照 → 回锁 begin_check，状态锁内不再有文件 I/O。reserve 刻意保留 `checking` 里的旧快照，避免窗口内 watcher 事件被当作新变更而重复检查。watcher 回声判定额外要求 `checking` 已含该入口，无在飞检查时跳过阻塞读；`CheckJob::snapshot` 改 `Arc<HashMap<Url, String>>`，批量事件只复制一次打开文档。新增 1000ms 静默期加固 `echo_of_own_save_does_not_start_second_check`（禁用回声抑制后该测试会失败）。fmt/clippy 全绿，25 项测试全绿。
