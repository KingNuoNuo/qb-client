// release 构建时隐藏控制台窗口（debug 保留，方便看 panic 输出）
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod api;
mod app;
mod config;
mod fonts;
mod i18n;
mod model;
mod theme;
mod titlebar;
mod util;
mod widgets;
mod worker;

/// 窗口/任务栏图标。
///
/// 直接嵌一份 64×64 的**非预乘 RGBA** 原始字节（`assets/icon-64.rgba`，
/// 由 `assets/icon.svg` 导出），这样不必引入 PNG 解码依赖 —— 单 exe 离线可用
/// 是硬要求，能不加依赖就不加。
fn app_icon() -> egui::IconData {
    const RGBA: &[u8] = include_bytes!("../assets/icon-64.rgba");
    egui::IconData {
        rgba: RGBA.to_vec(),
        width: 64,
        height: 64,
    }
}

fn main() -> eframe::Result<()> {
    // 这里的两个尺寸只是「viewport 建好之前」的占位值 —— 真正的尺寸在
    // 首帧由 `QbApp::update` 按当前显示器可用区钳一遍（原因见那边的注释：
    // 小屏上最小尺寸可能比屏幕还大，硬套会让窗口跑到屏幕外面去）。
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([app::WIN_DEFAULT.x, app::WIN_DEFAULT.y])
            .with_min_inner_size([app::WIN_MIN.x, app::WIN_MIN.y])
            .with_title("qBittorrent 管理器")
            .with_icon(app_icon()),
        ..Default::default()
    };

    eframe::run_native(
        "qb-client",
        native_options,
        Box::new(|cc| Ok(Box::new(app::QbApp::new(cc)))),
    )
}
