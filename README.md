# EffectCraft · OTools 插件

把 [storytold/effectcraft](https://github.com/storytold/effectcraft)（开源、纯 Rust 重写的 After Effects 替代品）
移植为 OTools 插件：上游的 UI 原封不动编译到 WebAssembly，外壳把「文件从哪来、存到哪去」
换成 OTools 宿主的对话框与磁盘读写。

- 插件目录：`plugins/otools-effectcraft`（在 [ootools/OTools](https://github.com/ootools/OTools) 主仓里）
- 上游源码：**不入库**，见 `upstream.pin` + `scripts/fetch-upstream.mjs`

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
