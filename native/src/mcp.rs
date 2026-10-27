//! 上游 `effectcraft-automation` 与宿主 MCP 契约之间的薄适配（见宿主 spec §5.4）。
//!
//! 上游已经给了完整的工具目录与执行入口，这里只做契约转换：
//!   - `tools::catalogue()` 的每个 `ToolDef::descriptor()` 已是 MCP Tool 对象
//!     （含 name / title / description / inputSchema / annotations）
//!   - `tools::run(&mut Backend, name, args)` 返回 `Reply::Json` / `Reply::Image`，
//!     转成 MCP 的 content 数组 + isError
//!   - `Backend::headless(Session)` 常驻，保证有状态工具序列落在同一个 Session 上
//!
//! **有意不做按工具名的过滤**：上游给每个 `ToolDef` 标了 `bridge_only`（需要连桌面 App
//! 的 UI 工具）。它们照常出现在工具表里，headless 调用时上游返回一句可读的
//! `NEED_BRIDGE` 错误，AI 能据此换工具。按名单过滤会在上游增删工具时静默漂移
//! （宿主 spec §10.2 的同一原则）。

use effectcraft_automation::base64;
use effectcraft_automation::tools::{self, Reply};
use effectcraft_automation::{Backend, Error};
use serde_json::{json, Value};
use std::sync::{Mutex, OnceLock};

/// 宿主 `__otools_mcp_tools__` 的响应体。
pub fn tools_payload() -> Value {
    let tools: Vec<Value> = tools::catalogue()
        .iter()
        .map(|tool| tool.descriptor())
        .collect();
    json!({ "tools": tools })
}

/// 上游声明的两个 MCP 资源（`effectcraft://document` / `effectcraft://commands`）。
///
/// 与上游 `server.rs` 的常量逐字对齐：上游把资源定义为「`doc_inspect` /
/// `command_list` 的同义入口」，所以这里复制的是**映射关系**而不是数据本身，
/// 读取时仍走 `tools::run`，拿到的是当前会话的真实内容。
const RESOURCES: &[(&str, &str, &str, &str, &str)] = &[
    (
        "effectcraft://document",
        "document",
        "Project",
        "The project overview and the active composition (same as doc_inspect).",
        "doc_inspect",
    ),
    (
        "effectcraft://commands",
        "commands",
        "Command catalog",
        "Every engine command with id, label, menu, shortcut and params (same as command_list).",
        "command_list",
    ),
];

/// 宿主 `__otools_mcp_resources__` 的响应体。
pub fn resources_payload() -> Value {
    let list: Vec<Value> = RESOURCES
        .iter()
        .map(|(uri, name, title, description, _)| {
            json!({
                "uri": uri,
                "name": name,
                "title": title,
                "description": description,
                "mimeType": "application/json"
            })
        })
        .collect();
    json!({ "resources": list })
}

/// 上游 `resources/templates/list` 返回空表（没有带参数的 URI 模板）。
pub fn resource_templates_payload() -> Value {
    json!({ "resourceTemplates": [] })
}

/// 读一个资源：按上游的映射换成对应的工具调用，结果包成 MCP `contents`。
pub fn read_resource_payload(uri: &str) -> Result<Value, String> {
    let tool = RESOURCES
        .iter()
        .find(|(u, ..)| *u == uri)
        .map(|(_, _, _, _, tool)| *tool)
        .ok_or_else(|| format!("resource not found: {uri}"))?;

    let text = {
        let mut guard = backend()
            .lock()
            .map_err(|_| "headless 会话状态已损坏，请重启 OTools".to_string())?;
        match tools::run(&mut guard, tool, &json!({})) {
            Ok(Reply::Json(v)) => serde_json::to_string(&v).unwrap_or_else(|_| v.to_string()),
            // 上游对 image 回复在这一分支返回 INTERNAL_ERROR，这里保持一致。
            Ok(Reply::Image { .. }) => {
                return Err("unexpected image for a JSON resource".to_string())
            }
            Err(error) => return Err(error.to_string()),
        }
    };
    Ok(json!({
        "contents": [{ "uri": uri, "mimeType": "application/json", "text": text }]
    }))
}

/// 进程内**常驻**的 headless 后端。
///
/// 工具序列有状态：`execute_command` 里的 `comp.new` → `add_keyframe` → `render_frame`
/// 依赖同一个 `Session`。每次调用新建 `Backend` 会让第二次调用面对空工程，整条工作流断裂。
/// 上游 MCP server 之所以没这问题，是因为它是长驻进程；这里同理，会话存在 dylib 内。
static BACKEND: OnceLock<Mutex<Backend>> = OnceLock::new();

fn backend() -> &'static Mutex<Backend> {
    // `effectcraft_host::session()` 与上游 CLI / 桌面 App 用的是同一套完整装配
    // （媒体解码、表达式引擎、脚本、导出器），而不是裸 `Session::default()`。
    BACKEND.get_or_init(|| Mutex::new(Backend::headless(effectcraft_host::session())))
}

/// 关掉会话（供宿主热更新前的停机握手调用）。
pub fn shutdown_backend() {
    if let Some(lock) = BACKEND.get() {
        if let Ok(mut guard) = lock.lock() {
            *guard = Backend::headless(effectcraft_host::session());
        }
    }
}

/// 宿主 `__otools_mcp_call__` 的响应体。
pub fn call_payload(name: &str, arguments: &Value) -> Value {
    let mut guard = match backend().lock() {
        Ok(guard) => guard,
        // 锁中毒（上一次调用 panic）时给出明确错误，而不是继续用半坏状态。
        Err(_) => {
            return json!({
                "content": [{ "type": "text", "text": "headless 会话状态已损坏，请重启 OTools" }],
                "isError": true
            })
        }
    };
    match tools::run(&mut guard, name, arguments) {
        Ok(reply) => reply_to_value(reply),
        Err(error) => error_result(&error),
    }
}

/// `Reply` → MCP 结果。与上游 `server.rs::call_tool` 的映射保持一致。
fn reply_to_value(reply: Reply) -> Value {
    match reply {
        Reply::Json(v) => {
            // 部分工具用 `failed` 计数报告批处理内的失败，上游据此置 isError。
            let failed = v
                .get("failed")
                .and_then(Value::as_u64)
                .is_some_and(|n| n > 0);
            json!({
                "content": [{ "type": "text", "text": serde_json::to_string_pretty(&v).unwrap_or_else(|_| v.to_string()) }],
                "isError": failed
            })
        }
        Reply::Image { png, info } => json!({
            "content": [
                { "type": "image", "data": base64::encode(&png), "mimeType": "image/png" },
                { "type": "text", "text": serde_json::to_string_pretty(&info).unwrap_or_else(|_| info.to_string()) }
            ],
            "isError": false
        }),
    }
}

/// 工具错误 → `isError: true` 的可读结果（让 AI 能自行纠正参数或换工具）。
fn error_result(error: &Error) -> Value {
    json!({
        "content": [{ "type": "text", "text": error.to_string() }],
        "isError": true
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // 这些用例共享进程内的常驻会话（这本身就是被测行为），必须串行执行。
    static TEST_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn tools_payload_maps_every_catalogue_entry() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let value = tools_payload();
        let list = value["tools"].as_array().expect("tools array");
        assert_eq!(
            list.len(),
            tools::catalogue().len(),
            "适配层不得截断上游工具表"
        );
        for tool in list {
            assert!(
                tool.get("name").and_then(Value::as_str).is_some(),
                "every tool needs a name: {tool}"
            );
            assert!(
                tool.get("inputSchema").is_some(),
                "every tool needs an inputSchema: {tool}"
            );
        }
    }

    #[test]
    fn tools_payload_passes_through_bridge_only_tools() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // 有意**不过滤** bridge_only 工具：headless 调用时上游给出 NEED_BRIDGE 可读错误。
        let value = tools_payload();
        let names: Vec<&str> = value["tools"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|t| t.get("name").and_then(Value::as_str))
            .collect();
        assert!(
            names.contains(&"screenshot"),
            "bridge_only 工具也应透传；got {names:?}"
        );
    }

    #[test]
    fn call_unknown_tool_returns_is_error() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let result = call_payload("definitely_not_a_tool", &json!({}));
        assert_eq!(result["isError"], true, "result: {result}");
    }

    #[test]
    fn call_read_only_tool_succeeds_headless() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let result = call_payload("list_commands", &json!({}));
        assert_eq!(result["isError"], false, "result: {result}");
        assert!(
            result.get("content").and_then(Value::as_array).is_some(),
            "must return MCP content array: {result}"
        );
    }

    #[test]
    fn session_state_persists_across_calls() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // 防止「每次调用新建 Backend」回归：新建的 comp 必须被下一次 get_project 看见。
        let created = call_payload(
            "execute_command",
            &json!({ "id": "comp.new", "params": { "name": "MCP test comp" } }),
        );
        assert_eq!(
            created["isError"], false,
            "comp.new should succeed: {created}"
        );

        let inspected = call_payload("get_project", &json!({}));
        assert_eq!(
            inspected["isError"], false,
            "get_project should succeed: {inspected}"
        );
        let text = inspected["content"][0]["text"].as_str().unwrap_or_default();
        assert!(
            text.contains("MCP test comp"),
            "the comp created in the previous call must still be there: {text}"
        );
    }

    fn tool_names() -> (std::sync::MutexGuard<'static, ()>, Vec<String>) {
        let guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let value = tools_payload();
        let names = value["tools"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|t| t.get("name").and_then(Value::as_str).map(str::to_string))
            .collect();
        (guard, names)
    }

    // ── resources ────────────────────────────────────────────────────────────
    //
    // upstream 的 `crates/automation/src/server.rs` 声明了两个资源
    // （`DOCUMENT_URI` / `COMMANDS_URI`）。`McpServer::new(backend)` 会消耗 `Backend`，
    // 而本适配层要长期持有它执行工具，所以这里按上游的**同一套映射**
    // （uri → 用哪个工具取数）自己转发一次；URI 与 description 必须与上游逐字一致。

    #[test]
    fn resources_payload_matches_upstream_uris() {
        let value = resources_payload();
        let resources = value["resources"].as_array().expect("resources array");
        let uris: Vec<&str> = resources
            .iter()
            .filter_map(|r| r.get("uri").and_then(Value::as_str))
            .collect();
        assert!(
            uris.contains(&"effectcraft://document"),
            "缺少工程资源，got {uris:?}"
        );
        assert!(
            uris.contains(&"effectcraft://commands"),
            "缺少命令目录资源，got {uris:?}"
        );
        for resource in resources {
            assert!(
                resource.get("name").and_then(Value::as_str).is_some(),
                "资源缺少 name: {resource}"
            );
            assert!(
                resource.get("mimeType").and_then(Value::as_str).is_some(),
                "资源缺少 mimeType: {resource}"
            );
        }
    }

    #[test]
    fn read_resource_returns_json_contents() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let value =
            read_resource_payload("effectcraft://commands").expect("read commands resource");
        let contents = value["contents"].as_array().expect("contents array");
        assert!(!contents.is_empty(), "contents 不应为空: {value}");
        let text = contents[0]["text"].as_str().expect("文本资源必须有 text");
        assert!(!text.trim().is_empty(), "资源文本不应为空");
    }

    #[test]
    fn read_unknown_resource_is_an_error() {
        let error = read_resource_payload("effectcraft://nope").expect_err("未知资源必须报错");
        assert!(error.contains("not found"), "got {error}");
    }

    #[test]
    fn resource_templates_is_empty() {
        // 上游没有带参数的 URI 模板，但方法必须答得上（空表而非 Unknown method）。
        let value = resource_templates_payload();
        let items = value["resourceTemplates"]
            .as_array()
            .expect("resourceTemplates array");
        assert!(items.is_empty(), "{value}");
    }

    /// 与工具面同一条守门逻辑：`plugin.json` 的 `mcp.resources` 必须与实际上报双向一致。
    ///
    /// 这里拿不到上游的 `ListResourcesResult`（会消耗 Backend），所以对照的是本适配层
    /// 上报的表——它自身由 `resources_payload_matches_upstream_uris` 与上游对齐。
    #[test]
    fn plugin_manifest_resources_match_reported() {
        let actual: Vec<String> = resources_payload()["resources"]
            .as_array()
            .expect("resources array")
            .iter()
            .filter_map(|r| r.get("uri").and_then(Value::as_str).map(str::to_string))
            .collect();

        let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../plugin.json");
        let raw = std::fs::read_to_string(&manifest)
            .unwrap_or_else(|e| panic!("读取 {} 失败: {e}", manifest.display()));
        let parsed: serde_json::Value =
            serde_json::from_str(&raw).expect("plugin.json 必须是合法 JSON");
        let declared: Vec<String> = parsed["mcp"]["resources"]
            .as_array()
            .expect("plugin.json 必须声明 mcp.resources")
            .iter()
            .map(|item| item.as_str().unwrap_or_default().to_string())
            .collect();

        let missing: Vec<&String> = actual.iter().filter(|u| !declared.contains(u)).collect();
        assert!(
            missing.is_empty(),
            "有资源但 plugin.json 的 mcp.resources 未列出: {missing:?}；实际 {actual:?}"
        );
        let extra: Vec<&String> = declared.iter().filter(|u| !actual.contains(u)).collect();
        assert!(
            extra.is_empty(),
            "plugin.json 的 mcp.resources 含不存在的 uri: {extra:?}；实际 {actual:?}"
        );
    }

    /// `plugin.json` 的 `mcp.expose` 是对外白名单初值。若它列了上游不存在的工具名，
    /// 宿主设置页会出现永远勾不上的条目；工具改名时这里必须同步。这条测试就是防漂移。
    #[test]
    fn plugin_manifest_expose_matches_upstream_tools() {
        let (_guard, names) = tool_names();
        let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../plugin.json");
        let raw = std::fs::read_to_string(&manifest)
            .unwrap_or_else(|e| panic!("读取 {} 失败: {e}", manifest.display()));
        let parsed: serde_json::Value =
            serde_json::from_str(&raw).expect("plugin.json 必须是合法 JSON");
        let expose = parsed["mcp"]["expose"]
            .as_array()
            .expect("plugin.json 必须声明 mcp.expose");
        let exposed: Vec<String> = expose
            .iter()
            .map(|item| item.as_str().unwrap_or_default().to_string())
            .collect();

        // 方向一：expose 不能列出上游没有的工具（否则设置页出现永远勾不上的死条目）。
        let unknown: Vec<&String> = exposed
            .iter()
            .filter(|name| !names.iter().any(|n| n == *name))
            .collect();
        assert!(
            unknown.is_empty(),
            "plugin.json 的 mcp.expose 含上游不存在的工具: {unknown:?}；上游实际有 {names:?}"
        );

        // 方向二（「不能少功能」的守门员）：上游每个工具都必须在 expose 里。
        // 上游新增工具时这条会红，提醒同步 expose，避免新能力静默不可见。
        let missing: Vec<&String> = names.iter().filter(|n| !exposed.contains(n)).collect();
        assert!(
            missing.is_empty(),
            "上游有工具但 plugin.json 的 mcp.expose 未列出（新能力会不可见）: {missing:?}"
        );
    }
}
