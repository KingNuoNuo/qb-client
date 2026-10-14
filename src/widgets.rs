//! 自绘控件与氛围层。
//!
//! egui 内置控件是"通用"取向的（默认圆角、默认间距、均匀配色），这个模块把它们
//! 换成工业实用风格的版本：发丝线、微圆角、等宽数字、颜色只服务状态语义。
//!
//! 分工原则（也是可访问性策略）：
//!   * **交互控件**（按钮、输入框、下拉、勾选、可选项）一律沿用 egui 内置实现，
//!     只靠 `theme::install` 改视觉 —— 这样键盘聚焦、Tab 遍历、焦点圈全部保住；
//!   * **纯展示元件**（遥测带、进度条、状态点、区块标题、背景氛围）在本模块自绘，
//!     因为它们不需要交互语义，但需要 egui 给不了的版式控制。
//!
//! 所有图形元素都由几何形状现画，不依赖 `● ↑ ↓ ＋ ⚠` 这类符号字形 ——
//! 那些字形在不同字体里的覆盖情况不可控，是中文界面上最容易出现的"方块"来源。

// 元件库会提供一些当前版式还没用上的绘制原语（例如徽标），保留是刻意的。
#![allow(dead_code)]

use egui::{
    Align2, Color32, Pos2, Rect, Response, Rounding, Sense, Shape, Stroke, Ui, pos2, vec2,
};

use crate::theme::{self as t, Palette, R_SM, S2};
use crate::i18n::tr;

/// `Painter::add` 返回的槽位句柄类型别名。只在这一处引用，方便将来 API 变动。
pub type Slot = egui::layers::ShapeIdx;

// ------------------------------------------------------------------ 颜色工具

/// 线性插值：k=0 → a，k=1 → b
pub fn lerp(a: Color32, b: Color32, k: f32) -> Color32 {
    let k = k.clamp(0.0, 1.0);
    let f = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * k).round() as u8;
    Color32::from_rgba_unmultiplied(
        f(a.r(), b.r()),
        f(a.g(), b.g()),
        f(a.b(), b.b()),
        f(a.a(), b.a()),
    )
}

/// 把不透明色转成带 alpha 的版本
pub fn alpha(c: Color32, a: u8) -> Color32 {
    Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), a)
}

// ------------------------------------------------------------------ 氛围层

/// 画整个窗口的背景：垂直渐变底 + 左上角琥珀光晕（暗色）/ 纯色层级（亮色）。
///
/// 走 `Order::Background` 图层，所有面板都盖在它上面；把中央面板的
/// `Frame` 设成透明，氛围就能从内容底下透出来。
pub fn backdrop(ctx: &egui::Context, p: &Palette) {
    let screen = ctx.screen_rect();
    if screen.width() < 4.0 || screen.height() < 4.0 {
        return;
    }
    // 关键：必须用 `LayerId::background()`。
    // egui 的各个 panel 本身画在 `{Order::Background, Id::new("background")}` 这一层，
    // 只有同层才能靠"先添加的在下"拿到正确叠放次序（本函数在 `update` 最开头调用）。
    // 若自造一个同属 Order::Background 但 id 不同的层，两者会按 Id 的哈希值排序；
    // 一旦排到 panel 之后，背景就画在 UI 上面，把整个界面盖住。
    let layer = egui::LayerId::background();
    let painter = ctx.layer_painter(layer);

    // ① 垂直渐变底（上浅下深，制造纵深）
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(screen.left_top(), p.bg_top);
    mesh.colored_vertex(screen.right_top(), p.bg_top);
    mesh.colored_vertex(screen.right_bottom(), p.bg);
    mesh.colored_vertex(screen.left_bottom(), p.bg);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    painter.add(Shape::mesh(mesh));

    // ② 左上角琥珀光晕（三角扇模拟径向渐变）
    //
    // 暗色主题下这是"屏幕的辉光"，成立；亮色主题下 `glow` 是全透明的 ——
    // 琥珀压在奶油色画布上只会变成一摊脏黄渍，把画布和行底色一起染暖，
    // 还会让画布比面板更亮、把"面板浮在画布上"的层级关系整个抹平。
    // 亮色主题改由表面层级（画布 → 面板 → 白）造纵深。
    if p.glow.a() > 0 {
        let r = screen.width().max(screen.height()) * 0.62;
        radial(
            &painter,
            pos2(screen.left() + screen.width() * 0.08, screen.top() - r * 0.28),
            r,
            p.glow,
        );
    }
}

// 这里原来还有一层 48pt 发丝网格，两套主题都调过强度之后**彻底退役了**。
// 结论写下来免得日后又绕回去：
//   * 它是纯装饰，唯一的作用是"让大片空背景不那么空"—— 而列表本身有行高、
//     发丝线和表头在撑着结构，根本不需要背景再补一层纹理；
//   * 数据密集的界面里，**背景纹理 100% 会和内容抢注意力**。亮度调到看不见时
//     它等于不存在，调到看得见时它就是方格纸 —— 中间没有"刚好的档位"；
//   * 亮暗两套主题对纹理的敏感度还不一样，得各调一次，维护成本纯属浪费。
// 需要质感就用间距、发丝线和字重去表达，别用背景图案。

/// 三角扇伪径向渐变：中心色 → 边缘全透明
fn radial(painter: &egui::Painter, center: Pos2, radius: f32, inner: Color32) {
    let outer = Color32::TRANSPARENT;
    let seg: u32 = 48;
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(center, inner);
    for i in 0..=seg {
        let a = i as f32 / seg as f32 * std::f32::consts::TAU;
        mesh.colored_vertex(
            center + vec2(a.cos() * radius, a.sin() * radius),
            outer,
        );
    }
    for i in 0..seg {
        mesh.add_triangle(0, 1 + i, 2 + i);
    }
    painter.add(Shape::mesh(mesh));
}

// ------------------------------------------------------------------ 基础元件

/// 水平发丝线
pub fn hairline(ui: &mut Ui, p: &Palette) {
    let w = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(w.max(1.0), 1.0), Sense::hover());
    ui.painter().rect_filled(rect, 0.0, p.line);
}

/// 竖直分隔条（遥测带之间用）
pub fn vsep(ui: &mut Ui, p: &Palette, h: f32) {
    let (rect, _) = ui.allocate_exact_size(vec2(1.0, h), Sense::hover());
    ui.painter().rect_filled(rect, 0.0, p.line);
}

/// 区块标题：小号、宽字距、后面接一条延伸到右边缘的发丝线
pub fn section(ui: &mut Ui, p: &Palette, text: &str) {
    ui.add_space(S2);
    let w = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(w.max(1.0), 20.0), Sense::hover());
    let painter = ui.painter();
    let cy = rect.center().y;
    let tr = painter.text(
        pos2(rect.left(), cy),
        Align2::LEFT_CENTER,
        spaced(text),
        t::id_sb(9.5),
        p.faint,
    );
    if tr.right() + 10.0 < rect.right() {
        painter.line_segment(
            [pos2(tr.right() + 10.0, cy), pos2(rect.right(), cy)],
            Stroke::new(1.0_f32, p.line),
        );
    }
}

/// 给中文短标签加字距：`"状态"` → `"状 态"`。
/// egui 没有 letter-spacing，用空格模拟小字号标题的追踪感。
///
/// 仅当两个相邻字符都是 CJK 时才插空格——英文（及拉丁字母）之间不加，
/// 否则英文标签会被拆成 `S t a t u s`。
pub fn spaced(s: &str) -> String {
    let mut o = String::new();
    let mut prev_cjk = false;
    for c in s.chars() {
        let is_cjk = (c as u32) > 0x2E80;
        if prev_cjk && is_cjk {
            o.push(' ');
        }
        o.push(c);
        prev_cjk = is_cjk;
    }
    o
}

// ------------------------------------------------------------------ 遥测带

/// 顶栏的一个读数块
pub struct Tele<'a> {
    /// `Some(true)` 上传箭头 / `Some(false)` 下载箭头 / `None` 不画
    pub arrow: Option<bool>,
    pub label: &'a str,
    pub value: String,
    pub unit: &'a str,
    pub color: Color32,
    /// 底部读数条的比例（0..1）
    pub frac: f32,
}

/// 大号等宽数字 + 小标签 + 底部细条。
/// 顶栏那一条"机架面板"就是靠它撑起来的。
pub fn telemetry(ui: &mut Ui, p: &Palette, it: &Tele<'_>, width: f32) {
    let (rect, _) = ui.allocate_exact_size(vec2(width, 42.0), Sense::hover());
    let painter = ui.painter();

    let mut lx = rect.left();
    if let Some(up) = it.arrow {
        arrow(painter, pos2(lx + 4.5, rect.top() + 10.5), 10.0, up, it.color);
        lx += 13.0;
    }
    painter.text(
        pos2(lx, rect.top() + 6.0),
        Align2::LEFT_TOP,
        spaced(it.label),
        t::id_sb(9.0),
        p.faint,
    );

    let vr = painter.text(
        pos2(rect.left(), rect.top() + 18.0),
        Align2::LEFT_TOP,
        it.value.as_str(),
        t::id_num_b(17.5),
        it.color,
    );
    if !it.unit.is_empty() {
        painter.text(
            pos2(vr.right() + 5.0, vr.bottom() - 2.0),
            Align2::LEFT_BOTTOM,
            it.unit,
            t::id(9.5),
            p.faint,
        );
    }

    // 底部读数条
    let by = rect.bottom() - 1.0;
    let bar = Rect::from_min_max(pos2(rect.left(), by), pos2(rect.right(), by + 1.5));
    painter.rect_filled(bar, 0.0, p.line);
    let k = it.frac.clamp(0.0, 1.0);
    if k > 0.002 {
        let mut f = bar;
        f.max.x = bar.left() + bar.width() * k;
        painter.rect_filled(f, 0.0, it.color);
    }
}

// ------------------------------------------------------------------ 表格单元

/// 左对齐文本单元（按估算字宽截断并加省略号）。
///
/// `_p` 目前用不到，但保留这个参数：所有 `*_cell` 单元函数的签名保持
/// `(ui, palette, …)` 一致，列顺序调整时不必回头核对哪个要传调色板。
pub fn text_cell(ui: &mut Ui, _p: &Palette, text: &str, color: Color32, size: f32) {
    let full = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(full.max(1.0), 17.0), Sense::hover());
    if rect.width() < 4.0 {
        return;
    }
    let shown = fit(text, rect.width(), size);
    ui.painter().text(
        pos2(rect.left(), rect.center().y),
        Align2::LEFT_CENTER,
        shown,
        t::id(size),
        color,
    );
}

/// 右对齐等宽数字单元 —— 数值列全部右对齐，小数点才能对齐成一条线。
/// `_p` 的理由同 [`text_cell`]。
pub fn num_cell(ui: &mut Ui, _p: &Palette, text: &str, color: Color32) {
    let full = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(full.max(1.0), 17.0), Sense::hover());
    if rect.width() < 4.0 {
        return;
    }
    ui.painter().text(
        pos2(rect.right(), rect.center().y),
        Align2::RIGHT_CENTER,
        text,
        t::id_num(12.0),
        color,
    );
}

/// 标签列：每个标签画一颗小胶囊，装不下才截断。
///
/// 原来把标签 `join(" ")` 当一整串文本画，两个标签会粘成
/// 「MOVIEPILOT 已整理」—— 既分不清边界，截断位置也毫无规律。
/// 胶囊给每个标签明确的边界，顺带把"标签"和"分类"在视觉上区分开。
pub fn tag_pills(ui: &mut Ui, p: &Palette, tags: &[&str]) {
    let full = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(full.max(1.0), 17.0), Sense::hover());
    if rect.width() < 24.0 || tags.is_empty() {
        return;
    }
    let painter = ui.painter();
    let font = t::id(10.5);
    let (pad, gap, ch) = (6.0_f32, 4.0_f32, 15.0_f32);
    let mut x = rect.left();

    for (i, tag) in tags.iter().enumerate() {
        let remain = rect.right() - x;
        if remain < 20.0 {
            break;
        }
        let g = painter.layout_no_wrap((*tag).to_owned(), font.clone(), Color32::PLACEHOLDER);
        let want = g.size().x + pad * 2.0;
        // 放得下就整颗画；放不下时**只允许最后一个标签被截断**，前面的直接不画 ——
        // 半颗被切掉的胶囊比"没有这个标签"更难看。
        let (text, w) = if want <= remain {
            ((*tag).to_owned(), want)
        } else if i + 1 == tags.len() {
            let shown = fit(tag, (remain - pad * 2.0).max(6.0), 10.5);
            let g2 = painter.layout_no_wrap(shown.clone(), font.clone(), Color32::PLACEHOLDER);
            (shown, (g2.size().x + pad * 2.0).min(remain))
        } else {
            break;
        };
        let pill = Rect::from_min_size(pos2(x, rect.center().y - ch * 0.5), vec2(w, ch));
        painter.rect_filled(pill, t::R_MD, p.hover);
        painter.text(
            pill.center(),
            Align2::CENTER_CENTER,
            text,
            font.clone(),
            p.muted,
        );
        x = pill.right() + gap;
    }
}

/// 勾选标记（**纯展示**，点击由调用方的整行响应统一处理）。
///
/// 为什么不用 `ui.checkbox()`：表格整行是 `Sense::click()`，两者在同一位置
/// 会抢同一次点击 —— egui 只会把 click 判给其中一个，结果就是
/// 「点勾选框变成整行单选」，多选直接失效。这里只负责画，
/// 点击集中到行上，**单一交互源**，不会互相打架。
pub fn check_mark(ui: &mut Ui, p: &Palette, checked: bool, size: f32) -> Response {
    let full = ui.available_width();
    let (box_rect, resp) = ui.allocate_exact_size(vec2(full.max(1.0), 17.0), Sense::hover());
    let side = size.min(box_rect.height()).min(full.max(1.0));
    let rect = Rect::from_center_size(box_rect.center(), vec2(side, side));
    let painter = ui.painter();

    if checked {
        painter.rect_filled(rect, R_SM, p.accent);
        // 对勾用折线现画，不依赖 "✓" 字形
        painter.add(Shape::line(
            vec![
                pos2(rect.left() + side * 0.22, rect.center().y - side * 0.02),
                pos2(rect.left() + side * 0.42, rect.bottom() - side * 0.26),
                pos2(rect.right() - side * 0.22, rect.top() + side * 0.26),
            ],
            Stroke::new(1.8_f32, p.on_accent),
        ));
    } else {
        // 未选中 = **空心细描边**。
        // 原来是灰底填充，读起来像"被禁用"或"半选"，而且灰底压在画布上会糊成一团。
        // 空心方框才是"可勾选"的通用语义，也把视觉重量让给了内容。
        painter.rect_filled(rect, R_SM, Color32::TRANSPARENT);
        painter.rect_stroke(rect, R_SM, Stroke::new(1.5_f32, p.line_hi));
    }

    resp
}

/// 状态单元：色点 + 状态文字 + 右侧 faint 的 peer 计数
pub fn status_cell(ui: &mut Ui, p: &Palette, text: &str, color: Color32, peers: Option<String>) {
    let full = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(full.max(1.0), 17.0), Sense::hover());
    if rect.width() < 12.0 {
        return;
    }
    let painter = ui.painter();
    let cy = rect.center().y;
    painter.circle_filled(pos2(rect.left() + 4.0, cy), 3.0, color);
    let _ = painter.text(
        pos2(rect.left() + 12.0, cy),
        Align2::LEFT_CENTER,
        text,
        t::id(12.0),
        color,
    );
    if let Some(x) = peers {
        painter.text(
            pos2(rect.right(), cy),
            Align2::RIGHT_CENTER,
            x,
            t::id_num(10.5),
            p.faint,
        );
    }
}

/// 自绘进度条：凹槽 + 读数头 + 右对齐百分比。
///
/// 数值走 `animate_value_with_time` 平滑过渡 —— 进度跳动会"闪"，
/// 平滑之后能直观看出速度差异。
pub fn progress(ui: &mut Ui, p: &Palette, anim: egui::Id, frac: f32, color: Color32) {
    let full = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(full.max(1.0), 17.0), Sense::hover());
    if rect.width() < 16.0 {
        return;
    }
    let frac = frac.clamp(0.0, 1.0);
    // 满进度 = 细条 + 降透明 + 淡色数字。
    // 理由：条满了就不再携带信息，可列表里 12 行有 9 行都是 100% 时，
    // 一排高饱和绿条会把眼睛全部吃掉，真正该看的"还在下载的那一行"反而被淹没。
    // 压细、调淡之后，"未完成"的任务条会自己跳出来 —— 层级是靠对比做出来的。
    let done = frac >= 0.999;
    let half = if done { 1.5_f32 } else { 2.5_f32 };
    let pct_w = 44.0;
    let cy = rect.center().y;
    let track = Rect::from_min_max(
        pos2(rect.left(), cy - half),
        pos2((rect.right() - pct_w - 6.0).max(rect.left() + 6.0), cy + half),
    );

    let k = ui
        .ctx()
        .animate_value_with_time(anim, frac, 0.32)
        .clamp(0.0, 1.0);

    let painter = ui.painter();
    painter.rect_filled(track, R_SM, p.well);
    if k > 0.0 {
        let mut fill = track;
        fill.max.x = track.left() + (track.width() * k).max(1.5);
        painter.rect_filled(fill, R_SM, if done { alpha(color, 160) } else { color });
        if !done {
            // 读数头：尾端 1.5pt 高亮，刻意做出"机械表"的感觉
            painter.rect_filled(
                Rect::from_min_max(pos2((fill.max.x - 1.5).max(fill.min.x), fill.min.y), fill.max),
                R_SM,
                alpha(p.text, 200),
            );
        }
    }
    painter.text(
        pos2(rect.right(), cy),
        Align2::RIGHT_CENTER,
        format!("{:>5.1}%", frac * 100.0),
        t::id_num(11.0),
        if done { p.faint } else { p.muted },
    );
}

/// 按估算字宽截断并加省略号。
/// 不走 `layout` 逐字测量 —— 每行每列都排版一次在长列表里开销明显，
/// 用"中日韩字符 = 1em、其余 ≈ 0.55em"的估算足够准，而且 O(n)。
pub fn fit(text: &str, avail: f32, size: f32) -> String {
    let mut w = 0.0f32;
    let mut out = String::new();
    let mut cut = false;
    for ch in text.chars() {
        let cw = if (ch as u32) > 0x2E80 { size } else { size * 0.55 };
        if w + cw > avail {
            cut = true;
            break;
        }
        w += cw;
        out.push(ch);
    }
    if cut {
        while out.ends_with(' ') {
            out.pop();
        }
        out.push('…');
    }
    out
}

// ------------------------------------------------------------------ 图形元件

/// 上下箭头（几何绘制，不依赖 ↑ ↓ 字形）
/// `up = true` 画向上箭头（上传），`false` 画向下箭头（下载）
pub fn arrow(painter: &egui::Painter, center: Pos2, size: f32, up: bool, color: Color32) {
    let h = size;
    let w = size * 0.72;
    let tw = (size * 0.22).max(1.0);
    // 局部坐标：(0,0) 为箭头中心，-h/2 是尖端
    let head = -h * 0.5;
    let shoulder = -h * 0.5 + h * 0.44;
    let tail = h * 0.5;
    let f = |y: f32| if up { center.y + y } else { center.y - y };

    painter.add(Shape::convex_polygon(
        vec![
            pos2(center.x, f(head)),
            pos2(center.x - w * 0.5, f(shoulder)),
            pos2(center.x + w * 0.5, f(shoulder)),
        ],
        color,
        Stroke::NONE,
    ));

    let (a, b) = (f(tail), f(shoulder - 1.0));
    painter.rect_filled(
        Rect::from_min_max(
            pos2(center.x - tw * 0.5, a.min(b)),
            pos2(center.x + tw * 0.5, a.max(b)),
        ),
        0.0,
        color,
    );
}

/// 警告三角：实心三角 + 用底色挖出的 "!"（同样是几何绘制）
pub fn warn_tri(painter: &egui::Painter, center: Pos2, size: f32, color: Color32, hole: Color32) {
    let h = size;
    let w = size * 1.12;
    painter.add(Shape::convex_polygon(
        vec![
            pos2(center.x, center.y - h * 0.56),
            pos2(center.x - w * 0.5, center.y + h * 0.48),
            pos2(center.x + w * 0.5, center.y + h * 0.48),
        ],
        color,
        Stroke::NONE,
    ));
    let bar = Rect::from_min_max(
        pos2(center.x - 0.9, center.y - h * 0.16),
        pos2(center.x + 0.9, center.y + h * 0.14),
    );
    painter.rect_filled(bar, 0.0, hole);
    painter.circle_filled(pos2(center.x, center.y + h * 0.30), 1.0, hole);
}

/// 连接指示灯：外圈 + 内点，可带呼吸动画强度 `pulse`（0..1）
pub fn beacon(painter: &egui::Painter, center: Pos2, color: Color32, pulse: f32) {
    let r = 3.4;
    if pulse > 0.01 {
        painter.circle_filled(center, r + 2.5 * pulse, alpha(color, (70.0 * pulse) as u8));
    }
    painter.circle_filled(center, r, color);
}

// ------------------------------------------------------------------ 行背景

/// 表格行背景。
///
/// 难点：要在单元格内容**之下**着色，但行矩形只有等所有单元格画完才知道。
/// 解法是先用 `Shape::Noop` 占一个绘制槽位（槽位的插入位置决定了叠放次序），
/// 等拿到 `row.response().rect` 之后再回填真正的矩形 —— 于是背景落在文字下面。
pub struct RowBg {
    painter: egui::Painter,
    slot: Slot,
}

impl RowBg {
    /// 必须在**本行第一个单元格内**调用，才能保证槽位在内容之前
    pub fn begin(ui: &Ui) -> Self {
        let painter = ui.painter().clone();
        let slot = painter.add(Shape::Noop);
        Self { painter, slot }
    }

    /// 回填行背景。`sel` 为选中态，`hover` 是 0..1 的悬停动画值。
    pub fn paint(&self, rect: Rect, sel: bool, hover: f32, p: &Palette) {
        let round = Rounding::same(t::R_MD * 0.5);
        let mut shapes: Vec<Shape> = Vec::with_capacity(3);

        if sel {
            // 白底 + 琥珀条。原来整行铺 10% 琥珀，在浅色画布上就是一片脏棕；
            // 换白之后既醒目又不脏，也和"白色 = 可交互/当前对象"的约定一致。
            shapes.push(Shape::rect_filled(rect, round, p.row_sel));
            // 左侧 2pt 琥珀条：选中态最直白的表达
            let bar = Rect::from_min_max(
                pos2(rect.left(), rect.top() + 2.0),
                pos2(rect.left() + 2.0, rect.bottom() - 2.0),
            );
            shapes.push(Shape::rect_filled(bar, round, p.accent));
        } else if hover > 0.004 {
            // 悬停 = 半透明白。方向和选中一致（都是提亮），只是强度不同，
            // 于是三档状态形成一条递进：无 → 淡白 → 纯白 + 琥珀条。
            shapes.push(Shape::rect_filled(
                rect,
                round,
                alpha(p.surface_hi, (90.0 * hover) as u8),
            ));
        }

        // 行间发丝线（替代斑马纹 —— 高密度列表里更干净）
        if !sel {
            let y = rect.bottom() - 0.5;
            let line = Rect::from_min_max(pos2(rect.left(), y), pos2(rect.right(), y + 0.6));
            shapes.push(Shape::rect_filled(line, 0.0, alpha(p.line, 150)));
        }

        self.painter.set(self.slot, Shape::Vec(shapes));
    }
}

/// 表格表头背景 + 底部发丝线（表头是固定行，不需要占位技巧）
pub fn header_bg(painter: &egui::Painter, rect: Rect, p: &Palette) {
    painter.rect_filled(rect, 0.0, p.surface);
    let y = rect.bottom() - 0.5;
    painter.rect_filled(
        Rect::from_min_max(pos2(rect.left(), y), pos2(rect.right(), y + 0.8)),
        0.0,
        p.line_hi,
    );
}

/// 竖排"胶囊"，用于限速开关之类的状态标签（纯展示）
pub fn badge(ui: &mut Ui, p: &Palette, text: &str, color: Color32, on: bool) -> Response {
    let font = t::id_sb(10.5);
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_owned(), font, Color32::PLACEHOLDER);
    let pad = vec2(8.0, 3.0);
    let size = galley.size() + pad * 2.0;
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());

    let fill = if on { alpha(color, 34) } else { p.surface_hi };
    let stroke = if on { color } else { p.line };
    let fg = if on { color } else { p.muted };

    let k = ui
        .ctx()
        .animate_bool_with_time(resp.id.with("k"), resp.hovered(), 0.14);
    let painter = ui.painter();
    painter.rect_filled(rect, R_SM, lerp(fill, p.hover, k * 0.5));
    painter.rect_stroke(rect, R_SM, Stroke::new(1.0_f32, lerp(stroke, color, k)));
    painter.galley(rect.center() - galley.size() / 2.0, galley, fg);

    resp
}

// ------------------------------------------------------------------ 左栏导航

/// 左栏筛选条目。
///
/// 为什么不用内置的 `SelectableLabel`：它的背景只有"文字宽度"，做不出
/// 通栏高亮 + 左侧强调条 + 右对齐等宽计数这一套版式。所以这里自绘，
/// 但把可访问性补齐：
///   * `Sense::click()` 会被 egui 纳入 Tab 焦点链，`has_focus()` 可用；
///   * 聚焦时画 2pt 强调色焦点圈（焦点必须可见，这是硬性要求）；
///   * Enter / Space 激活由调用方用 `has_focus()` + `input` 判定，
///     不依赖框架是否合成键盘点击（见 `app::side_panel`）。
pub fn nav_row(ui: &mut Ui, p: &Palette, text: &str, count: Option<usize>, on: bool) -> Response {
    let w = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(vec2(w.max(40.0), 22.0), Sense::click());

    let k = ui
        .ctx()
        .animate_bool_with_time(resp.id.with("hov"), resp.hovered() || resp.has_focus(), 0.13);
    let sel = ui
        .ctx()
        .animate_bool_with_time(resp.id.with("sel"), on, 0.13);

    let painter = ui.painter();
    let round = Rounding::same(t::R_MD * 0.6);

    if on {
        // 选中项 = 白底 + 左侧琥珀条（"白色 = 当前操作对象"）。
        // 原来用 10% 琥珀整块填充，压在浅色栏底上会糊成一片脏棕，
        // 而且和悬停态只差一点透明度，两者几乎分不出来。
        painter.rect_filled(rect, round, p.surface_hi);
        painter.rect_stroke(rect, round, Stroke::new(1.0_f32, p.line));
    } else if k > 0.004 {
        painter.rect_filled(rect, round, alpha(p.hover, (170.0 * k) as u8));
    }
    // 左侧 2pt 强调条：从 0 长出来，不是硬切
    if sel > 0.01 {
        let bar = Rect::from_min_max(
            pos2(rect.left(), rect.top() + 3.0),
            pos2(rect.left() + 2.0 * sel, rect.bottom() - 3.0),
        );
        painter.rect_filled(bar, round, p.accent);
    }
    if resp.has_focus() {
        painter.rect_stroke(rect, round, Stroke::new(2.0_f32, p.accent));
    }

    painter.text(
        pos2(rect.left() + 12.0, rect.center().y),
        Align2::LEFT_CENTER,
        text,
        if on { t::id_sb(12.5) } else { t::id(12.5) },
        if on { p.text } else { p.muted },
    );
    if let Some(n) = count {
        painter.text(
            pos2(rect.right() - 8.0, rect.center().y),
            Align2::RIGHT_CENTER,
            n.to_string(),
            t::id_num(10.5),
            if on { p.accent } else { p.faint },
        );
    }

    resp
}

// ------------------------------------------------------------------ 弹窗外壳

/// 弹窗尺寸与标题。
pub struct DialogSpec<'a> {
    /// 稳定 id。决定窗口的位置记忆，**不要**用可变内容拼。
    pub id: &'a str,
    pub title: &'a str,
    /// 标题下方的小字说明，可省
    pub subtitle: Option<&'a str>,
    /// `[宽, 高]`。宽必须给正数（标题栏按它撑满整窗）；高给 0 表示自适应内容。
    pub size: [f32; 2],
    /// 允许拖边框改尺寸
    pub resizable: bool,
}

/// 标题栏高度。标题 + 副标题两行能舒适地放进 46pt。
const DLG_TITLE_H: f32 = 46.0;
/// 内容区左右留白
const DLG_PAD_X: f32 = 16.0;
/// 关闭按钮边长
const DLG_CLOSE: f32 = 24.0;

/// 统一的弹窗外壳：**自绘**标题栏（含关闭按钮）+ 内容区 + 可选页脚。
///
/// # 为什么要自己画标题栏
///
/// 不是为了好看，是因为内置那个关闭按钮**根本没有状态反馈**：
///
/// ```ignore
/// // egui-0.29.1/src/containers/window.rs:1197
/// let visuals = ui.style().interact(&response);
/// let stroke  = visuals.fg_stroke;          // ← 只取描边色，不画底
/// ui.painter().line_segment([..], stroke);  // ← 两根线就是全部
/// ```
///
/// 它唯一的可视反馈来自 `fg_stroke`；而本套主题里 `inactive / hovered /
/// active` 三态的 `fg_stroke` 都是同一个 `p.text`（见 `theme::visuals`），
/// 于是那个 × 悬停、按下、聚焦**三态完全同色** —— 点了像没反应。
///
/// 想修只有两条路：① 改全局四态文字色（会连带所有按钮文字一起变）；
/// ② 自己画。既然弹窗版式本来就要按新版语言收敛，选 ②。
///
/// 返回 `false` = 用户关掉了窗口（点 ×，或焦点不在输入框时按 Esc）。
/// 调用方接自己的逻辑，写法保持 `self.show_x = keep && !close;`
pub fn dialog(
    ctx: &egui::Context,
    p: &Palette,
    spec: DialogSpec<'_>,
    body: impl FnOnce(&mut Ui),
) -> bool {
    let mut keep = true;
    let outer_w = if spec.size[0] > 0.0 { spec.size[0] } else { 520.0 };

    let mut win = egui::Window::new(spec.id)
        .id(egui::Id::new(("qb-dlg", spec.id)))
        .title_bar(false) // ← 关键：不要内置标题栏，也就没有了那个默认的 ×
        .collapsible(false)
        .resizable(spec.resizable)
        .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
        .frame(dialog_frame(p));

    win = win.default_width(outer_w);
    // ⚠️ 高度**必须显式给**，哪怕是不限高。
    //
    // `egui::Window` 内部的 `Resize` 带着默认尺寸 `[340, 420]`
    // （egui 0.29 `containers/window.rs` 的 `Resize::default()`），
    // 而它的合成规则是 `max(default_size, content_size)`。
    // 所以只要不调用 `default_height`，每个"内容不长"的弹窗都会被
    // 硬撑到 420pt —— footer 上方凭空多出一大片空白，按钮还会被
    // `Align::Center` 推到半空。给个 1.0 的底线，`Resize` 就会老老实实
    // 按内容收拢。
    win = win.default_height(if spec.size[1] > 0.0 { spec.size[1] } else { 1.0 });

    win.show(ctx, |ui| {
        ui.set_max_width(outer_w);
        title_bar(ui, p, &spec, &mut keep, outer_w);
        egui::Frame::none()
            .inner_margin(egui::Margin {
                left: DLG_PAD_X,
                right: DLG_PAD_X,
                top: t::S3,
                bottom: t::S4,
            })
            .show(ui, |ui| {
                ui.set_width((outer_w - DLG_PAD_X * 2.0).max(120.0));
                body(ui);
            });
    });

    // Esc 关闭。但**焦点在输入框里时不动** —— 那种情况下 Esc 的语义是
    // 「结束输入」，顺手把整个弹窗关掉会让人丢掉刚敲的内容。
    let no_focus = ctx.memory(|m| m.focused().is_none());
    if no_focus && ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        keep = false;
    }

    keep
}

/// 弹窗底板：内容面 + 1pt 强边框 + 微圆角 + 最高一档阴影。
fn dialog_frame(p: &Palette) -> egui::Frame {
    egui::Frame::none()
        .fill(p.surface_hi)
        .stroke(Stroke::new(1.0_f32, p.line_hi))
        .rounding(Rounding::same(t::R_LG))
        .inner_margin(egui::Margin::same(0.0))
        .shadow(egui::epaint::Shadow {
            offset: vec2(0.0, 8.0),
            blur: 24.0,
            spread: 0.0,
            color: Color32::from_black_alpha(130),
        })
}

/// 自绘标题栏：左侧琥珀竖条 + 标题 / 副标题 + 右侧关闭按钮。
fn title_bar(ui: &mut Ui, p: &Palette, spec: &DialogSpec<'_>, keep: &mut bool, width: f32) {
    let (rect, _) = ui.allocate_exact_size(vec2(width.max(160.0), DLG_TITLE_H), Sense::hover());
    let painter = ui.painter().clone();

    // 底：比内容面暗一档，把「标题栏 / 内容区」分成两块
    painter.rect_filled(
        rect,
        Rounding {
            nw: t::R_LG,
            ne: t::R_LG,
            sw: 0.0,
            se: 0.0,
        },
        p.surface,
    );
    // 左侧 3pt 琥珀竖条：全屏唯一强调色，出现在每个弹窗左上角，
    // 让"这是个弹窗"不靠阴影也能一眼看出来。
    painter.rect_filled(
        Rect::from_min_size(
            pos2(rect.left() + 14.0, rect.center().y - 9.0),
            vec2(3.0, 18.0),
        ),
        t::R_SM,
        p.accent,
    );

    let tx = rect.left() + 26.0;
    // 右侧要留给关闭按钮，标题/副标题超长一律截断加省略号 ——
    // 任务名可以很长，让它溢出到 × 底下就点不着关闭了。
    let text_room = (rect.right() - 12.0 - DLG_CLOSE - 12.0 - tx).max(40.0);
    match spec.subtitle {
        Some(sub) => {
            painter.text(
                pos2(tx, rect.center().y - 3.0),
                Align2::LEFT_BOTTOM,
                fit(spec.title, text_room, 13.5),
                t::id_sb(13.5),
                p.text,
            );
            painter.text(
                pos2(tx, rect.center().y + 4.0),
                Align2::LEFT_TOP,
                fit(sub, text_room, 11.0),
                t::id(11.0),
                p.faint,
            );
        }
        None => {
            painter.text(
                pos2(tx, rect.center().y),
                Align2::LEFT_CENTER,
                fit(spec.title, text_room, 13.5),
                t::id_sb(13.5),
                p.text,
            );
        }
    }

    // 底部分隔线
    painter.rect_filled(
        Rect::from_min_max(
            pos2(rect.left(), rect.bottom() - 1.0),
            pos2(rect.right(), rect.bottom()),
        ),
        0.0,
        p.line_hi,
    );

    // 关闭按钮最后画（叠在标题栏底之上）
    let btn = Rect::from_min_size(
        pos2(
            rect.right() - 12.0 - DLG_CLOSE,
            rect.center().y - DLG_CLOSE * 0.5,
        ),
        vec2(DLG_CLOSE, DLG_CLOSE),
    );
    if close_button(ui, p, btn).clicked() {
        *keep = false;
    }
}

/// 自绘关闭按钮：**四态各自可分**。
///
/// 常态无底、× 用三级灰；悬停浮出底色 + 边框、× 提到一级色；
/// 按下整块转琥珀；键盘聚焦再补 2pt 焦点圈（焦点必须可见）。
pub fn close_button(ui: &mut Ui, p: &Palette, rect: Rect) -> Response {
    let resp = ui.interact(rect, ui.id().with("dlg-close"), Sense::click());
    let k = ui
        .ctx()
        .animate_bool_with_time(resp.id.with("hov"), resp.hovered() || resp.has_focus(), 0.12);
    let down = resp.is_pointer_button_down_on();
    let round = Rounding::same(t::R_SM);
    let painter = ui.painter();

    if down {
        painter.rect_filled(rect, round, p.accent_soft);
        painter.rect_stroke(rect, round, Stroke::new(1.0_f32, p.accent));
    } else if k > 0.004 {
        painter.rect_filled(rect, round, alpha(p.hover, (255.0 * k) as u8));
        painter.rect_stroke(rect, round, Stroke::new(1.0_f32, lerp(p.line, p.line_hi, k)));
    }

    let fg = if down {
        p.accent
    } else {
        lerp(p.muted, p.text, k)
    };
    // × 是几何绘制，不用 "✕ ⨯ ×" 字形
    let r = rect.shrink(DLG_CLOSE * 0.33);
    let s = Stroke::new(1.5_f32, fg);
    painter.line_segment([r.left_top(), r.right_bottom()], s);
    painter.line_segment([r.right_top(), r.left_bottom()], s);

    if resp.has_focus() {
        painter.rect_stroke(rect, round, Stroke::new(2.0_f32, p.accent));
    }

    resp.on_hover_text(tr("关闭（Esc）"))
}

/// 弹窗页脚：发丝线 + 右对齐的按钮行。
///
/// ⚠️ 内部是 `right_to_left` 布局，**先添加的按钮在最右边**。
/// 所以调用顺序是「主操作 → 次操作」，写起来是反的，特此注明：
///
/// ```ignore
/// dialog_footer(ui, p, |ui| {
///     if primary_button(ui, p, "提交").clicked() { submit = true; }
///     if ghost_button(ui, p, "取消", 0.0).clicked() { close = true; }
/// });
/// ```
pub fn dialog_footer(ui: &mut Ui, p: &Palette, body: impl FnOnce(&mut Ui)) {
    // 先把剩余高度吃掉，让按钮行**贴住弹窗底部**。
    //
    // 不这么做的话，`right_to_left(Align::Center)` 里的 `Center` 是
    // 「在剩余空间里垂直居中」—— 只要窗口比内容高，按钮就会悬在半空，
    // 上下各留一片空白。吃掉富余之后，无论弹窗多高，页脚都在最下面。
    let need = t::S3 + 1.0 + t::S2 + 30.0;
    let slack = ui.available_height() - need;
    if slack.is_finite() && slack > 0.5 {
        ui.add_space(slack);
    }
    ui.add_space(t::S3);
    hairline(ui, p);
    ui.add_space(t::S2);
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        body(ui);
    });
}

// ------------------------------------------------------------------ Tab 条

/// 详情面板的 Tab 条：文字 + 右侧小计数，当前页用**底部琥珀下划线**标记。
///
/// 返回被点击的 Tab 下标。不做成胶囊 / 方块：弹窗里已经有一个琥珀竖条和
/// 一个实心主按钮，再来一块色块就吵了 —— 下划线是重量最轻、
/// 又能明确回答"我现在在哪一页"的做法。
pub fn tab_bar(
    ui: &mut Ui,
    p: &Palette,
    tabs: &[(&str, Option<usize>)],
    cur: usize,
) -> Option<usize> {
    let full = ui.available_width();
    let (strip, _) = ui.allocate_exact_size(vec2(full.max(1.0), 30.0), Sense::hover());
    let painter = ui.painter().clone();

    // 基线一条，把 Tab 条和下面的表格分开
    painter.rect_filled(
        Rect::from_min_max(
            pos2(strip.left(), strip.bottom() - 1.0),
            pos2(strip.right(), strip.bottom()),
        ),
        0.0,
        p.line,
    );

    let font = t::id(12.5);
    let font_sb = t::id_sb(12.5);
    let font_n = t::id_num(10.0);
    let pad = 13.0;
    let mut x = strip.left();
    let mut hit = None;

    for (i, (label, count)) in tabs.iter().enumerate() {
        let on = i == cur;
        let g = painter.layout_no_wrap(
            (*label).to_owned(),
            if on { font_sb.clone() } else { font.clone() },
            Color32::PLACEHOLDER,
        );
        let cg = count
            .map(|c| painter.layout_no_wrap(c.to_string(), font_n.clone(), Color32::PLACEHOLDER));
        let tw = pad * 2.0 + g.size().x + cg.as_ref().map_or(0.0, |c| c.size().x + 6.0);
        if x + tw > strip.right() + 0.5 {
            break;
        }
        let tr = Rect::from_min_size(pos2(x, strip.top()), vec2(tw, strip.height() - 1.0));
        let resp = ui.interact(tr, ui.id().with("tab").with(i), Sense::click());
        let kb = resp.has_focus()
            && ui.input(|inp| {
                inp.key_pressed(egui::Key::Enter) || inp.key_pressed(egui::Key::Space)
            });
        if resp.clicked() || kb {
            hit = Some(i);
        }

        let k = ui
            .ctx()
            .animate_bool_with_time(resp.id.with("hov"), resp.hovered() || on, 0.13);
        if !on && k > 0.004 {
            painter.rect_filled(
                Rect::from_min_max(tr.left_top(), pos2(tr.right(), tr.bottom())),
                Rounding {
                    nw: t::R_MD,
                    ne: t::R_MD,
                    sw: 0.0,
                    se: 0.0,
                },
                alpha(p.hover, (150.0 * k) as u8),
            );
        }

        let cx = tr.left() + pad;
        painter.galley(
            pos2(cx, tr.center().y - g.size().y * 0.5),
            g.clone(),
            if on { p.text } else { lerp(p.muted, p.text, k) },
        );
        if let Some(c) = &cg {
            let n = count.unwrap_or(0);
            painter.galley(
                pos2(cx + g.size().x + 6.0, tr.center().y - c.size().y * 0.5),
                c.clone(),
                if on {
                    p.accent
                } else if n == 0 {
                    p.faint
                } else {
                    p.muted
                },
            );
        }

        // 当前页的下划线（从中间往两边长出来，不是硬切）
        if on {
            let hw = tw * 0.5;
            painter.rect_filled(
                Rect::from_min_max(
                    pos2(tr.center().x - hw, strip.bottom() - 2.0),
                    pos2(tr.center().x + hw, strip.bottom()),
                ),
                0.0,
                p.accent,
            );
        }
        if resp.has_focus() {
            painter.rect_stroke(tr, Rounding::same(t::R_SM), Stroke::new(2.0_f32, p.accent));
        }

        x += tw;
    }

    hit
}
