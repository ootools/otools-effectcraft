//! OTools 宿主桥（`window.effectcraftOtools`）。
//!
//! 上游的保存/渲染队列导出只有「浏览器下载」一条路（`files::download`）：
//! 把 dist 拖进普通浏览器没问题，但在 OTools 插件里会变成「每次保存都下一个新文件」。
//! 这里提供宿主的另存对话框与磁盘写入，`files::download` 优先走它，不可用才退回浏览器下载。
//!
//! JS 侧实现见 `shell/index.html`。

use base64::Engine as _;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;
use web_sys::js_sys;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = effectcraftOtools, js_name = "available")]
    fn host_available() -> bool;

    /// 宿主界面语言（BCP 47，如 `zh-CN`）：OTools 注入到插件 webview 的
    /// `window.__OTOOLS_LOCALE__.locale` / `document.documentElement.lang`。
    #[wasm_bindgen(js_namespace = effectcraftOtools, js_name = "language")]
    fn host_language_raw() -> String;

    /// `{name, dataBase64, forcePick}` → `{path}` 或 `null`（用户取消）。
    #[wasm_bindgen(js_namespace = effectcraftOtools, js_name = "saveFile")]
    fn host_save_file(request: JsValue) -> js_sys::Promise;
}

/// 宿主是否可用（不在 OTools 里时退回上游的浏览器下载）。
pub fn available() -> bool {
    host_available()
}

/// 宿主界面语言（BCP 47，如 `zh-CN`）：跟随 OTools 宿主语言切换。
pub fn host_language() -> String {
    host_language_raw()
}

/// 社区/外部链接（Discord、ArtCraft 官网、上游 GitHub）：在 OTools 插件里点这些
/// 会把用户拽到外部，让插件不纯粹。语言切换监听与链接屏蔽用到它。
pub fn is_community_url(url: &str) -> bool {
    url.contains("discord.gg") || url.contains("getartcraft.com") || url.contains("github.com/storytold")
}

/// 把字节交给宿主写盘。异步进行：调用方（同步的 `download`）立刻返回。
pub fn deliver(name: &str, data: &[u8]) {
    let request = save_request(name, &base64::engine::general_purpose::STANDARD.encode(data), true);
    let name = name.to_string();
    wasm_bindgen_futures::spawn_local(async move {
        match save_file(&request).await {
            Ok(Some(path)) => log::info!("已保存 {name} -> {path}"),
            Ok(None) => log::warn!("保存被取消：{name}"),
            Err(e) => log::error!("保存失败：{e}"),
        }
    });
}

fn save_request(name: &str, data: &str, force_pick: bool) -> JsValue {
    let request = js_sys::Object::new();
    let _ = js_sys::Reflect::set(&request, &JsValue::from_str("name"), &JsValue::from_str(name));
    let _ = js_sys::Reflect::set(&request, &JsValue::from_str("dataBase64"), &JsValue::from_str(data));
    let _ = js_sys::Reflect::set(&request, &JsValue::from_str("forcePick"), &JsValue::from_bool(force_pick));
    request.into()
}

async fn save_file(request: &JsValue) -> Result<Option<String>, String> {
    let value = JsFuture::from(host_save_file(request.clone())).await.map_err(js_error)?;
    if value.is_null() || value.is_undefined() {
        return Ok(None);
    }
    let path = js_sys::Reflect::get(&value, &JsValue::from_str("path")).map_err(|_| "宿主没有返回路径".to_string())?;
    Ok(path.as_string())
}

fn js_error(e: JsValue) -> String {
    e.as_string().unwrap_or_else(|| format!("{e:?}"))
}
