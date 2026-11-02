# jik-lsp

基于 Rust 和 `tower-lsp` 的 Jik 语言服务器，目前支持完整文档同步和保存后诊断。

```sh
cargo build --release
```

启动 `target/release/jik-lsp`，使用标准输入/输出进行 LSP 通信。在编辑器的语言服务器配置中将 `.jik` 文件关联到该程序；初始化选项示例：

```json
{
  "jikPath": "/absolute/path/to/jik",
  "entryPath": "/absolute/path/to/project/main.jik"
}
```

`jikPath` 可省略，默认使用 PATH 中的 `jik`。`entryPath` 可省略，默认检查保存的文件；配置时必须使用绝对路径。`jik check` 要求有效的程序入口，直接检查独立模块可能报告缺少 `main`。编译器需要自身安装目录中的 `jiklib`，并遵循其 `JIK_PKG_PATH` 设置。

诊断只在保存后运行，编译器读取磁盘，不检查未保存的编辑内容；只要任一打开的本地文件与磁盘内容不同，检查会跳过。文件修改会清除已有诊断，直至再次保存并检查。诊断位置由编译器文本输出推算，为零长度，且不是完整源范围。编译器启动失败、超时、输出过大或无法定位的消息会写入客户端日志。路径别名和检查期间外部修改文件仍可能影响结果；请以编译器输出为准。

目前尚不提供补全、跳转或语义索引。使用 `cargo test` 运行测试；真实编译器测试在 `../jik/jik` 存在时运行，否则跳过。
