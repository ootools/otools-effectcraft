// EffectCraft 的 OTools 插件外壳：入口。
//
// 上游 `apps/effectcraft-web` 是 lib crate，本文件把它的模块整层挂进来当 bin 跑：
// 这些模块本来就是「宿主适配层」（files / persist / storage / browse / audio / worker…），
// 相当于把上游的浏览器外壳当作我们自己的外壳用。真正的业务（engine / ui-egui）走 path 依赖。
//
// 与上游 `web/index.html` 的区别只有两点（都在 index.html 里）：
//   - 用 trunk 的 data-trunk 注入加载器，不再手写 `init()` + `ec.start()`
//   - 强制 `?noworkers`：worker 里按文件名硬编码拼 wasm 路径，trunk 的 filehash 会对不上
//
// 没移植的部分（宿主文件对话框、宿主写盘）见 AGENTS.md 的「状态」一节。

#[cfg(target_arch = "wasm32")]
mod api;
#[cfg(target_arch = "wasm32")]
mod app;
#[cfg(target_arch = "wasm32")]
mod audio;
#[cfg(target_arch = "wasm32")]
mod browse;
#[cfg(target_arch = "wasm32")]
mod diskcache;
#[cfg(target_arch = "wasm32")]
mod files;
#[cfg(target_arch = "wasm32")]
mod frames;
#[cfg(target_arch = "wasm32")]
mod otools;
#[cfg(target_arch = "wasm32")]
mod persist;
#[cfg(target_arch = "wasm32")]
mod storage;
#[cfg(target_arch = "wasm32")]
mod worker;

pub mod store;

// 上游 `lib.rs` 里的 `pub use app::*;`：其余模块是用 `crate::post` / `crate::repaint` /
// `crate::page_url` 这样引用它们的，少了这一行整个外壳都编译不过。
#[cfg(target_arch = "wasm32")]
pub use app::*;

#[cfg(target_arch = "wasm32")]
fn main() {
    eframe::WebLogger::init(log::LevelFilter::Info).ok();
    wasm_bindgen_futures::spawn_local(async move {
        if let Err(e) = app::start("effectcraft_canvas".to_string()).await {
            log::error!("EffectCraft 启动失败：{e:?}");
            if let Some(document) = web_sys::window().and_then(|w| w.document())
                && let Some(el) = document.get_element_by_id("loading")
            {
                el.set_inner_html(&format!("<div class=\"err\">EffectCraft 启动失败：{e:?}</div>"));
            }
        }
    });
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    eprintln!("otools-effectcraft 只在浏览器里运行：在 shell/ 目录用 `trunk build --release` 构建");
}
