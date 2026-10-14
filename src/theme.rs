//! 设计系统 —— 令牌层。
//!
//! 视觉方向：**工业实用（Industrial / Utilitarian）**
//!
//! 三条铁律：
//!   1. 颜色只用语义令牌，组件里不写死色值（亮/暗主题靠覆盖令牌切换，组件代码不动）
//!   2. 一个主导表面色 + **一个**锐利强调色（琥珀），强调色只留给"当前选中 / 主操作"
//!   3. 状态色（下载青 / 上传绿 / 警告橙 / 错误红）是**语义层**，不参与强调色竞争；
//!      且任何状态都同时有文字或图形载体，不靠颜色单独表达含义
//!
//! 圆角一律很小（2–6pt）：工业风的关键是"发丝线与直角"，不是圆润。

// 令牌层天然会有"暂时没被用上"的档位（间距阶梯、字重变体），
// 保留它们是设计系统的一部分，不该报 dead_code。
#![allow(dead_code)]

use std::collections::BTreeMap;
use std::sync::OnceLock;

use egui::{Color32, FontFamily, FontId, Rounding, Stroke, TextStyle, Theme, ThemePreference};

// ---------------------------------------------------------------- 字体族

/// 展示字：Archivo Black（工业 grotesque）。只用于品牌字样，不作正文。
pub fn f_display() -> FontFamily {
    FontFamily::Name("qb-display".into())
}
/// 界面正文：Barlow Regular
pub fn f_ui() -> FontFamily {
    FontFamily::Proportional
}
/// 界面强调：Barlow SemiBold
pub fn f_sb() -> FontFamily {
    FontFamily::Name("qb-sb".into())
}
/// 数字：IBM Plex Mono SemiBold —— 表格里所有数值都走这里，保证等宽对齐
pub fn f_num() -> FontFamily {
    FontFamily::Name("qb-num".into())
}
/// 大号数字：IBM Plex Mono Bold（遥测带用）
pub fn f_num_bold() -> FontFamily {
    FontFamily::Name("qb-num-b".into())
}
/// 等宽正文：IBM Plex Mono
pub fn f_mono() -> FontFamily {
    FontFamily::Monospace
}

pub fn id(size: f32) -> FontId {
    FontId::new(size, f_ui())
}
pub fn id_sb(size: f32) -> FontId {
    FontId::new(size, f_sb())
}
pub fn id_num(size: f32) -> FontId {
    FontId::new(size, f_num())
}
pub fn id_num_b(size: f32) -> FontId {
    FontId::new(size, f_num_bold())
}
pub fn id_display(size: f32) -> FontId {
    FontId::new(size, f_display())
}
pub fn id_mono(size: f32) -> FontId {
    FontId::new(size, f_mono())
}

// ---------------------------------------------------------------- 尺寸令牌

/// 圆角阶梯。工业风刻意压到最小。
pub const R_SM: f32 = 2.0; // 进度条、徽标
pub const R_MD: f32 = 4.0; // 按钮、输入框、行
pub const R_LG: f32 = 6.0; // 窗口、弹层

/// 间距阶梯（4 / 8 / 12 / 16 / 24 / 32）
pub const S1: f32 = 4.0;
pub const S2: f32 = 8.0;
pub const S3: f32 = 12.0;
pub const S4: f32 = 16.0;
pub const S5: f32 = 24.0;
pub const S6: f32 = 32.0;

/// 表格行高。高密度但不能挤 —— 触摸目标另说（桌面鼠标场景 24–26 够用）
pub const ROW_H: f32 = 25.0;
pub const HEADER_H: f32 = 27.0;

// ---------------------------------------------------------------- 调色板

#[derive(Clone, Copy)]
pub struct Palette {
    /// 画布底色（渐变末端，屏幕下方）
    pub bg: Color32,
    /// 画布渐变色（屏幕上方）
    pub bg_top: Color32,
    /// 面板底色（顶栏 / 侧栏）
    pub surface: Color32,
    /// 卡片、输入框、按钮
    pub surface_hi: Color32,
    /// 凹槽（进度条底）
    pub well: Color32,
    /// 文本输入框内部。
    /// 亮色下**必须比画布更亮** —— 用灰底会让输入框看起来像"被禁用"，
    /// 而凹槽（进度条底）恰恰相反，要比画布更暗才像一道槽。两者在亮色下
    /// 分道扬镳，所以是两个令牌。
    pub field: Color32,
    /// 悬停底色
    pub hover: Color32,
    /// 发丝边框
    pub line: Color32,
    /// 强边框（悬停 / 聚焦）
    pub line_hi: Color32,
    /// 主文本（暖白，不用纯白）
    pub text: Color32,
    /// 次级文本
    pub muted: Color32,
    /// 三级文本 / 小标签
    pub faint: Color32,

    /// 唯一强调色（琥珀）—— 只用于选中态与主操作
    pub accent: Color32,
    /// 强调色的低透明底
    pub accent_soft: Color32,
    /// 压在强调色上的文字
    pub on_accent: Color32,

    /// 语义状态色
    pub dl: Color32,
    pub up: Color32,
    pub warn: Color32,
    pub err: Color32,

    /// 文本选中底色（输入框里拖选的那种）
    pub sel: Color32,
    /// 行选中底色。和 `sel` 分开：亮色主题下"选中的行"用白色，
    /// 语义是"白色 = 可交互/当前操作对象"；而输入框里的文字选中必须是有色的，
    /// 白底白字选中等于没有。
    pub row_sel: Color32,
    /// 左上角光晕（亮色主题下刻意置为全透明，见 `light()` 注释）
    pub glow: Color32,
}

/// 暗色（默认）。暖调近黑，不是纯黑也不是蓝黑。
fn dark() -> Palette {
    let accent = Color32::from_rgb(255, 159, 28); // 琥珀
    Palette {
        bg: Color32::from_rgb(12, 13, 15),
        bg_top: Color32::from_rgb(20, 22, 26),
        surface: Color32::from_rgba_unmultiplied(18, 20, 24, 236),
        surface_hi: Color32::from_rgb(27, 30, 35),
        well: Color32::from_rgb(9, 10, 12),
        field: Color32::from_rgb(9, 10, 12),
        hover: Color32::from_rgb(35, 39, 45),
        line: Color32::from_rgb(35, 39, 45),
        line_hi: Color32::from_rgb(56, 62, 71),
        text: Color32::from_rgb(233, 231, 226),
        muted: Color32::from_rgb(139, 146, 155),
        faint: Color32::from_rgb(96, 103, 112),

        accent,
        accent_soft: Color32::from_rgba_unmultiplied(255, 159, 28, 34),
        on_accent: Color32::from_rgb(26, 18, 4),

        dl: Color32::from_rgb(63, 193, 255),
        up: Color32::from_rgb(74, 222, 128),
        warn: Color32::from_rgb(245, 165, 36),
        err: Color32::from_rgb(255, 92, 92),

        sel: Color32::from_rgba_unmultiplied(255, 159, 28, 34),
        row_sel: Color32::from_rgba_unmultiplied(255, 159, 28, 30),
        glow: Color32::from_rgba_unmultiplied(255, 159, 28, 18),
    }
}

/// 亮色。**"白色 = 可交互"**是这一套的核心约定：
/// 画布是暖灰（数据落在灰底上），面板比画布略亮（像机箱面板），
/// 而按钮 / 输入框 / 选中的行一律是纯白 —— 于是"能操作的地方"一眼可辨。
///
/// 两条和暗色主题不同的决定：
///   * **没有光晕**。暗底上的琥珀径向渐变读作"屏幕的辉光"，很合理；
///     但同一招压在奶油色画布上就只是一摊脏黄渍，会把整片背景和行底色都染暖。
///     亮色下改用"表面层级"造纵深（画布 → 面板 → 白色），不需要光。
///   * **背景纯净无纹理**。深色发丝线在奶油底上的对比感远强于暗色下的白线，
///     同一套纹理放进来就直接变方格纸 —— 两套主题统一走"背景不印花"。
///
/// 强调色换成同色系更深一档（#B45309），保证压在白底上对比度 ≥ 4.5:1。
fn light() -> Palette {
    let accent = Color32::from_rgb(180, 83, 9);
    Palette {
        bg: Color32::from_rgb(237, 234, 227),
        bg_top: Color32::from_rgb(244, 241, 236),
        surface: Color32::from_rgba_unmultiplied(248, 246, 242, 246),
        surface_hi: Color32::from_rgb(255, 255, 255),
        well: Color32::from_rgb(223, 218, 209),
        field: Color32::from_rgb(255, 255, 255),
        hover: Color32::from_rgb(231, 227, 219),
        line: Color32::from_rgb(220, 214, 203),
        line_hi: Color32::from_rgb(194, 186, 171),
        text: Color32::from_rgb(27, 26, 23),
        muted: Color32::from_rgb(107, 102, 96),
        faint: Color32::from_rgb(151, 143, 133),

        accent,
        accent_soft: Color32::from_rgba_unmultiplied(180, 83, 9, 22),
        on_accent: Color32::from_rgb(255, 255, 255),

        dl: Color32::from_rgb(3, 105, 161),
        up: Color32::from_rgb(21, 128, 61),
        warn: Color32::from_rgb(161, 98, 7),
        err: Color32::from_rgb(185, 28, 28),

        sel: Color32::from_rgba_unmultiplied(180, 83, 9, 46),
        row_sel: Color32::from_rgb(255, 255, 255),
        glow: Color32::TRANSPARENT,
    }
}

static DARK: OnceLock<Palette> = OnceLock::new();
static LIGHT: OnceLock<Palette> = OnceLock::new();

/// 取当前主题的调色板。返回 `'static`，可以随便复制引用、塞进闭包，不会有借用冲突。
pub fn pal(dark_mode: bool) -> &'static Palette {
    if dark_mode {
        DARK.get_or_init(dark)
    } else {
        LIGHT.get_or_init(light)
    }
}

// ---------------------------------------------------------------- 样式安装

/// 把令牌灌进 egui 的全局 Style。
/// 主题切换时重新调用即可（组件代码完全不用改）。
pub fn install(ctx: &egui::Context, dark_mode: bool) {
    // egui 0.29 起，样式按主题分两份存放（dark_style / light_style），
    // 而 `Context::set_style` 只写"当前生效主题"那一份。eframe 默认跟随系统主题，
    // 系统是浅色时我们灌的深色样式就被晾在 dark 槽里读不到 —— 界面会静默退回 egui 默认样式。
    // 所以这里两份都写：主题切换由本客户端自己管（`theme_dirty`），不依赖 egui 跟随系统。
    let base = (*ctx.style()).clone();
    ctx.set_style_of(Theme::Dark, build_style(&base, true));
    ctx.set_style_of(Theme::Light, build_style(&base, false));
    ctx.set_theme(if dark_mode {
        ThemePreference::Dark
    } else {
        ThemePreference::Light
    });
}

/// 由令牌生成一套完整 `Style`。`base` 只用来补齐我们未显式覆盖的少量默认值。
fn build_style(base: &egui::Style, dark_mode: bool) -> egui::Style {
    let p = pal(dark_mode);
    let mut style = base.clone();

    // ---- 字阶 ----
    let mut ts: BTreeMap<TextStyle, FontId> = BTreeMap::new();
    ts.insert(TextStyle::Heading, id_display(17.0));
    ts.insert(TextStyle::Body, id(13.5));
    ts.insert(TextStyle::Button, id_sb(12.5));
    ts.insert(TextStyle::Small, id(11.0));
    ts.insert(TextStyle::Monospace, id_mono(12.5));
    // 命名样式：给 RichText::text_style() 用
    ts.insert(TextStyle::Name("sb".into()), id_sb(12.5));
    ts.insert(TextStyle::Name("num".into()), id_num(12.0));
    ts.insert(TextStyle::Name("num-b".into()), id_num_b(12.0));
    ts.insert(TextStyle::Name("tele".into()), id_num_b(17.0));
    ts.insert(TextStyle::Name("cap".into()), id_sb(9.0));
    style.text_styles = ts;

    // ---- 视觉 ----
    style.visuals = visuals(p, dark_mode);

    // ---- 间距 ----
    let sp = &mut style.spacing;
    sp.item_spacing = egui::vec2(S2, S1 + 2.0);
    sp.button_padding = egui::vec2(10.0, 4.0);
    sp.window_margin = egui::Margin::same(S4);
    sp.menu_margin = egui::Margin::same(S1 + 2.0);
    sp.menu_spacing = 4.0;
    sp.indent = 14.0;
    sp.interact_size = egui::vec2(26.0, 22.0);
    sp.slider_width = 150.0;
    sp.combo_width = 130.0;
    sp.icon_width = 16.0;
    sp.icon_width_inner = 9.0;
    sp.icon_spacing = 7.0;
    sp.tooltip_width = 420.0;

    // 动效：微交互 150–200ms（skill 基准）
    style.animation_time = 0.16;

    style
}

fn visuals(p: &Palette, dark_mode: bool) -> egui::Visuals {
    let mut v = if dark_mode {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };
    v.dark_mode = dark_mode;

    v.panel_fill = p.bg;
    v.window_fill = p.surface_hi;
    v.window_stroke = Stroke::new(1.0_f32, p.line_hi);
    v.window_rounding = Rounding::same(R_LG);
    v.menu_rounding = Rounding::same(R_MD);
    v.extreme_bg_color = p.field;
    v.faint_bg_color = p.hover;
    v.code_bg_color = p.well;
    v.hyperlink_color = p.accent;
    v.warn_fg_color = p.warn;
    v.error_fg_color = p.err;
    v.striped = false; // 不再用斑马纹，改用行间发丝线
    v.slider_trailing_fill = true;
    v.window_highlight_topmost = false;

    // 选中态 = 强调色的唯一主战场
    v.selection.bg_fill = p.sel;
    v.selection.stroke = Stroke::new(1.0_f32, p.accent);

    // 三级高度体系（Y 偏移 2 / 8 / 24）
    v.window_shadow = epaint_shadow(egui::vec2(0.0, 8.0), 24.0, 130);
    v.popup_shadow = epaint_shadow(egui::vec2(0.0, 6.0), 18.0, 110);

    // ---- 控件四态 ----
    let w = &mut v.widgets;

    w.noninteractive.bg_fill = p.surface_hi;
    w.noninteractive.weak_bg_fill = p.surface_hi;
    w.noninteractive.bg_stroke = Stroke::new(1.0_f32, p.line);
    w.noninteractive.fg_stroke = Stroke::new(1.0_f32, p.text); // ← 全局默认文字色
    w.noninteractive.rounding = Rounding::same(R_MD);
    w.noninteractive.expansion = 0.0;

    w.inactive.bg_fill = p.surface_hi;
    w.inactive.weak_bg_fill = p.surface_hi;
    w.inactive.bg_stroke = Stroke::new(1.0_f32, p.line);
    w.inactive.fg_stroke = Stroke::new(1.0_f32, p.text);
    w.inactive.rounding = Rounding::same(R_MD);
    w.inactive.expansion = 0.0;

    w.hovered.bg_fill = p.hover;
    w.hovered.weak_bg_fill = p.hover;
    w.hovered.bg_stroke = Stroke::new(1.0_f32, p.line_hi);
    w.hovered.fg_stroke = Stroke::new(1.0_f32, p.text);
    w.hovered.rounding = Rounding::same(R_MD);
    w.hovered.expansion = 0.0;

    w.active.bg_fill = p.accent_soft;
    w.active.weak_bg_fill = p.accent_soft;
    w.active.bg_stroke = Stroke::new(1.0_f32, p.accent);
    w.active.fg_stroke = Stroke::new(1.0_f32, p.text);
    w.active.rounding = Rounding::same(R_MD);
    w.active.expansion = 0.0;

    // 展开态（下拉 / 菜单按钮保持点开时的样子）
    w.open.bg_fill = p.hover;
    w.open.weak_bg_fill = p.hover;
    w.open.bg_stroke = Stroke::new(1.0_f32, p.line_hi);
    w.open.fg_stroke = Stroke::new(1.0_f32, p.text);
    w.open.rounding = Rounding::same(R_MD);
    w.open.expansion = 0.0;

    v
}

fn epaint_shadow(offset: egui::Vec2, blur: f32, alpha: u8) -> egui::epaint::Shadow {
    egui::epaint::Shadow {
        offset,
        blur,
        spread: 0.0,
        color: Color32::from_black_alpha(alpha),
    }
}
