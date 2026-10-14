// release 构建时隐藏控制台窗口（debug 保留，方便看 panic 输出）
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod api;
mod app;
mod config;
mod fonts;
mod model;
mod theme;
mod titlebar;
mod util;
mod widgets;
mod worker;

fn main() -> eframe::Result<()> {
    // 这里的两个尺寸只是「viewport 建好之前」的占位值 —— 真正的尺寸在
    // 首帧由 `QbApp::update` 按当前显示器可用区钳一遍（原因见那边的注释：
    // 小屏上最小尺寸可能比屏幕还大，硬套会让窗口跑到屏幕外面去）。
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([app::WIN_DEFAULT.x, app::WIN_DEFAULT.y])
            .with_min_inner_size([app::WIN_MIN.x, app::WIN_MIN.y])
            .with_title("qBittorrent 管理器"),
        ..Default::default()
    };

    eframe::run_native(
        "qb-client",
        native_options,
        Box::new(|cc| Ok(Box::new(app::QbApp::new(cc)))),
    )
}
