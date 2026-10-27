# EffectCraft · OTools 插件

把 [storytold/effectcraft](https://github.com/storytold/effectcraft)（开源、纯 Rust 重写的 After Effects 替代品）
移植为 OTools 插件：上游的 UI 原封不动编译到 WebAssembly，外壳把「文件从哪来、存到哪去」
换成 OTools 宿主的对话框与磁盘读写。

- 插件目录：`plugins/otools-effectcraft`（在 [ootools/OTools](https://github.com/ootools/OTools) 主仓里）
- 上游源码：**不入库**，见 `upstream.pin` + `scripts/fetch-upstream.mjs`

## MCP

插件通过宿主的 **headless MCP 后端**暴露上游 `effectcraft-automation` 的 36 个工具
（`execute_command` / `get_layer` / `add_effect` / `render_frame` / `ui_*` …），
不另写一套工具定义：

- `plugin.json` 声明 `mcp.backend = "headless"` + `native.enabled = true`，`expose` 列出全部工具；
- **`annotations` 一律不写**：上游 `ToolDef::descriptor()` 已给出权威的
  `readOnlyHint` / `destructiveHint`，manifest 写注解只会**覆盖**它（宿主 registry
  与设置页都是 manifest 优先）。写错一个字段就会让只读工具被当成破坏性、默认不勾选；
- `native/` 是薄适配层（cdylib），`__otools_mcp_tools__` 直接返回上游
  `tools::catalogue()` 的 `descriptor()`，`__otools_mcp_call__` 走
  `tools::run(&mut Backend, name, args)`；
- 会话常驻在 dylib 内的 `OnceLock<Mutex<Backend>>`，因此
  `execute_command comp.new → add_keyframe → render_frame` 这类有状态序列落在同一个 Session 上；
- **不做按工具名的过滤**：上游给每个工具标了 `bridge_only`，它们照常出现在工具表里，
  headless 调用时返回上游的 `NEED_BRIDGE` 可读错误，AI 能据此换工具。

`native/` 里有一条 `plugin_manifest_expose_matches_upstream_tools` 测试，
断言 `plugin.json` 的 `mcp.expose` 每个名字都真实存在于上游工具表——工具改名时 CI 会红。

### resources

上游除 tools 外还提供 **MCP resources**，插件原样转发（不复制内容）：

| URI | 上游 name | 内容 |
| --- | --- | --- |
| `effectcraft://document` | `document` | 工程 + 当前合成（等同 `doc_inspect`） |
| `effectcraft://commands` | `commands` | 命令目录（等同 `command_list`） |

宿主 ABI 的三个 method：`__otools_mcp_resources__`（→ `{resources:[…]}`）、
`__otools_mcp_resource_templates__`（→ `{resourceTemplates:[…]}`）、
`__otools_mcp_read_resource__`（`{uri}` → `{contents:[{uri,mimeType,text|blob}]}`）。
读失败以 `ok:false` 表达（resources 没有 `isError` 信封）。

⚠️ 这里**只复用上游的资源「映射」，不启动上游 MCP 服务**：`McpServer::new(backend)`
会消耗 `Backend`，而本插件要长期持有它执行工具。因此 `native/src/mcp.rs` 按上游的
同一个映射（uri → 用哪个工具取数）自己转发一次，URI 与 description 与上游逐字一致。

本地冒烟（按宿主的方式 dlopen 并真调工具 + 列举/读取资源）：

```bash
cargo build --release --manifest-path native/Cargo.toml
node scripts/smoke-mcp.mjs ..          # 参数是各 craft 仓库的共同父目录
```

数据流：宿主 FFI → `__otools_mcp_call__` → `Mutex<Backend>` → 上游 `tools::run` → MCP content。

## 上游版本

打包所用的上游提交。`upstream.pin` 是唯一真源，这张表用来回溯「哪个插件版本打的是哪份上游代码」，
每次发布都要在这里加一行。

| 插件版本 | 上游标签 | 上游提交 | 打包日期 |
| --- | --- | --- | --- |
| 0.1.4 | v0.11.1 | `2a01226894c6bc58ae4fadcc24a10198d693acaa` | 2026-10-21 |
| 0.1.3 | v0.11.1 | `2a01226894c6bc58ae4fadcc24a10198d693acaa` | 2026-10-21 |
| 0.1.2 | — | `05955c08b4fd025f1a09882df72e6dbff1a8148a` | — |

## 界面语言

上游自带 10 种语言的词典（菜单与面板正文），插件启动时把宿主的界面语言写进
`prefs.general.language`，运行中切换也会跟上。

上游词典**没覆盖**两处裸英文：面板标签页标题（`PanelKind::title()`）与工作区名
（`dock::WORKSPACES`），所以中文界面下标签页会显示 `Info` / `Audio` / `Standard`。
`shell/patches/effectcraft-i18n.patch` 补了这张小表（简繁中文，查不到回落英文），
只影响显示——`title()` 仍返回英文，`from_name` 与偏好里存的工作区名都按英文匹配。

## 构建

```bash
unset RUSTC_WRAPPER                 # 它会把 cargo 指向旧 rustc
node scripts/fetch-upstream.mjs     # 拉取 pinned 的上游到 upstream/
node scripts/build-wasm.mjs         # trunk build --release → dist/
```

需要 rustc >= 1.95 与 wasm32-unknown-unknown 目标（egui 0.36 的要求）。

详细约定见 [AGENTS.md](./AGENTS.md)。
