//! 字体装载。
//!
//! 三套 Latin 字体**内嵌进 exe**（均为 SIL OFL 1.1，许可全文见
//! `assets/fonts/OFL.txt`），保证换台机器也不会退化成默认字体：
//!
//!   Archivo Black  —— 展示字（品牌字样；Archivo 是工业 grotesque 的典型）
//!   Barlow         —— 界面正文（DIN 系骨架，R/M/SB 三档真字重）
//!   IBM Plex Mono  —— 等宽数字（表格里所有数值靠它对齐）
//!
//! **中文不内嵌** —— 一套 CJK 字体动辄 10–20MB，塞进 exe 不划算。
//! 改为运行时从系统字体目录挑一份中文字体，追加到每个字体族末尾做
//! fallback：egui 逐字体查找字形，Latin 命中的走 Barlow/Plex，没命中
//! （也就是中文）自动落到中文字体，两边都不会出方块。
//!
//! 三个平台的候选写在**同一张表**里 —— `std::fs::read` 打不开的路径
//! 直接跳过，所以跨平台不需要 cfg，也省得维护三份。
//!
//! 另外刻意**不用** `● ○ ◌ ↑ ↓ ＋ ⚠` 这类符号字形来画界面元素 ——
//! 它们在不同字体的覆盖情况不可控。所有圆点、箭头、加号、警告三角一律
//! 由 `widgets` 里的 painter 现画，只依赖几何图形，不依赖字形。

use egui::{FontData, FontDefinitions, FontFamily};

const ARCHIVO: &[u8] = include_bytes!("../assets/fonts/ArchivoBlack-Regular.ttf");
const BARLOW_R: &[u8] = include_bytes!("../assets/fonts/Barlow-Regular.ttf");
const BARLOW_M: &[u8] = include_bytes!("../assets/fonts/Barlow-Medium.ttf");
const BARLOW_SB: &[u8] = include_bytes!("../assets/fonts/Barlow-SemiBold.ttf");
const PLEX_R: &[u8] = include_bytes!("../assets/fonts/IBMPlexMono-Regular.ttf");
const PLEX_SB: &[u8] = include_bytes!("../assets/fonts/IBMPlexMono-SemiBold.ttf");
const PLEX_B: &[u8] = include_bytes!("../assets/fonts/IBMPlexMono-Bold.ttf");

const K_ARCHIVO: &str = "qb-archivo";
const K_BARLOW: &str = "qb-barlow";
const K_BARLOW_M: &str = "qb-barlow-m";
const K_BARLOW_SB: &str = "qb-barlow-sb";
const K_PLEX: &str = "qb-plex";
const K_PLEX_SB: &str = "qb-plex-sb";
const K_PLEX_B: &str = "qb-plex-b";
const K_CJK: &str = "qb-cjk";

/// 系统中文字体候选，按优先级。三个平台写在同一张表里，读不到的自动跳过。
const CJK_CANDIDATES: &[&str] = &[
    // ---- Windows ----
    "C:\\Windows\\Fonts\\msyh.ttc",     // 微软雅黑（Windows 各语言版都带）
    "C:\\Windows\\Fonts\\msyh.ttf",
    "C:\\Windows\\Fonts\\msyhl.ttc",    // 雅黑 Light
    "C:\\Windows\\Fonts\\Deng.ttf",     // 等线
    "C:\\Windows\\Fonts\\simhei.ttf",   // 黑体
    "C:\\Windows\\Fonts\\simsun.ttc",   // 宋体
    // ---- macOS：苹方是系统标配，日/宋体做兜底 ----
    "/System/Library/Fonts/PingFang.ttc",
    "/System/Library/Fonts/Hiragino Sans GB.ttc",
    "/System/Library/Fonts/STHeiti Medium.ttc",
    "/System/Library/Fonts/Supplemental/Songti.ttc",
    // ---- Linux：Noto CJK 覆盖绝大多数发行版，文泉驿兜底 ----
    "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
    "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc",
    "/usr/share/fonts/truetype/noto/NotoSansCJK-Regular.ttc",
    "/usr/share/fonts/truetype/wqy/wqy-microhei.ttc",
    "/usr/share/fonts/wenquanyi/wqy-zenhei/wqy-zenhei.ttc",
];

pub fn install(ctx: &egui::Context) {
    let mut f = FontDefinitions::default();

    f.font_data
        .insert(K_ARCHIVO.into(), FontData::from_static(ARCHIVO));
    f.font_data
        .insert(K_BARLOW.into(), FontData::from_static(BARLOW_R));
    f.font_data
        .insert(K_BARLOW_M.into(), FontData::from_static(BARLOW_M));
    f.font_data
        .insert(K_BARLOW_SB.into(), FontData::from_static(BARLOW_SB));
    f.font_data
        .insert(K_PLEX.into(), FontData::from_static(PLEX_R));
    f.font_data
        .insert(K_PLEX_SB.into(), FontData::from_static(PLEX_SB));
    f.font_data
        .insert(K_PLEX_B.into(), FontData::from_static(PLEX_B));

    // 中文字体（可选；找不到就纯 Latin，界面里的中文会变方块）
    let mut has_cjk = false;
    for path in CJK_CANDIDATES {
        if let Ok(bytes) = std::fs::read(path) {
            f.font_data
                .insert(K_CJK.into(), FontData::from_owned(bytes));
            has_cjk = true;
            break;
        }
    }
    if !has_cjk {
        eprintln!("[qb-client] 未找到系统中文字体，界面中文将显示为方块");
    }

    // egui 自带字体保留在末尾兜底（emoji / 图标字形）。
    // 用 `contains_key` 过滤而不是硬编码 —— 万一上游改了字体名，这里不会 panic。
    let tail: Vec<String> = ["NotoEmoji-Regular", "emoji-icon-font", "Ubuntu-Light", "Hack"]
        .iter()
        .filter(|n| f.font_data.contains_key(**n))
        .map(|n| (*n).to_string())
        .collect();

    let build = |base: &[&str]| -> Vec<String> {
        let mut v: Vec<String> = base.iter().map(|s| (*s).to_string()).collect();
        if has_cjk {
            v.push(K_CJK.to_string());
        }
        v.extend(tail.iter().cloned());
        v
    };

    f.families.insert(
        FontFamily::Proportional,
        build(&[K_BARLOW, K_BARLOW_M, K_BARLOW_SB]),
    );
    f.families
        .insert(FontFamily::Monospace, build(&[K_PLEX, K_PLEX_SB]));
    f.families.insert(
        FontFamily::Name("qb-sb".into()),
        build(&[K_BARLOW_SB, K_BARLOW_M, K_BARLOW]),
    );
    f.families.insert(
        FontFamily::Name("qb-display".into()),
        build(&[K_ARCHIVO, K_BARLOW_SB]),
    );
    f.families.insert(
        FontFamily::Name("qb-num".into()),
        build(&[K_PLEX_SB, K_PLEX]),
    );
    f.families.insert(
        FontFamily::Name("qb-num-b".into()),
        build(&[K_PLEX_B, K_PLEX_SB]),
    );

    ctx.set_fonts(f);
}
