//! 应用主体：UI 布局 + 交互。
//!
//! # 版式
//!
//! ```text
//! ┌──────────────────────────────────────────────────────────────────┐
//! │ qBittorrent       ● 已连接        ↓ 12.4  ↑ 3.1   任务  节点     │  顶栏：品牌 + 状态 + 遥测带
//! │ 远程管理台          WebAPI 2.11     MiB/s  MiB/s    128   421    │
//! ├──────────┬───────────────────────────────────────────────────────┤
//! │ 状态      │ [搜索] [+ 添加任务] [暂停] [恢复] [删除]      工具条  │
//! │  全部 128 │ ────────────────────────────────────────────────────  │
//! │  下载中 12│  名称            大小    进度       状态   下载  上传 │  高密度表格
//! │ 分类      │  ───────────────────────────────────────────────────  │
//! │ 标签      │  xxx.iso       4.20GiB ▓▓▓░░ 62.4% 下载中 12.4M 1.2M │
//! ├──────────┴───────────────────────────────────────────────────────┤
//! │ 就绪                         任务 128 · 选中 3   DHT 421 · 连接正常│  状态栏
//! └──────────────────────────────────────────────────────────────────┘
//! ```
//!
//! 所有颜色都从 `theme::pal()` 取，主题切换只换调色板，布局代码不动。

use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use egui::{Align2, Color32, Key, RichText, Sense, Stroke, pos2, vec2};
use egui_extras::{Column, TableBuilder};

use crate::api::AddOptions;
use crate::config::Config;
use crate::model::{Peer, StateGroup, Torrent, TorrentFile, TorrentProperties, Tracker, WebSeed};
use crate::theme::{self as th, Palette};
use crate::util::{
    bytes_to_kib, fmt_duration, fmt_eta, fmt_ratio, fmt_size, fmt_speed, fmt_ts, kib_to_bytes,
    kib_to_bytes_pm,
};
use crate::widgets::{self as w, RowBg, Tele};
use crate::worker::{Cmd, Conn, Shared, Worker};

/// 遥测带底部细条的软量程：没设限速时按 20 MiB/s / 10 MiB/s 折算占用比
const SOFT_DL: f32 = 20.0 * 1024.0 * 1024.0;
const SOFT_UP: f32 = 10.0 * 1024.0 * 1024.0;
/// DHT 节点数的软量程
const SOFT_NODES: f32 = 600.0;

/// 勾选列宽度。
/// 行点击要判断「点的是不是勾选框」，判定阈值必须和表格列宽一致 ——
/// 所以抽成常量，避免改了列宽忘了改阈值。
const CHECK_W: f32 = 28.0;


// ---------------- 状态快照 ----------------

/// 从 `Shared` 拷出来的一份只读快照，避免渲染期间长时间持锁。
#[derive(Clone)]
struct Snap {
    conn: Conn,
    version: String,
    webapi: String,
    transfer: crate::model::TransferInfo,
    torrents: Arc<Vec<Torrent>>,
    categories: Vec<String>,
    tags: Vec<String>,
    alt_speed: bool,
    global_dl_limit: i64,
    global_up_limit: i64,
    last_error: Option<String>,
    last_update: Option<Instant>,
    /// 与 `Shared` 一样用 `Arc`：每帧只自增引用计数，不深拷贝日志
    log: Arc<Vec<String>>,
    /// 任务详情的五个模块（整体一份，避免出现"常规是 A、Trackers 是 B"的错位）
    detail: crate::worker::DetailData,
}

impl Default for Snap {
    fn default() -> Self {
        Self {
            conn: Conn::Disconnected,
            version: String::new(),
            webapi: String::new(),
            transfer: Default::default(),
            torrents: Arc::new(Vec::new()),
            categories: Vec::new(),
            tags: Vec::new(),
            alt_speed: false,
            global_dl_limit: 0,
            global_up_limit: 0,
            last_error: None,
            last_update: None,
            log: Arc::new(Vec::new()),
            detail: Default::default(),
        }
    }
}

impl From<&Shared> for Snap {
    fn from(s: &Shared) -> Self {
        Self {
            conn: s.conn.clone(),
            version: s.version.clone(),
            webapi: s.webapi.clone(),
            transfer: s.transfer.clone(),
            torrents: s.torrents.clone(),
            categories: s.categories.clone(),
            tags: s.tags.clone(),
            alt_speed: s.alt_speed,
            global_dl_limit: s.global_dl_limit,
            global_up_limit: s.global_up_limit,
            last_error: s.last_error.clone(),
            last_update: s.last_update,
            log: s.log.clone(),
            detail: s.detail.clone(),
        }
    }
}

// ---------------- 筛选 / 排序 ----------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Filter {
    All,
    Downloading,
    Seeding,
    Completed,
    Stopped,
    Error,
    Active,
}

impl Filter {
    fn label(&self) -> &'static str {
        match self {
            Filter::All => "全部",
            Filter::Downloading => "下载中",
            Filter::Seeding => "做种中",
            Filter::Completed => "已完成",
            Filter::Stopped => "已停止",
            Filter::Error => "错误",
            Filter::Active => "活动中",
        }
    }

    fn passes(&self, t: &Torrent) -> bool {
        match self {
            Filter::All => true,
            Filter::Downloading => t.group() == StateGroup::Downloading,
            Filter::Seeding => t.group() == StateGroup::Seeding,
            Filter::Completed => t.is_completed(),
            Filter::Stopped => t.is_stopped(),
            Filter::Error => t.group() == StateGroup::Error,
            Filter::Active => t.is_active(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SortKey {
    Name,
    Size,
    Progress,
    State,
    DlSpeed,
    UpSpeed,
    Ratio,
    Eta,
    Category,
    Added,
}

fn cmp_torrent(a: &Torrent, b: &Torrent, k: SortKey) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    match k {
        SortKey::Name => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
        SortKey::Size => a.size.cmp(&b.size),
        SortKey::Progress => a.progress.partial_cmp(&b.progress).unwrap_or(Ordering::Equal),
        SortKey::State => a.state.cmp(&b.state),
        SortKey::DlSpeed => a.dlspeed.cmp(&b.dlspeed),
        SortKey::UpSpeed => a.upspeed.cmp(&b.upspeed),
        SortKey::Ratio => a.ratio.partial_cmp(&b.ratio).unwrap_or(Ordering::Equal),
        SortKey::Eta => a.eta.cmp(&b.eta),
        SortKey::Category => a.category.cmp(&b.category),
        SortKey::Added => a.added_on.cmp(&b.added_on),
    }
}

/// 表格里收集到的操作，渲染结束后统一执行，避开闭包借用冲突。
///
/// 之所以「先收集、后执行」：`TableBuilder` 的 body 闭包是 `FnMut`，
/// 在它里面直接改 `self` 会和 `ui` 的可变借用打架。
enum Action {
    /// 切换某一行的选中态（点勾选框 / Ctrl+点击 / 回车）
    Toggle(String),
    /// 只选中这一行（普通单击）
    SelectOnly(String),
    OpenDetail(String),
    Pause(Vec<String>),
    Resume(Vec<String>),
    Recheck(Vec<String>),
    Reannounce(Vec<String>),
    ForceStart(Vec<String>, bool),
    Sequential(Vec<String>, bool),
    SetCategory(Vec<String>, String),
    AddTags(Vec<String>, String),
    RemoveTags(Vec<String>, String),
    /// 打开「移动保存路径」窗口。第二个参数是预填路径（单选时用当前路径）。
    MoveTo(Vec<String>, String),
    Delete(Vec<String>),
    CopyHash(String),
}

// ---------------- 弹窗：任务详情 ----------------

/// 详情面板的五个模块，与 qBittorrent 原生 WebUI 的 Torrent details 一一对应。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DetailTab {
    General,
    Trackers,
    Peers,
    HttpSources,
    Content,
}

impl DetailTab {
    const ALL: [DetailTab; 5] = [
        Self::General,
        Self::Trackers,
        Self::Peers,
        Self::HttpSources,
        Self::Content,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::General => "常规",
            Self::Trackers => "Trackers",
            Self::Peers => "节点",
            Self::HttpSources => "HTTP 源",
            Self::Content => "内容",
        }
    }

    /// 显式给出下标，不依赖枚举的声明顺序和 `as usize`
    fn idx(self) -> usize {
        match self {
            Self::General => 0,
            Self::Trackers => 1,
            Self::Peers => 2,
            Self::HttpSources => 3,
            Self::Content => 4,
        }
    }
}

// ---------------- App ----------------

/// eframe 持久化用的键
const PREF_KEY: &str = "qb-prefs";

/// 窗口尺寸下限（点）。再小主表格就只剩名字列了。
pub const WIN_MIN: egui::Vec2 = egui::vec2(1040.0, 640.0);
/// 没有历史偏好时的默认尺寸（点）。
pub const WIN_DEFAULT: egui::Vec2 = egui::vec2(1440.0, 900.0);
/// 允许窗口占用的显示器比例 —— 剩下的留给任务栏和窗口边框。
const WIN_FIT: f32 = 0.94;

/// 需要跨次启动记住的界面偏好。
///
/// 连接参数不在这里（那属于 `Config`，用户要能直接看/改 JSON）；
/// 这里只放"用户不会去手改、但不想每次重设"的界面状态。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct Prefs {
    dark: bool,
    /// ⚠️ 单位是**点**（logical point），不是物理像素。
    ///
    /// 存点的理由：用户在意的是"窗口里能放多少东西"，这件事跟着点走。
    /// 存像素的话，同一份尺寸搬到 200% 的屏上会缩成一半大。
    /// 代价是**必须按当前显示器重新钳一次**才能用 —— 见 `update` 里的
    /// 显示器自适应块。
    width: f32,
    height: f32,
}

impl Default for Prefs {
    fn default() -> Self {
        Self {
            dark: true,
            width: WIN_DEFAULT.x,
            height: WIN_DEFAULT.y,
        }
    }
}

pub struct QbApp {
    worker: Worker,
    shared: Arc<Mutex<Shared>>,
    cfg: Config,
    /// 上一帧的窗口内容尺寸，退出时写进 `Prefs`
    screen_size: egui::Vec2,
    /// 是否已经把历史尺寸按显示器可用区套用过一次
    size_applied: bool,
    /// 上一帧算出的显示器**可用区**（点）。
    /// 换屏 / 改缩放 / 插拔外接屏时它会变，用来触发重新钳制。
    last_avail: Option<egui::Vec2>,

    // 弹窗开关
    show_settings: bool,
    show_add: bool,
    show_delete: bool,
    show_limits: bool,
    show_log: bool,
    show_new_category: bool,
    show_detail: bool,
    show_move: bool,

    // 添加表单
    add_urls: String,
    add_savepath: String,
    add_category: String,
    add_tags: String,
    add_paused: bool,
    add_sequential: bool,

    // 删除表单
    del_files: bool,

    // 限速表单（KiB/s）
    lim_dl: i64,
    lim_up: i64,
    lim_torrent_dl: i64,
    lim_torrent_up: i64,

    // 新建分类
    new_cat_name: String,
    new_cat_path: String,

    // 移动保存路径
    move_path: String,

    // 批量打标签
    bulk_tag: String,

    // 筛选 / 排序
    search: String,
    filter: Filter,
    sel_category: Option<String>,
    sel_tag: Option<String>,
    selected: HashSet<String>,
    sort_key: SortKey,
    sort_desc: bool,

    // 详情
    detail_hash: Option<String>,
    /// 当前停在哪个 Tab（跨次打开记住，常用的那个不用每次重找）
    detail_tab: DetailTab,

    dark: bool,
    theme_dirty: bool,
    /// 标题栏配色是否已经同步过一次。窗口句柄要等第一帧才拿得到，
    /// 所以不能在 `new()` 里做。
    titlebar_done: bool,
    toast: Option<(String, Instant, bool)>,

    /// 当前主题调色板。`'static` 引用，可以直接复制进闭包，不产生借用冲突。
    p: &'static Palette,
}

impl QbApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        // 上一次运行留下的界面偏好（主题 / 窗口尺寸）
        let prefs: Prefs = cc
            .storage
            .and_then(|s| eframe::get_value(s, PREF_KEY))
            .unwrap_or_default();

        // 顺序有讲究：字体先装好，样式里的字阶引用了自定义字体族名
        crate::fonts::install(&cc.egui_ctx);
        th::install(&cc.egui_ctx, prefs.dark);

        // 历史尺寸**不当场套用** —— 它必须被钳进当前显示器的可用区，
        // 而显示器信息要等 viewport 建好才拿得到（见 `update` 的首帧处理）。
        let want = if prefs.width >= 600.0 && prefs.height >= 400.0 {
            egui::vec2(prefs.width, prefs.height)
        } else {
            WIN_DEFAULT
        };

        let cfg = Config::load();
        let worker = Worker::spawn(cfg.clone());
        let shared = worker.shared.clone();

        Self {
            worker,
            shared,
            cfg,
            screen_size: want,
            size_applied: false,
            last_avail: None,
            show_settings: false,
            show_add: false,
            show_delete: false,
            show_limits: false,
            show_log: false,
            show_new_category: false,
            show_detail: false,
            show_move: false,
            add_urls: String::new(),
            add_savepath: String::new(),
            add_category: String::new(),
            add_tags: String::new(),
            add_paused: false,
            add_sequential: false,
            del_files: false,
            lim_dl: 0,
            lim_up: 0,
            lim_torrent_dl: -1,
            lim_torrent_up: -1,
            new_cat_name: String::new(),
            new_cat_path: String::new(),
            move_path: String::new(),
            bulk_tag: String::new(),
            search: String::new(),
            filter: Filter::All,
            sel_category: None,
            sel_tag: None,
            selected: HashSet::new(),
            sort_key: SortKey::Added,
            sort_desc: true,
            detail_hash: None,
            detail_tab: DetailTab::General,
            dark: prefs.dark,
            theme_dirty: false,
            titlebar_done: false,
            toast: None,
            p: th::pal(prefs.dark),
        }
    }

    fn snapshot(&self) -> Snap {
        match self.shared.lock() {
            Ok(g) => Snap::from(&*g),
            Err(_) => Snap::default(),
        }
    }

    fn toast_ok(&mut self, msg: impl Into<String>) {
        self.toast = Some((msg.into(), Instant::now(), false));
    }

    fn toast_err(&mut self, msg: impl Into<String>) {
        self.toast = Some((msg.into(), Instant::now(), true));
    }

    fn add_opts(&self) -> AddOptions {
        AddOptions {
            savepath: self.add_savepath.trim().to_string(),
            category: self.add_category.trim().to_string(),
            tags: self.add_tags.trim().to_string(),
            paused: self.add_paused,
            sequential: self.add_sequential,
        }
    }

    /// 当前选中的任务 hash 列表
    fn targets(&self) -> Vec<String> {
        let mut v: Vec<String> = self.selected.iter().cloned().collect();
        v.sort();
        v
    }

    // ---------------- 顶栏：品牌 + 连接状态 + 遥测带 ----------------

    fn top_bar(&mut self, ctx: &egui::Context, snap: &Snap) {
        let p = th::pal(self.dark);
        let connected = snap.conn == Conn::Connected;

        let frame = egui::Frame::none()
            .fill(p.surface)
            .inner_margin(egui::Margin::symmetric(th::S4, th::S2));

        egui::TopBottomPanel::top("top_bar")
            .frame(frame)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = th::S4;

                    // ---------- 品牌 ----------
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 1.0;
                        ui.label(
                            RichText::new("qBittorrent")
                                .font(th::id_display(15.5))
                                .color(p.text),
                        );
                        ui.label(
                            RichText::new("远程管理台")
                                .font(th::id_sb(9.5))
                                .color(p.accent),
                        );
                    });

                    w::vsep(ui, p, 30.0);

                    // ---------- 连接状态 ----------
                    let (dot, title, sub) = match &snap.conn {
                        Conn::Connected => (
                            p.up,
                            "已连接".to_string(),
                            if snap.version.is_empty() {
                                "qBittorrent".to_string()
                            } else {
                                format!("qBittorrent {}", snap.version)
                            },
                        ),
                        Conn::Connecting => (p.warn, "连接中".to_string(), "正在握手".to_string()),
                        Conn::Disconnected => {
                            (p.faint, "未连接".to_string(), "点右侧「连接设置」".to_string())
                        }
                        Conn::Error(_) => (p.err, "连接失败".to_string(), "见底部提示".to_string()),
                    };
                    let (brect, _b) = ui.allocate_exact_size(vec2(12.0, 30.0), Sense::hover());
                    w::beacon(
                        ui.painter(),
                        brect.center(),
                        dot,
                        if connected { 0.55 } else { 0.0 },
                    );
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 1.0;
                        let _ = ui
                            .label(RichText::new(title).font(th::id_sb(12.5)).color(dot))
                            .on_hover_text(if snap.webapi.is_empty() {
                                self.cfg.base_url()
                            } else {
                                format!("{}\nWebAPI {}", self.cfg.base_url(), snap.webapi)
                            });
                        ui.label(RichText::new(sub).font(th::id(9.5)).color(p.faint));
                    });

                    // ---------- 右侧遥测带 ----------
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.spacing_mut().item_spacing.x = th::S3;

                        let (dv, du) = split_speed(snap.transfer.dl_info_speed);
                        let (uv, uu) = split_speed(snap.transfer.up_info_speed);
                        let nodes = snap.transfer.dht_nodes;
                        let conn_txt = conn_status_cn(&snap.transfer.connection_status);
                        let total = snap.torrents.len();
                        let active = snap.torrents.iter().filter(|t| t.is_active()).count();
                        let act_txt = format!("活动 {}", active);

                        // 从右往左排：节点 → 任务 → 上传 → 下载
                        w::telemetry(
                            ui,
                            p,
                            &Tele {
                                arrow: None,
                                label: "节点",
                                value: nodes.to_string(),
                                unit: &conn_txt,
                                color: p.muted,
                                frac: nodes as f32 / SOFT_NODES,
                            },
                            108.0,
                        );
                        w::vsep(ui, p, 30.0);

                        w::telemetry(
                            ui,
                            p,
                            &Tele {
                                arrow: None,
                                label: "任务",
                                value: total.to_string(),
                                unit: &act_txt,
                                color: p.text,
                                frac: if total > 0 {
                                    active as f32 / total as f32
                                } else {
                                    0.0
                                },
                            },
                            88.0,
                        );
                        w::vsep(ui, p, 30.0);

                        w::telemetry(
                            ui,
                            p,
                            &Tele {
                                arrow: Some(true),
                                label: "上传",
                                value: uv,
                                unit: &uu,
                                color: if snap.transfer.up_info_speed > 0 {
                                    p.up
                                } else {
                                    p.faint
                                },
                                frac: rate_frac(
                                    snap.transfer.up_info_speed,
                                    snap.global_up_limit,
                                    SOFT_UP,
                                ),
                            },
                            114.0,
                        );
                        w::vsep(ui, p, 30.0);

                        w::telemetry(
                            ui,
                            p,
                            &Tele {
                                arrow: Some(false),
                                label: "下载",
                                value: dv,
                                unit: &du,
                                color: if snap.transfer.dl_info_speed > 0 {
                                    p.dl
                                } else {
                                    p.faint
                                },
                                frac: rate_frac(
                                    snap.transfer.dl_info_speed,
                                    snap.global_dl_limit,
                                    SOFT_DL,
                                ),
                            },
                            114.0,
                        );
                    });
                });
            });
    }


    // ---------------- 左栏：状态 / 分类 / 标签 ----------------

    fn side_panel(&mut self, ctx: &egui::Context, snap: &Snap) {
        let p = th::pal(self.dark);
        let total = snap.torrents.len();
        let cnt = |f: Filter| snap.torrents.iter().filter(|t| f.passes(t)).count();

        egui::SidePanel::left("side")
            .resizable(true)
            .default_width(208.0)
            .min_width(168.0)
            .max_width(340.0)
            .frame(
                egui::Frame::none()
                    .fill(p.surface)
                    .inner_margin(egui::Margin::symmetric(th::S3, th::S2)),
            )
            .show(ctx, |ui| {
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        // ---------- 状态 ----------
                        w::section(ui, p, "状态");
                        for f in [
                            Filter::All,
                            Filter::Downloading,
                            Filter::Seeding,
                            Filter::Completed,
                            Filter::Stopped,
                            Filter::Error,
                            Filter::Active,
                        ] {
                            let n = if f == Filter::All { total } else { cnt(f) };
                            let on = self.filter == f;
                            let r = w::nav_row(ui, p, f.label(), Some(n), on);
                            if r.clicked() || kb_activated(ui, &r) {
                                self.filter = f;
                            }
                        }

                        // ---------- 分类 ----------
                        w::section(ui, p, "分类");
                        let r = w::nav_row(ui, p, "全部", None, self.sel_category.is_none());
                        if r.clicked() || kb_activated(ui, &r) {
                            self.sel_category = None;
                        }
                        for c in &snap.categories {
                            let on = self.sel_category.as_deref() == Some(c.as_str());
                            let r = w::nav_row(ui, p, c.as_str(), None, on);
                            if r.clicked() || kb_activated(ui, &r) {
                                self.sel_category = if on { None } else { Some(c.clone()) };
                            }
                        }
                        if snap.categories.is_empty() {
                            ui.add_space(2.0);
                            let _ = ui.label(
                                RichText::new("（还没有分类）").font(th::id(11.0)).color(p.faint),
                            );
                        }

                        // ---------- 标签 ----------
                        w::section(ui, p, "标签");
                        let r = w::nav_row(ui, p, "全部", None, self.sel_tag.is_none());
                        if r.clicked() || kb_activated(ui, &r) {
                            self.sel_tag = None;
                        }
                        for g in &snap.tags {
                            let on = self.sel_tag.as_deref() == Some(g.as_str());
                            let r = w::nav_row(ui, p, g.as_str(), None, on);
                            if r.clicked() || kb_activated(ui, &r) {
                                self.sel_tag = if on { None } else { Some(g.clone()) };
                            }
                        }
                        if snap.tags.is_empty() {
                            ui.add_space(2.0);
                            let _ = ui.label(
                                RichText::new("（还没有标签）").font(th::id(11.0)).color(p.faint),
                            );
                        }

                        // ---------- 维护 ----------
                        w::section(ui, p, "维护");
                        ui.add_space(th::S1);
                        if ui.button("+ 新建分类").clicked() {
                            self.show_new_category = true;
                        }
                        ui.add_space(th::S5);
                    });
            });
    }


    // ---------------- 状态栏 ----------------

    fn bottom_bar(&mut self, ctx: &egui::Context, snap: &Snap) {
        let p = th::pal(self.dark);
        let frame = egui::Frame::none()
            .fill(p.surface)
            .inner_margin(egui::Margin::symmetric(th::S4, th::S1 + 2.0));

        egui::TopBottomPanel::bottom("bottom_bar")
            .frame(frame)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = th::S2;

                    // ---- 左：toast > 错误 > 最近一条日志 > 就绪 ----
                    let mut shown = false;
                    if let Some((msg, at, is_err)) = self.toast.clone() {
                        if at.elapsed() < Duration::from_secs(5) {
                            let c = if is_err { p.err } else { p.up };
                            // 淡入：用存活时间驱动透明度，不做硬切
                            let a = ((at.elapsed().as_secs_f32() / 0.18).clamp(0.0, 1.0)
                                * 255.0) as u8;
                            let (rect, _) =
                                ui.allocate_exact_size(vec2(th::S3, 14.0), Sense::hover());
                            let cy = rect.center().y;
                            if is_err {
                                w::warn_tri(ui.painter(), pos2(rect.center().x, cy), 11.0, c, p.bg);
                            } else {
                                ui.painter().circle_filled(pos2(rect.center().x, cy), 3.3, c);
                            }
                            ui.label(
                                RichText::new(msg)
                                    .font(th::id_sb(11.5))
                                    .color(w::alpha(c, a)),
                            );
                            shown = true;
                        } else {
                            self.toast = None;
                        }
                    }

                    if !shown {
                        if let Some(e) = &snap.last_error {
                            let (rect, _) =
                                ui.allocate_exact_size(vec2(th::S3, 14.0), Sense::hover());
                            w::warn_tri(ui.painter(), rect.center(), 12.0, p.err, p.bg);
                            let _ = ui.label(
                                RichText::new(e.as_str()).font(th::id(11.5)).color(p.err),
                            );
                        } else if let Some(line) = snap.log.last() {
                            let _ =
                                ui.label(RichText::new(line.as_str()).font(th::id(11.5)).color(p.faint));
                        } else {
                            let _ =
                                ui.label(RichText::new("就绪").font(th::id(11.5)).color(p.faint));
                        }
                    }

                    // ---- 右：统计读数 ----
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.spacing_mut().item_spacing.x = th::S2;

                        if let Some(t) = snap.last_update {
                            let _ = ui.label(
                                RichText::new(format!("{:.0}", t.elapsed().as_secs_f32()))
                                    .font(th::id_num(10.5))
                                    .color(p.muted),
                            );
                            let _ = ui.label(
                                RichText::new(w::spaced("刷新")).font(th::id_sb(9.0)).color(p.faint),
                            );
                            w::vsep(ui, p, 12.0);
                        }

                        let _ = ui.label(
                            RichText::new(conn_status_cn(&snap.transfer.connection_status))
                                .font(th::id(10.5))
                                .color(p.muted),
                        );
                        w::vsep(ui, p, 12.0);

                        stat_rtl(ui, p, "节点", &snap.transfer.dht_nodes.to_string());
                        stat_rtl(ui, p, "选中", &self.selected.len().to_string());
                        stat_rtl(ui, p, "任务", &snap.torrents.len().to_string());
                    });
                });
            });
    }


    // ---------------- 工具条 ----------------

    fn toolbar(&mut self, ui: &mut egui::Ui, snap: &Snap, p: &Palette) {
        let has_sel = !self.selected.is_empty();
        let connected = snap.conn == Conn::Connected;
        let bh = 24.0;

        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = th::S2;

            // ---- 搜索 ----
            let search_ed = ui.add(
                egui::TextEdit::singleline(&mut self.search)
                    .hint_text("搜索名称 / 分类 / 标签 / hash")
                    .desired_width(230.0)
                    .font(th::id(12.5)),
            );
            if !self.search.is_empty() && ghost_button(ui, p, "清除", 36.0)
                .on_hover_text("清空搜索条件（Esc）")
                .clicked()
            {
                self.search.clear();
            }
            // Esc 清空搜索。只在搜索框有焦点时生效，免得和「关窗口」抢这个键
            if search_ed.has_focus() && ui.input(|i| i.key_pressed(Key::Escape)) {
                self.search.clear();
            }

            w::vsep(ui, p, 22.0);

            // ---- 主操作 ----
            if primary_button(ui, p, "+ 添加任务").clicked() {
                self.show_add = true;
            }

            // ---- 批量操作（需连接 + 有选中）----
            let on = has_sel && connected;
            if ui
                .add_enabled(
                    on,
                    egui::Button::new(RichText::new("暂停").font(th::id(12.5)))
                        .min_size(vec2(50.0, bh)),
                )
                .clicked()
            {
                let h = self.targets();
                self.worker.send(Cmd::Pause(h));
            }
            if ui
                .add_enabled(
                    on,
                    egui::Button::new(RichText::new("恢复").font(th::id(12.5)))
                        .min_size(vec2(50.0, bh)),
                )
                .clicked()
            {
                let h = self.targets();
                self.worker.send(Cmd::Resume(h));
            }
            if ui
                .add_enabled(
                    on,
                    egui::Button::new(RichText::new("删除").font(th::id(12.5)))
                        .min_size(vec2(50.0, bh)),
                )
                .clicked()
            {
                self.show_delete = true;
            }

            if has_sel {
                let _ = ui.label(
                    RichText::new(format!("已选 {}", self.selected.len()))
                        .font(th::id_num(11.0))
                        .color(p.accent),
                );
            }

            // ---- 右侧：设置 / 日志 / 主题 / 限速 ----
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = th::S2;

                if ui
                    .add(egui::Button::new(RichText::new("连接设置").font(th::id(12.5))).min_size(vec2(0.0, bh)))
                    .clicked()
                {
                    self.show_settings = true;
                }
                if ghost_button(ui, p, "日志", 44.0).clicked() {
                    self.show_log = true;
                }
                w::vsep(ui, p, 22.0);

                let theme_txt = if self.dark { "浅色" } else { "深色" };
                if ghost_button(ui, p, theme_txt, 48.0)
                    .on_hover_text("切换深浅色主题")
                    .clicked()
                {
                    self.dark = !self.dark;
                    self.theme_dirty = true;
                }

                if ui
                    .add(egui::Button::new(RichText::new("限速").font(th::id(12.5))).min_size(vec2(44.0, bh)))
                    .clicked()
                {
                    self.lim_dl = bytes_to_kib(snap.global_dl_limit);
                    self.lim_up = bytes_to_kib(snap.global_up_limit);
                    // 如果恰好只选中一个任务，顺手把它的独立限速也读出来回填 ——
                    // 否则输入框会一直停留在上次的值，很容易误改。
                    let sel = self.targets();
                    let one = if sel.len() == 1 {
                        snap.torrents.iter().find(|t| t.hash == sel[0])
                    } else {
                        None
                    };
                    match one {
                        Some(t) => {
                            // qB 单任务限速：-1 = 跟随全局，0 = 不限，>0 = bytes/s
                            self.lim_torrent_dl =
                                if t.dl_limit < 0 { -1 } else { bytes_to_kib(t.dl_limit) };
                            self.lim_torrent_up =
                                if t.up_limit < 0 { -1 } else { bytes_to_kib(t.up_limit) };
                        }
                        None => {
                            self.lim_torrent_dl = -1;
                            self.lim_torrent_up = -1;
                        }
                    }
                    self.show_limits = true;
                }

                if toggle_button(ui, p, "备用限速", snap.alt_speed, 0.0)
                    .on_hover_text("临时切换到备用限速方案")
                    .clicked()
                {
                    self.worker.send(Cmd::ToggleAltSpeed);
                }

                if connected
                    && ui
                        .add(
                            egui::Button::new(RichText::new("断开").font(th::id(12.5)))
                                .min_size(vec2(44.0, bh)),
                        )
                        .clicked()
                {
                    self.worker.send(Cmd::Disconnect);
                }
            });
        });

        // ---- 第二行：批量打分类 / 标签（只在有选中时占位）----
        if has_sel {
            ui.add_space(th::S1);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = th::S2;
                let _ = ui.label(
                    RichText::new(w::spaced("批量"))
                        .font(th::id_sb(9.0))
                        .color(p.faint),
                );

                egui::ComboBox::from_id_salt("bulk_cat")
                    .selected_text(RichText::new("设为分类…").font(th::id(12.0)))
                    .width(126.0)
                    .show_ui(ui, |ui| {
                        if ui.selectable_label(false, "（清空分类）").clicked() {
                            let h = self.targets();
                            self.worker.send(Cmd::SetCategory {
                                hashes: h,
                                category: String::new(),
                            });
                        }
                        for c in &snap.categories {
                            if ui.selectable_label(false, c.as_str()).clicked() {
                                let h = self.targets();
                                self.worker.send(Cmd::SetCategory {
                                    hashes: h,
                                    category: c.clone(),
                                });
                            }
                        }
                    });

                ui.add(
                    egui::TextEdit::singleline(&mut self.bulk_tag)
                        .desired_width(96.0)
                        .hint_text("标签名")
                        .font(th::id(12.0)),
                );
                if ui.button("+ 打标签").clicked() {
                    let t = self.bulk_tag.trim().to_string();
                    if !t.is_empty() {
                        let h = self.targets();
                        self.worker.send(Cmd::AddTags { hashes: h, tags: t });
                        self.bulk_tag.clear();
                    }
                }
                w::vsep(ui, p, 18.0);
                if ghost_button(ui, p, "全选", 44.0).clicked() {
                    for x in snap.torrents.iter() {
                        self.selected.insert(x.hash.clone());
                    }
                }
                if ghost_button(ui, p, "取消选择", 60.0).clicked() {
                    self.selected.clear();
                }
            });
        }
    }

    // ---------------- 空态 ----------------

    fn empty_state(&self, ui: &mut egui::Ui, p: &Palette, snap: &Snap) {
        let avail_w = ui.available_width();
        let avail_h = ui.available_height();
        let (rect, _) = ui.allocate_exact_size(
            vec2(avail_w.max(80.0), avail_h.max(120.0)),
            Sense::hover(),
        );
        let c = rect.center();
        let painter = ui.painter();

        // 十字准星：工业图纸的"没有目标"标记
        let box_rect = egui::Rect::from_center_size(pos2(c.x, c.y - 30.0), vec2(56.0, 56.0));
        let mut r = egui::Rounding::same(th::R_MD);
        r.nw = 0.0;
        r.se = 0.0;
        painter.rect_stroke(box_rect, r, Stroke::new(1.0_f32, p.line_hi));
        painter.line_segment(
            [
                pos2(box_rect.left(), box_rect.center().y),
                pos2(box_rect.right(), box_rect.center().y),
            ],
            Stroke::new(1.0_f32, p.line),
        );
        painter.line_segment(
            [
                pos2(box_rect.center().x, box_rect.top()),
                pos2(box_rect.center().x, box_rect.bottom()),
            ],
            Stroke::new(1.0_f32, p.line),
        );
        painter.circle_filled(box_rect.center(), 3.5, p.accent);

        let msg = if snap.torrents.is_empty() {
            "还没有任务"
        } else {
            "当前筛选条件下没有任务"
        };
        painter.text(
            pos2(c.x, c.y + 14.0),
            Align2::CENTER_CENTER,
            msg,
            th::id_sb(13.5),
            p.muted,
        );
        painter.text(
            pos2(c.x, c.y + 36.0),
            Align2::CENTER_CENTER,
            if snap.torrents.is_empty() {
                "点工具条上的「+ 添加任务」，提交 magnet 链接或 .torrent 文件"
            } else {
                "试着放宽左侧筛选，或清空搜索框"
            },
            th::id(11.0),
            p.faint,
        );
    }

    // ---------------- 主表格 ----------------

    fn table(&mut self, ctx: &egui::Context, snap: &Snap) {
        let p = th::pal(self.dark);

        egui::CentralPanel::default()
            .frame(egui::Frame::none().inner_margin(egui::Margin::symmetric(th::S4, th::S2)))
            .show(ctx, |ui| {
                self.toolbar(ui, snap, p);
                ui.add_space(th::S2);
                w::hairline(ui, p);
                ui.add_space(th::S1);

                // ---------- 过滤 ----------
                let q = self.search.trim().to_lowercase();
                let mut rows: Vec<&Torrent> = snap
                    .torrents
                    .iter()
                    .filter(|t| {
                        if !q.is_empty() {
                            let hit = t.name.to_lowercase().contains(&q)
                                || t.hash.to_lowercase().starts_with(&q)
                                || t.category.to_lowercase().contains(&q)
                                || t.tags.to_lowercase().contains(&q);
                            if !hit {
                                return false;
                            }
                        }
                        if !self.filter.passes(t) {
                            return false;
                        }
                        if let Some(c) = &self.sel_category {
                            if &t.category != c {
                                return false;
                            }
                        }
                        if let Some(g) = &self.sel_tag {
                            if !t.tag_list().contains(&g.as_str()) {
                                return false;
                            }
                        }
                        true
                    })
                    .collect();

                // ---------- 排序 ----------
                let sk = self.sort_key;
                let sd = self.sort_desc;
                rows.sort_by(|a, b| {
                    let o = cmp_torrent(a, b, sk);
                    if sd {
                        o.reverse()
                    } else {
                        o
                    }
                });

                if rows.is_empty() {
                    self.empty_state(ui, p, snap);
                    return;
                }

                // ---------- 表格 ----------
                let selected = RefCell::new(std::mem::take(&mut self.selected));
                let actions: RefCell<Vec<Action>> = RefCell::new(Vec::new());
                let new_sort: Cell<Option<(SortKey, bool)>> = Cell::new(None);
                // 复用同一个槽位容器：`rows` 是顺序驱动的，每行开始时覆盖、结束时取走
                let row_bg: RefCell<Option<RowBg>> = RefCell::new(None);

                // 修饰键 / 按键状态每帧读一次就够，不必每行读一遍
                let (ctrl, shift) = ctx.input(|i| {
                    (i.modifiers.ctrl || i.modifiers.command, i.modifiers.shift)
                });
                let (enter_key, space_key) =
                    ctx.input(|i| (i.key_pressed(Key::Enter), i.key_pressed(Key::Space)));
                // 右键菜单里的分类 / 标签清单，提前取好
                let cats: &Vec<String> = &snap.categories;
                let all_tags: &Vec<String> = &snap.tags;

                table_borderless(ui);
                TableBuilder::new(ui)
                    .id_salt("torrents")
                    .striped(false)
                    .resizable(true)
                    .sense(Sense::click())
                    .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
                    .auto_shrink([false, false])
                    .min_scrolled_height(120.0)
                    .column(Column::exact(CHECK_W))
                    .column(Column::remainder().at_least(190.0).clip(true))
                    .column(Column::initial(84.0).at_least(72.0))
                    .column(Column::initial(122.0).at_least(98.0))
                    .column(Column::initial(106.0).at_least(86.0))
                    .column(Column::initial(92.0).at_least(74.0))
                    .column(Column::initial(92.0).at_least(74.0))
                    .column(Column::initial(62.0).at_least(52.0))
                    .column(Column::initial(72.0).at_least(56.0))
                    .column(Column::initial(78.0).at_least(58.0).clip(true))
                    .column(Column::initial(134.0).at_least(70.0).clip(true))
                    .header(th::HEADER_H, |mut header| {
                        header.col(|ui| header_cell_bg(ui, p));
                        sortable_header(&mut header, "名称", SortKey::Name, sk, sd, &new_sort, p);
                        sortable_header(&mut header, "大小", SortKey::Size, sk, sd, &new_sort, p);
                        sortable_header(
                            &mut header,
                            "进度",
                            SortKey::Progress,
                            sk,
                            sd,
                            &new_sort,
                            p,
                        );
                        sortable_header(&mut header, "状态", SortKey::State, sk, sd, &new_sort, p);
                        sortable_header(&mut header, "下载", SortKey::DlSpeed, sk, sd, &new_sort, p);
                        sortable_header(&mut header, "上传", SortKey::UpSpeed, sk, sd, &new_sort, p);
                        sortable_header(&mut header, "分享率", SortKey::Ratio, sk, sd, &new_sort, p);
                        sortable_header(&mut header, "剩余", SortKey::Eta, sk, sd, &new_sort, p);
                        sortable_header(
                            &mut header,
                            "分类",
                            SortKey::Category,
                            sk,
                            sd,
                            &new_sort,
                            p,
                        );
                        header.col(|ui| {
                            header_cell_bg(ui, p);
                            let _ = ui.label(
                                RichText::new(w::spaced("标签"))
                                    .font(th::id_sb(10.0))
                                    .color(p.faint),
                            );
                        });
                    })
                    .body(|body| {
                        body.rows(th::ROW_H, rows.len(), |mut row| {
                            let idx = row.index();
                            let t = rows[idx];
                            let hash = t.hash.clone();
                            let group = t.group();
                            let is_sel = selected.borrow().contains(&hash);

                            // 勾选列：只画不点（点击统一由整行响应处理），
                            // 同时顺手占下本行背景的绘制槽位
                            row.col(|ui| {
                                *row_bg.borrow_mut() = Some(RowBg::begin(ui));
                                let _ = w::check_mark(ui, p, is_sel, 13.0);
                            });

                            // 名称
                            row.col(|ui| {
                                let col = if t.is_stopped() { p.faint } else { p.text };
                                let _ = ui
                                    .add(
                                        egui::Label::new(
                                            RichText::new(t.name.as_str()).font(th::id(12.5)).color(col),
                                        )
                                        .truncate()
                                        .selectable(false),
                                    )
                                    .on_hover_text(format!(
                                        "{}\n保存路径：{}\nhash：{}",
                                        t.name, t.save_path, t.hash
                                    ));
                            });

                            // 大小
                            row.col(|ui| w::num_cell(ui, p, &fmt_size(t.size), p.muted));

                            // 进度
                            row.col(|ui| {
                                let frac = t.progress.clamp(0.0, 1.0) as f32;
                                let col = if t.is_completed() {
                                    p.up
                                } else if t.is_stopped() {
                                    p.muted
                                } else {
                                    p.dl
                                };
                                w::progress(
                                    ui,
                                    p,
                                    egui::Id::new(("qb-prog", hash.as_str())),
                                    frac,
                                    col,
                                );
                            });

                            // 状态
                            row.col(|ui| {
                                let peers = if t.num_seeds > 0 || t.num_leechs > 0 {
                                    Some(format!("{}/{}", t.num_seeds, t.num_leechs))
                                } else {
                                    None
                                };
                                w::status_cell(ui, p, t.state_cn(), state_color(p, group), peers);
                            });

                            // 下载
                            row.col(|ui| {
                                if t.dlspeed > 0 {
                                    w::num_cell(ui, p, &fmt_speed(t.dlspeed), p.dl);
                                } else {
                                    w::num_cell(ui, p, "-", p.faint);
                                }
                            });

                            // 上传
                            row.col(|ui| {
                                if t.upspeed > 0 {
                                    w::num_cell(ui, p, &fmt_speed(t.upspeed), p.up);
                                } else {
                                    w::num_cell(ui, p, "-", p.faint);
                                }
                            });

                            // 分享率
                            row.col(|ui| {
                                let r = t.ratio;
                                let col = if r >= 1.0 { p.up } else { p.muted };
                                w::num_cell(ui, p, &fmt_ratio(r), col);
                            });

                            // 剩余时间
                            row.col(|ui| {
                                if t.is_completed() || t.eta < 0 || t.eta >= 8640000 {
                                    w::num_cell(ui, p, "-", p.faint);
                                } else {
                                    w::num_cell(ui, p, &fmt_eta(t.eta), p.muted);
                                }
                            });

                            // 分类
                            row.col(|ui| {
                                if t.category.is_empty() {
                                    w::text_cell(ui, p, "-", p.faint, 12.0);
                                } else {
                                    w::text_cell(ui, p, t.category.as_str(), p.muted, 12.0);
                                }
                            });

                            // 标签
                            row.col(|ui| {
                                let tags = t.tag_list();
                                if tags.is_empty() {
                                    w::text_cell(ui, p, "-", p.faint, 12.0);
                                } else {
                                    w::tag_pills(ui, p, &tags);
                                }
                            });

                            // ---- 行背景回填 + 交互 ----
                            let resp = row.response();

                            let hov = ctx.animate_bool_with_time(
                                egui::Id::new(("qb-row-h", hash.as_str())),
                                resp.hovered(),
                                0.12,
                            );
                            if let Some(bg) = row_bg.borrow_mut().take() {
                                bg.paint(resp.rect, is_sel, hov, p);
                            }

                            // 单击语义：
                            //   点在勾选框上 / Ctrl(⌘) / Shift / 回车 → 切换这一行
                            //   其它                                  → 只选中这一行
                            // 判定统一在「行」上做，勾选框自己不持有交互 —— 单一交互源，不会抢点击。
                            if resp.double_clicked() {
                                actions.borrow_mut().push(Action::OpenDetail(hash.clone()));
                            } else if resp.clicked() {
                                let in_check = resp.interact_pointer_pos().is_some_and(|pt| {
                                    pt.x <= resp.rect.left() + CHECK_W
                                });
                                actions.borrow_mut().push(if in_check || ctrl || shift {
                                    Action::Toggle(hash.clone())
                                } else {
                                    Action::SelectOnly(hash.clone())
                                });
                            } else if resp.has_focus() && (enter_key || space_key) {
                                actions.borrow_mut().push(Action::Toggle(hash.clone()));
                            }

                            resp.context_menu(|ui| {
                                ui.label(RichText::new(t.name.as_str()).font(th::id_sb(12.5)));
                                ui.separator();
                                if ui.button("详情").clicked() {
                                    actions.borrow_mut().push(Action::OpenDetail(hash.clone()));
                                    ui.close_menu();
                                }
                                if ui.button("暂停").clicked() {
                                    actions.borrow_mut().push(Action::Pause(vec![hash.clone()]));
                                    ui.close_menu();
                                }
                                if ui.button("恢复").clicked() {
                                    actions.borrow_mut().push(Action::Resume(vec![hash.clone()]));
                                    ui.close_menu();
                                }
                                if ui.button("强制开始 开 / 关").clicked() {
                                    actions.borrow_mut().push(Action::ForceStart(
                                        vec![hash.clone()],
                                        !t.force_start,
                                    ));
                                    ui.close_menu();
                                }
                                if ui.button("顺序下载 开 / 关").clicked() {
                                    actions.borrow_mut().push(Action::Sequential(
                                        vec![hash.clone()],
                                        !t.seq_dl,
                                    ));
                                    ui.close_menu();
                                }

                                ui.separator();
                                // 分类 / 标签：右键菜单是「就地改这一个任务」最顺手的入口
                                let cur_cat = t.category.clone();
                                ui.menu_button(
                                    if cur_cat.is_empty() {
                                        "设为分类".to_string()
                                    } else {
                                        format!("设为分类（当前：{}）", cur_cat)
                                    },
                                    |ui| {
                                        if ui.button("（清空分类）").clicked() {
                                            actions.borrow_mut().push(Action::SetCategory(
                                                vec![hash.clone()],
                                                String::new(),
                                            ));
                                            ui.close_menu();
                                        }
                                        for c in cats {
                                            let on = cur_cat.as_str() == c.as_str();
                                            if ui.selectable_label(on, c.as_str()).clicked() {
                                                actions.borrow_mut().push(Action::SetCategory(
                                                    vec![hash.clone()],
                                                    c.clone(),
                                                ));
                                                ui.close_menu();
                                            }
                                        }
                                        if cats.is_empty() {
                                            ui.weak("（还没有分类）");
                                        }
                                    },
                                );

                                let cur_tags = t.tag_list();
                                ui.menu_button("标签", |ui| {
                                    if cur_tags.is_empty() {
                                        ui.weak("（该任务还没有标签）");
                                    } else {
                                        ui.label(RichText::new("点击移除：").font(th::id(11.0)));
                                        for g in &cur_tags {
                                            if ui.button(format!("− {}", g)).clicked() {
                                                actions.borrow_mut().push(Action::RemoveTags(
                                                    vec![hash.clone()],
                                                    (*g).to_string(),
                                                ));
                                                ui.close_menu();
                                            }
                                        }
                                    }
                                    ui.separator();
                                    ui.label(RichText::new("点击添加：").font(th::id(11.0)));
                                    let mut any = false;
                                    for g in all_tags {
                                        if cur_tags.contains(&g.as_str()) {
                                            continue;
                                        }
                                        any = true;
                                        if ui.button(format!("+ {}", g)).clicked() {
                                            actions.borrow_mut().push(Action::AddTags(
                                                vec![hash.clone()],
                                                g.clone(),
                                            ));
                                            ui.close_menu();
                                        }
                                    }
                                    if !any {
                                        ui.weak("（没有可添加的标签）");
                                    }
                                });

                                if ui.button("复制 hash").clicked() {
                                    ui.output_mut(|o| o.copied_text = hash.clone());
                                    actions.borrow_mut().push(Action::CopyHash(hash.clone()));
                                    ui.close_menu();
                                }
                                if ui.button("移动保存路径…").clicked() {
                                    actions.borrow_mut().push(Action::MoveTo(
                                        vec![hash.clone()],
                                        t.save_path.clone(),
                                    ));
                                    ui.close_menu();
                                }

                                ui.separator();
                                if ui.button("重新校验").clicked() {
                                    actions.borrow_mut().push(Action::Recheck(vec![hash.clone()]));
                                    ui.close_menu();
                                }
                                if ui.button("重新汇报 Tracker").clicked() {
                                    actions
                                        .borrow_mut()
                                        .push(Action::Reannounce(vec![hash.clone()]));
                                    ui.close_menu();
                                }
                                ui.separator();
                                if ui.button("删除…").clicked() {
                                    actions.borrow_mut().push(Action::Delete(vec![hash.clone()]));
                                    ui.close_menu();
                                }
                            });
                        });
                    });

                // ---- 写回 ----
                self.selected = selected.into_inner();
                if let Some((k, d)) = new_sort.get() {
                    self.sort_key = k;
                    self.sort_desc = d;
                }
                for a in actions.into_inner() {
                    self.apply(a);
                }
            });
    }


    fn apply(&mut self, a: Action) {
        match a {
            Action::Toggle(h) => {
                // `HashSet::remove` 会告诉你到底删掉没有 —— 顺手当切换用，省一次 contains
                if !self.selected.remove(&h) {
                    self.selected.insert(h);
                }
            }
            Action::SelectOnly(h) => {
                self.selected.clear();
                self.selected.insert(h);
            }
            Action::OpenDetail(h) => {
                self.detail_hash = Some(h.clone());
                self.show_detail = true;
                self.worker.send(Cmd::FetchDetail(h));
            }
            Action::Pause(h) => self.worker.send(Cmd::Pause(h)),
            Action::Resume(h) => self.worker.send(Cmd::Resume(h)),
            Action::Recheck(h) => self.worker.send(Cmd::Recheck(h)),
            Action::Reannounce(h) => self.worker.send(Cmd::Reannounce(h)),
            Action::ForceStart(h, v) => self.worker.send(Cmd::SetForceStart { hashes: h, value: v }),
            Action::Sequential(h, v) => self.worker.send(Cmd::SetSequential { hashes: h, value: v }),
            Action::SetCategory(h, c) => self
                .worker
                .send(Cmd::SetCategory { hashes: h, category: c }),
            Action::AddTags(h, t) => self.worker.send(Cmd::AddTags { hashes: h, tags: t }),
            Action::RemoveTags(h, t) => self.worker.send(Cmd::RemoveTags { hashes: h, tags: t }),
            Action::MoveTo(h, cur) => {
                self.selected = h.into_iter().collect();
                self.move_path = cur;
                self.show_move = true;
            }
            Action::Delete(h) => {
                self.selected = h.into_iter().collect();
                self.show_delete = true;
            }
            Action::CopyHash(h) => {
                // 真正的写剪贴板在右键菜单里做（那里才有 ui），这里只给个反馈
                self.toast_ok(format!("已复制 hash：{}", h));
            }
        }
    }

    // ---------------- 弹窗 ----------------

    fn windows(&mut self, ctx: &egui::Context, snap: &Snap) {
        self.win_settings(ctx);
        self.win_add(ctx, snap);
        self.win_delete(ctx, snap);
        self.win_limits(ctx, snap);
        self.win_log(ctx, snap);
        self.win_new_category(ctx);
        self.win_move(ctx, snap);
        self.win_detail(ctx, snap);
    }

    fn win_settings(&mut self, ctx: &egui::Context) {
        if !self.show_settings {
            return;
        }
        let p = self.p;
        let mut close = false;
        let mut do_save = false;
        let mut do_connect = false;

        let keep = w::dialog(
            ctx,
            p,
            w::DialogSpec {
                id: "dlg-settings",
                title: "连接设置",
                subtitle: Some("直连 qBittorrent 的 WebUI，配置存在本地 JSON 里"),
                size: [470.0, 0.0],
                resizable: false,
            },
            |ui| {
                ui.label(
                    RichText::new("服务器")
                        .font(th::id_sb(11.0))
                        .color(p.faint),
                );
                ui.add_space(2.0);
                egui::Grid::new("cfg_grid")
                    .num_columns(2)
                    .spacing([14.0, 9.0])
                    .show(ui, |ui| {
                        ui.label("主机 / IP");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.cfg.host).desired_width(240.0),
                        );
                        ui.end_row();

                        ui.label("端口");
                        ui.add(egui::DragValue::new(&mut self.cfg.port).range(1..=65535));
                        ui.end_row();

                        ui.label("用户名");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.cfg.username).desired_width(240.0),
                        );
                        ui.end_row();

                        ui.label("密码");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.cfg.password)
                                .password(true)
                                .desired_width(240.0),
                        );
                        ui.end_row();

                        ui.label("轮询间隔");
                        ui.horizontal(|ui| {
                            ui.add(egui::DragValue::new(&mut self.cfg.refresh_secs).range(1..=60));
                            ui.label("秒");
                        });
                        ui.end_row();
                    });

                ui.add_space(10.0);
                ui.label(
                    RichText::new("行为")
                        .font(th::id_sb(11.0))
                        .color(p.faint),
                );
                ui.add_space(2.0);
                ui.checkbox(&mut self.cfg.use_https, "使用 HTTPS");
                ui.checkbox(&mut self.cfg.ignore_cert, "忽略证书校验（自签名证书）");
                ui.checkbox(&mut self.cfg.auto_connect, "启动时自动连接");

                ui.add_space(10.0);
                ui.label(
                    RichText::new("显示")
                        .font(th::id_sb(11.0))
                        .color(p.faint),
                );
                ui.add_space(2.0);
                // 界面缩放。
                //
                // 和「系统 DPI 缩放」是两回事，别混：
                //   · 系统缩放（Windows 的 100%/150%/200%）由 eframe 读进
                //     `pixels_per_point`，我们不用管，改了系统设置 UI 自己跟着变；
                //   · 这里调的是 `zoom_factor`，**在系统缩放之上再乘一层**。
                // 存在的意义：4K 屏上把系统设成 150% 之后，字还是偏小、
                // 一屏能塞下的行数还是偏多，这时候用户需要一个自己的旋钮，
                // 而不是被迫去改整个系统的缩放（那会连带影响别的软件）。
                ui.horizontal(|ui| {
                    ui.label("界面缩放");
                    let z = ctx.zoom_factor();
                    if ghost_button(ui, p, "－", 0.0).clicked() {
                        ctx.set_zoom_factor((z - 0.05).max(0.7));
                    }
                    ui.label(
                        RichText::new(format!("{:.0}%", z * 100.0))
                            .font(th::id_num(12.0))
                            .color(p.text),
                    );
                    if ghost_button(ui, p, "＋", 0.0).clicked() {
                        ctx.set_zoom_factor((z + 0.05).min(2.0));
                    }
                    if ghost_button(ui, p, "重置", 0.0).clicked() {
                        ctx.set_zoom_factor(1.0);
                    }
                    ui.label(
                        RichText::new("Ctrl + / − / 0")
                            .font(th::id(10.5))
                            .color(p.faint),
                    );
                });

                ui.add_space(10.0);
                ui.label(
                    RichText::new(format!("配置文件：{}", Config::config_path().display()))
                        .font(th::id(10.5))
                        .color(p.faint),
                );
                ui.label(
                    RichText::new("注意：密码以明文保存在该文件中。")
                        .font(th::id(10.5))
                        .color(p.warn),
                );

                w::dialog_footer(ui, p, |ui| {
                    if primary_button(ui, p, "保存并连接").clicked() {
                        do_save = true;
                        do_connect = true;
                    }
                    if ghost_button(ui, p, "仅保存", 0.0).clicked() {
                        do_save = true;
                    }
                    if ghost_button(ui, p, "取消", 0.0).clicked() {
                        close = true;
                    }
                });
            },
        );

        if do_save {
            match self.cfg.save() {
                Ok(_) => {
                    self.toast_ok("配置已保存");
                    // 轮询间隔可能改了，通知网络线程立即生效。
                    // 这里只更新间隔、不重连 —— 用户改个数字不该把会话踢掉。
                    self.worker.send(Cmd::UpdateRefresh(self.cfg.refresh_secs));
                }
                Err(e) => self.toast_err(format!("保存配置失败：{}", e)),
            }
        }
        if do_connect {
            self.worker.send(Cmd::Connect(self.cfg.clone()));
        }
        self.show_settings = keep && !close;
    }

    fn win_add(&mut self, ctx: &egui::Context, snap: &Snap) {
        if !self.show_add {
            return;
        }
        let p = self.p;
        let mut close = false;
        let mut submit = false;
        let mut pick = false;

        let keep = w::dialog(
            ctx,
            p,
            w::DialogSpec {
                id: "dlg-add",
                title: "添加任务",
                subtitle: Some("magnet / .torrent URL / 本地种子文件"),
                size: [640.0, 0.0],
                resizable: false,
            },
            |ui| {
                ui.add(
                    egui::TextEdit::multiline(&mut self.add_urls)
                        .desired_rows(6)
                        .desired_width(f32::INFINITY)
                        .hint_text("每行一个：magnet:?xt=urn:btih:…  或  https://…/x.torrent"),
                );

                ui.add_space(6.0);
                if ghost_button(ui, p, "从本地选择 .torrent 文件并添加…", 0.0).clicked() {
                    pick = true;
                }

                ui.add_space(10.0);
                ui.label(RichText::new("选项").font(th::id_sb(11.0)).color(p.faint));
                ui.add_space(2.0);
                egui::Grid::new("add_grid")
                    .num_columns(2)
                    .spacing([14.0, 9.0])
                    .show(ui, |ui| {
                        ui.label("保存路径");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.add_savepath)
                                .desired_width(360.0)
                                .hint_text("留空 = 使用 qB 默认 / 分类目录"),
                        );
                        ui.end_row();

                        ui.label("分类");
                        let cat_text = if self.add_category.is_empty() {
                            "（无）".to_string()
                        } else {
                            self.add_category.clone()
                        };
                        egui::ComboBox::from_id_salt("add_cat")
                            .selected_text(cat_text)
                            .width(200.0)
                            .show_ui(ui, |ui| {
                                ui.selectable_value(
                                    &mut self.add_category,
                                    String::new(),
                                    "（无）",
                                );
                                for c in &snap.categories {
                                    ui.selectable_value(
                                        &mut self.add_category,
                                        c.clone(),
                                        c.as_str(),
                                    );
                                }
                            });
                        ui.end_row();

                        ui.label("标签");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.add_tags)
                                .desired_width(360.0)
                                .hint_text("逗号分隔，如：电影, 1080p"),
                        );
                        ui.end_row();
                    });

                ui.add_space(6.0);
                ui.checkbox(&mut self.add_paused, "添加后暂停");
                ui.checkbox(&mut self.add_sequential, "顺序下载");

                w::dialog_footer(ui, p, |ui| {
                    // 不置灰：置灰的按钮说不出"为什么不能点"。留可点，
                    // 空输入时由下面的校验给出明确提示（toast）。
                    if primary_button(ui, p, "提交").clicked() {
                        submit = true;
                    }
                    if ghost_button(ui, p, "取消", 0.0).clicked() {
                        close = true;
                    }
                });
            },
        );

        if submit {
            let urls = self.add_urls.clone();
            if urls.lines().any(|l| !l.trim().is_empty()) {
                self.worker.send(Cmd::AddUrls {
                    urls,
                    opts: self.add_opts(),
                });
                self.add_urls.clear();
                close = true;
            } else {
                self.toast_err("请先填入 magnet 链接或 URL");
            }
        }

        if pick {
            if let Some(path) = rfd::FileDialog::new()
                .add_filter("BitTorrent 种子", &["torrent"])
                .set_title("选择 .torrent 文件")
                .pick_file()
            {
                match std::fs::read(&path) {
                    Ok(data) => {
                        let name = path
                            .file_name()
                            .map(|s| s.to_string_lossy().to_string())
                            .unwrap_or_else(|| "upload.torrent".to_string());
                        self.worker.send(Cmd::AddFile {
                            name,
                            data,
                            opts: self.add_opts(),
                        });
                        close = true;
                    }
                    Err(e) => self.toast_err(format!("读取种子文件失败：{}", e)),
                }
            }
        }

        self.show_add = keep && !close;
    }

    fn win_delete(&mut self, ctx: &egui::Context, snap: &Snap) {
        if !self.show_delete {
            return;
        }
        let p = self.p;
        let hashes = self.targets();
        let n = hashes.len();
        let mut close = false;
        let mut confirm = false;

        // 名字直接取传入的快照，不必再锁一次共享状态
        let names: Vec<String> = hashes
            .iter()
            .filter_map(|h| {
                snap.torrents
                    .iter()
                    .find(|t| &t.hash == h)
                    .map(|t| t.name.clone())
            })
            .take(8)
            .collect();

        let keep = w::dialog(
            ctx,
            p,
            w::DialogSpec {
                id: "dlg-delete",
                title: "删除任务",
                subtitle: Some("qBittorrent 侧的删除不可撤销，请确认范围"),
                size: [480.0, 0.0],
                resizable: false,
            },
            |ui| {
                ui.label(
                    RichText::new(format!("即将删除 {} 个任务", n))
                        .font(th::id_sb(13.0))
                        .color(p.err),
                );
                ui.add_space(6.0);
                for nm in &names {
                    ui.label(RichText::new(format!("· {}", nm)).font(th::id(11.5)).color(p.muted));
                }
                if n > names.len() {
                    ui.label(
                        RichText::new(format!("… 以及另外 {} 个", n - names.len()))
                            .font(th::id(11.5))
                            .color(p.faint),
                    );
                }

                ui.add_space(10.0);
                ui.checkbox(&mut self.del_files, "同时删除已下载的数据文件（不可恢复）");

                w::dialog_footer(ui, p, |ui| {
                    let label = if self.del_files {
                        "删除任务与数据"
                    } else {
                        "仅删除任务"
                    };
                    if danger_button(ui, p, label).clicked() {
                        confirm = true;
                    }
                    if ghost_button(ui, p, "取消", 0.0).clicked() {
                        close = true;
                    }
                });
            },
        );

        if confirm {
            self.worker.send(Cmd::Delete {
                hashes: hashes.clone(),
                files: self.del_files,
            });
            self.selected.clear();
            close = true;
        }
        self.show_delete = keep && !close;
    }

    fn win_limits(&mut self, ctx: &egui::Context, snap: &Snap) {
        if !self.show_limits {
            return;
        }
        let hashes = self.targets();
        let p = self.p;
        let mut close = false;
        let mut apply_global = false;
        let mut apply_torrent = false;

        let keep = w::dialog(
            ctx,
            p,
            w::DialogSpec {
                id: "dlg-limits",
                title: "限速",
                subtitle: Some("单位 KiB/s；0 = 不限，-1 = 跟随全局"),
                size: [440.0, 0.0],
                resizable: false,
            },
            |ui| {
                // ---- 全局 ----
                ui.label(
                    RichText::new("全局限速")
                        .font(th::id_sb(11.0))
                        .color(p.faint),
                );
                egui::Grid::new("lim_grid")
                    .num_columns(2)
                    .spacing([14.0, 9.0])
                    .show(ui, |ui| {
                        ui.label("下载");
                        ui.add(egui::DragValue::new(&mut self.lim_dl).range(0..=10_485_760));
                        ui.end_row();
                        ui.label("上传");
                        ui.add(egui::DragValue::new(&mut self.lim_up).range(0..=10_485_760));
                        ui.end_row();
                    });
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if primary_button(ui, p, "应用").clicked() {
                        apply_global = true;
                    }
                    if ghost_button(ui, p, "解除", 0.0).clicked() {
                        self.lim_dl = 0;
                        self.lim_up = 0;
                        apply_global = true;
                    }
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new(format!(
                            "当前 ↓ {}   ↑ {}",
                            if snap.global_dl_limit <= 0 {
                                "不限".to_string()
                            } else {
                                fmt_speed(snap.global_dl_limit)
                            },
                            if snap.global_up_limit <= 0 {
                                "不限".to_string()
                            } else {
                                fmt_speed(snap.global_up_limit)
                            }
                        ))
                        .font(th::id_num(11.0))
                        .color(p.muted),
                    );
                });

                // ---- 单任务 ----
                ui.add_space(14.0);
                w::hairline(ui, p);
                ui.add_space(10.0);
                ui.label(
                    RichText::new(format!("选中任务的独立限速（{} 个）", hashes.len()))
                        .font(th::id_sb(11.0))
                        .color(p.faint),
                );
                egui::Grid::new("lim2_grid")
                    .num_columns(2)
                    .spacing([14.0, 9.0])
                    .show(ui, |ui| {
                        ui.label("下载");
                        ui.add(egui::DragValue::new(&mut self.lim_torrent_dl).range(-1..=10_485_760));
                        ui.end_row();
                        ui.label("上传");
                        ui.add(egui::DragValue::new(&mut self.lim_torrent_up).range(-1..=10_485_760));
                        ui.end_row();
                    });

                w::dialog_footer(ui, p, |ui| {
                    let en = !hashes.is_empty();
                    if en && primary_button(ui, p, "应用到选中任务").clicked() {
                        apply_torrent = true;
                    } else if !en {
                        ghost_button(ui, p, "应用到选中任务", 0.0);
                    }
                    if ghost_button(ui, p, "关闭", 0.0).clicked() {
                        close = true;
                    }
                });
            },
        );

        if apply_global {
            self.worker.send(Cmd::SetGlobalDlLimit(kib_to_bytes(self.lim_dl)));
            self.worker.send(Cmd::SetGlobalUpLimit(kib_to_bytes(self.lim_up)));
        }
        if apply_torrent {
            // 注意用 `_pm` 版本：单任务限速的 -1 是「跟随全局」，
            // 普通的 `kib_to_bytes` 会把它压成 0（= 不限），语义就错了。
            self.worker.send(Cmd::SetTorrentLimit {
                hashes: hashes.clone(),
                dl: Some(kib_to_bytes_pm(self.lim_torrent_dl)),
                up: Some(kib_to_bytes_pm(self.lim_torrent_up)),
            });
        }
        self.show_limits = keep && !close;
    }

    fn win_log(&mut self, ctx: &egui::Context, snap: &Snap) {
        if !self.show_log {
            return;
        }
        let p = self.p;
        let mut close = false;
        let mut refresh = false;

        let keep = w::dialog(
            ctx,
            p,
            w::DialogSpec {
                id: "dlg-log",
                title: "操作日志",
                subtitle: Some("只保留最近 150 条，重启即清空"),
                size: [720.0, 460.0],
                resizable: true,
            },
            |ui| {
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .stick_to_bottom(true)
                    .max_height(340.0)
                    .show(ui, |ui| {
                        if snap.log.is_empty() {
                            ui.label(
                                RichText::new("（暂无日志）")
                                    .font(th::id(12.0))
                                    .color(p.faint),
                            );
                        }
                        for line in snap.log.iter() {
                            ui.label(
                                RichText::new(line.as_str())
                                    .font(th::id_mono(11.5))
                                    .color(p.muted),
                            );
                        }
                    });

                w::dialog_footer(ui, p, |ui| {
                    if ghost_button(ui, p, "关闭", 0.0).clicked() {
                        close = true;
                    }
                    if ghost_button(ui, p, "刷新", 0.0).clicked() {
                        refresh = true;
                    }
                    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        ui.label(
                            RichText::new(format!("共 {} 条", snap.log.len()))
                                .font(th::id_num(11.0))
                                .color(p.faint),
                        );
                    });
                });
            },
        );

        if refresh {
            self.worker.send(Cmd::RefreshNow);
        }
        self.show_log = keep && !close;
    }

    fn win_new_category(&mut self, ctx: &egui::Context) {
        if !self.show_new_category {
            return;
        }
        let p = self.p;
        let mut close = false;
        let mut submit = false;

        let keep = w::dialog(
            ctx,
            p,
            w::DialogSpec {
                id: "dlg-newcat",
                title: "新建分类",
                subtitle: Some("分类可以绑定独立的保存目录"),
                size: [440.0, 0.0],
                resizable: false,
            },
            |ui| {
                egui::Grid::new("newcat_grid")
                    .num_columns(2)
                    .spacing([14.0, 9.0])
                    .show(ui, |ui| {
                        ui.label("名称");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.new_cat_name)
                                .desired_width(260.0),
                        );
                        ui.end_row();
                        ui.label("保存路径");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.new_cat_path)
                                .desired_width(260.0)
                                .hint_text("留空 = 使用默认下载目录"),
                        );
                        ui.end_row();
                    });

                w::dialog_footer(ui, p, |ui| {
                    let en = !self.new_cat_name.trim().is_empty();
                    if en && primary_button(ui, p, "创建").clicked() {
                        submit = true;
                    } else if !en {
                        ghost_button(ui, p, "创建", 0.0);
                    }
                    if ghost_button(ui, p, "取消", 0.0).clicked() {
                        close = true;
                    }
                });
            },
        );

        if submit {
            self.worker.send(Cmd::CreateCategory {
                name: self.new_cat_name.trim().to_string(),
                save_path: self.new_cat_path.trim().to_string(),
            });
            self.new_cat_name.clear();
            self.new_cat_path.clear();
            close = true;
        }
        self.show_new_category = keep && !close;
    }

    /// 移动保存路径。
    ///
    /// qB 会**真的把磁盘上的数据搬过去**（`setLocation`），所以这里必须写清楚
    /// 影响范围，并且目标路径非空才允许提交 —— 空路径 qB 会直接 400。
    fn win_move(&mut self, ctx: &egui::Context, snap: &Snap) {
        if !self.show_move {
            return;
        }
        let hashes = self.targets();
        let n = hashes.len();
        let p = self.p;
        let mut close = false;
        let mut confirm = false;

        // 单选时把当前路径摆出来，省得用户去别处抄
        let cur_path = if n == 1 {
            hashes
                .first()
                .and_then(|h| snap.torrents.iter().find(|t| &t.hash == h))
                .map(|t| t.save_path.clone())
        } else {
            None
        };

        let keep = w::dialog(
            ctx,
            p,
            w::DialogSpec {
                id: "dlg-move",
                title: "移动保存路径",
                subtitle: Some("qB 会把已下载的数据文件一并搬到新目录"),
                size: [560.0, 0.0],
                resizable: false,
            },
            |ui| {
                ui.label(
                    RichText::new(format!("将移动 {} 个任务。", n))
                        .font(th::id(12.0))
                        .color(p.muted),
                );
                if let Some(c) = &cur_path {
                    ui.label(
                        RichText::new(format!("当前：{}", c))
                            .font(th::id_mono(11.0))
                            .color(p.faint),
                    );
                }

                ui.add_space(8.0);
                ui.label("目标路径");
                ui.add_space(2.0);
                ui.add(
                    egui::TextEdit::singleline(&mut self.move_path)
                        .desired_width(f32::INFINITY)
                        .hint_text("/downloads/new  或  D:\\下载\\new"),
                );

                w::dialog_footer(ui, p, |ui| {
                    let en = n > 0 && !self.move_path.trim().is_empty();
                    if en && primary_button(ui, p, "移动").clicked() {
                        confirm = true;
                    } else if !en {
                        ghost_button(ui, p, "移动", 0.0);
                    }
                    if ghost_button(ui, p, "取消", 0.0).clicked() {
                        close = true;
                    }
                });
            },
        );

        if confirm {
            let path = self.move_path.trim().to_string();
            self.worker.send(Cmd::SetLocation {
                hashes: hashes.clone(),
                path,
            });
            self.move_path.clear();
            close = true;
        }
        self.show_move = keep && !close;
    }

    fn win_detail(&mut self, ctx: &egui::Context, snap: &Snap) {
        if !self.show_detail {
            return;
        }
        let hash = match &self.detail_hash {
            Some(h) => h.clone(),
            None => {
                self.show_detail = false;
                return;
            }
        };
        let Some(t) = snap.torrents.iter().find(|t| t.hash == hash) else {
            self.show_detail = false;
            return;
        };

        let p = self.p;
        let d = &snap.detail;
        let mine = d.is_for(&hash);

        // 打开（或换了任务）时自动拉一次。`loading` 挡着，不会连发；
        // 拉失败也只会停在错误态，由用户点「刷新」重试，不会无限重试。
        if !mine && !d.loading {
            self.worker.send(Cmd::FetchDetail(hash.clone()));
        }

        let mut close = false;
        let mut refresh = false;
        let mut tab = self.detail_tab;
        let counts = [
            None,
            Some(d.trackers.len()),
            Some(d.peers.len()),
            Some(d.webseeds.len()),
            Some(d.files.len()),
        ];

        let keep = w::dialog(
            ctx,
            p,
            w::DialogSpec {
                id: "dlg-detail",
                title: "任务详情",
                subtitle: Some(t.name.as_str()),
                size: [980.0, 640.0],
                resizable: true,
            },
            |ui| {
                let tabs: Vec<(&str, Option<usize>)> = DetailTab::ALL
                    .iter()
                    .zip(counts.iter())
                    .map(|(tb, c)| (tb.label(), *c))
                    .collect();
                if let Some(i) = w::tab_bar(ui, p, &tabs, tab.idx()) {
                    tab = DetailTab::ALL[i];
                }
                ui.add_space(8.0);

                if !mine && d.loading {
                    ui.add_space(48.0);
                    ui.vertical_centered(|ui| {
                        ui.spinner();
                        ui.add_space(8.0);
                        ui.label(
                            RichText::new("正在拉取任务详情…")
                                .font(th::id(12.0))
                                .color(p.faint),
                        );
                    });
                } else {
                    match tab {
                        DetailTab::General => detail_general(ui, p, t, d.props.as_ref()),
                        DetailTab::Trackers => detail_trackers(ui, p, &d.trackers),
                        DetailTab::Peers => detail_peers(ui, p, &d.peers),
                        DetailTab::HttpSources => detail_webseeds(ui, p, &d.webseeds),
                        DetailTab::Content => detail_files(ui, p, &d.files),
                    }
                }

                w::dialog_footer(ui, p, |ui| {
                    if ghost_button(ui, p, "关闭", 0.0).clicked() {
                        close = true;
                    }
                    if ghost_button(ui, p, "刷新", 0.0).clicked() {
                        refresh = true;
                    }
                    // 靠左：状态 / 错误 / 更新时间
                    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        if d.loading {
                            ui.spinner();
                            ui.label(
                                RichText::new("拉取中…").font(th::id(11.0)).color(p.faint),
                            );
                        } else if let Some(e) = &d.error {
                            ui.label(
                                RichText::new(w::fit(e, 460.0, 11.0))
                                    .font(th::id(11.0))
                                    .color(p.err),
                            );
                        } else if let Some(at) = d.at {
                            let s = at.elapsed().as_secs();
                            let txt = if s < 3 {
                                "刚刚更新".to_string()
                            } else {
                                format!("{} 秒前更新", s)
                            };
                            ui.label(
                                RichText::new(txt)
                                    .font(th::id_num(10.5))
                                    .color(p.faint),
                            );
                        }
                    });
                });
            },
        );

        if refresh {
            self.worker.send(Cmd::FetchDetail(hash.clone()));
        }
        self.detail_tab = tab;
        self.show_detail = keep && !close;
    }

    /// 把应用主题同步给 Windows 原生死标题栏。
    ///
    /// 不做这件事的话，浅色主题下窗口最上面那条还是深的 —— 系统在"深色模式"时
    /// Windows 默认就给深色标题栏，它不看应用内部的配色。这是改版前最扎眼的
    /// 一处不一致：整个窗口的第一像素就和内容对不上。
    ///
    /// ⚠️ `raw-window-handle` 只挂在 `[target.'cfg(windows)'.dependencies]` 下，
    /// 而且这套逻辑只在 Win32 句柄上才有意义 —— 所以整段必须按平台门住，
    /// 否则 macOS / Linux 直接编不过（找不到 crate）。
    #[cfg(windows)]
    fn sync_titlebar(&self, frame: &eframe::Frame) {
        use raw_window_handle::{HasWindowHandle, RawWindowHandle};

        let Ok(handle) = frame.window_handle() else {
            return;
        };
        if let RawWindowHandle::Win32(w) = handle.as_raw() {
            crate::titlebar::apply(w.hwnd.get(), self.dark);
        }
    }

    /// 非 Windows：没有"原生死标题栏配色"这件问题，空实现即可。
    #[cfg(not(windows))]
    fn sync_titlebar(&self, _frame: &eframe::Frame) {}
}

impl eframe::App for QbApp {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        ctx.request_repaint_after(Duration::from_millis(250));

        // 主题切换：只换调色板 + 重灌样式，布局代码一行不动
        if self.theme_dirty {
            self.p = th::pal(self.dark);
            th::install(ctx, self.dark);
            self.theme_dirty = false;
            self.sync_titlebar(frame);
        }
        // 首帧补一次：窗口句柄要等窗口建好才拿得到
        if !self.titlebar_done {
            self.sync_titlebar(frame);
            self.titlebar_done = true;
        }

        // ---- 显示器自适应 ----
        //
        // egui 本身已经把「缩放」这件事做完了：所有坐标都是**点**，
        // eframe 从 winit 拿到当前显示器的 scale factor 塞进
        // `pixels_per_point`，渲染时统一乘上去；字体图集也跟着 ppp
        // 重新烘焙，所以在 100% / 125% / 150% / 200% 下 UI 比例一致、
        // 文字不糊。这一段补的是它**不管**的那半：
        //
        //   `Prefs` 存的是点，而同一份"点"在不同显示器上占的物理面积
        //   天差地别 —— 150% 屏上存下的 1440×900 搬到 100% 的 1366×768
        //   笔记本上，1440pt 比屏幕还宽，`with_min_inner_size(1040×640)`
        //   更是直接满足不了，Windows 会把窗口强行拉成超过屏幕的尺寸，
        //   于是标题栏和 × 一起被推出屏幕外，关都关不掉。
        //
        // ⚠️ 单位有个坑，别照文档写。
        // `ViewportInfo::monitor_size` 的注释是 "Current monitor size in
        // egui points"，但它由 egui-winit 用 `monitor.size().to_logical(ppp)`
        // 算出来 —— 而窗口刚建好那几帧 `egui_ctx` 的 ppp 还是 1.0，算出来
        // 就是**物理像素**。实测首帧拿到 2560，可这台机器的逻辑宽只有
        // 1706.67，直接照用会把可用区放大 1.5 倍，钳制等于没做（窗口反而
        // 比屏幕还宽）。
        //
        // 归一办法：同一帧内 `monitor_size * ppp` 恒等于物理尺寸（因为
        // `monitor_size = 物理 / 那一帧的 ppp`），再按原生缩放折算回点。
        // 正常帧 ppp ≥ 原生缩放，系数为 1，原样使用；只有上面那种异常帧
        // 才需要缩一下 —— 所以取 `(ppp / 原生).min(1.0)`。
        let mon = ctx.input(|i| i.viewport().monitor_size);
        if let Some(mon) = mon {
            // `ViewportInfo::monitor_size` 就是 egui 点（实测：4K 屏 @150%，
            // 物理宽 3840，egui 给的是 2560 = 3840 / 1.5，与文档一致）。
            let avail = mon * WIN_FIT;

            if self
                .last_avail
                .map_or(true, |a| (a - avail).length() > 1.0)
            {
                self.last_avail = Some(avail);
                // 下限跟着显示器缩：小屏上不能出现「最小尺寸比屏幕还大」
                // 这种自相矛盾的约束，否则比不管还糟。
                let lo = clamp_size(WIN_MIN, egui::Vec2::ZERO, avail);
                ctx.send_viewport_cmd(egui::ViewportCommand::MinInnerSize(lo));
                if !self.size_applied {
                    // 首帧：把历史尺寸（或默认尺寸）钳进来
                    ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(clamp_size(
                        self.screen_size,
                        lo,
                        avail,
                    )));
                    self.size_applied = true;
                }
            }

            // 每帧兜一道：窗口只要超出显示器就收回来。
            // 判据用**整个显示器**（不是 0.94 的可用区）—— 否则窗口最大化
            // 时会被这段逻辑当成"越界"再拽小，那才是真的烦人。
            let cur = ctx.screen_rect().size();
            if cur.x > mon.x + 0.5 || cur.y > mon.y + 0.5 {
                let lo = clamp_size(WIN_MIN, egui::Vec2::ZERO, avail);
                ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(clamp_size(
                    cur, lo, avail,
                )));
            }
        }

        // 记下当前尺寸，退出时写进偏好（`save` 里拿不到 ctx）
        self.screen_size = ctx.screen_rect().size();

        let p = self.p;
        let snap = self.snapshot();

        // 氛围层走 Order::Background，必须最先画（中央面板是透明的，网格从底下透出来）
        w::backdrop(ctx, p);

        self.top_bar(ctx, &snap);
        self.side_panel(ctx, &snap);
        self.bottom_bar(ctx, &snap);
        self.table(ctx, &snap);
        self.windows(ctx, &snap);
    }

    /// eframe 在退出（以及周期性）时调用，把界面偏好落盘。
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(
            storage,
            PREF_KEY,
            &Prefs {
                dark: self.dark,
                width: self.screen_size.x,
                height: self.screen_size.y,
            },
        );
    }
}

// ---------------- 辅助 ----------------

/// 把一个尺寸逐轴夹进 `[lo, hi]`。`egui::Vec2` 没有 `clamp`。
///
/// 调用方必须保证 `lo <= hi`（`Vec2::clamp` 在 `min > max` 时会 panic，
/// 这里同理）—— 本文件里的用法都先取过 `WIN_MIN` 和可用区的较小者。
fn clamp_size(v: egui::Vec2, lo: egui::Vec2, hi: egui::Vec2) -> egui::Vec2 {
    egui::vec2(v.x.clamp(lo.x, hi.x), v.y.clamp(lo.y, hi.y))
}

fn state_color(p: &Palette, g: StateGroup) -> Color32 {
    match g {
        StateGroup::Downloading => p.dl,
        StateGroup::Seeding => p.up,
        StateGroup::Stopped => p.faint,
        StateGroup::Error => p.err,
        StateGroup::Checking => p.warn,
        StateGroup::Other => p.muted,
    }
}

/// B/s → (数值, 单位)。遥测带要把两者分开排版：数值大号等宽、单位小号灰色。
fn split_speed(bps: i64) -> (String, String) {
    let s = fmt_speed(bps);
    if let Some((v, u)) = s.split_once(' ') {
        return (v.to_string(), u.to_string());
    }
    (s, String::new())
}

/// 读数条比例。设了限速就按"限速占用率"，没设就按软量程折算 —— 
/// 这样进度条永远是有意义的读数，而不是一根常量。
fn rate_frac(cur: i64, limit: i64, soft: f32) -> f32 {
    let cur = cur.max(0) as f32;
    if limit > 0 {
        (cur / limit as f32).clamp(0.0, 1.0)
    } else {
        (cur / soft).clamp(0.0, 1.0)
    }
}

/// 条目是否被键盘激活（Enter / Space）。
/// 自绘控件不保证框架会合成键盘点击，所以显式判定一次。
fn kb_activated(ui: &egui::Ui, r: &egui::Response) -> bool {
    r.has_focus() && ui.input(|i| i.key_pressed(Key::Enter) || i.key_pressed(Key::Space))
}

/// 状态栏里的"小标签 + 等宽读数"。右到左布局中先放读数，读起来才是「标签 值」。
fn stat_rtl(ui: &mut egui::Ui, p: &Palette, label: &str, value: &str) {
    let _ = ui.label(
        RichText::new(value)
            .font(th::id_num(10.5))
            .color(p.muted),
    );
    let _ = ui.label(
        RichText::new(w::spaced(label))
            .font(th::id_sb(9.0))
            .color(p.faint),
    );
}

/// 主操作按钮：实心强调色底 + 反色文字。
///
/// 用内置 `Button` 而不是自绘，为的是保住键盘聚焦、焦点圈、禁用态这些原生行为；
/// hover / 按下态靠临时改写这个 scope 的弱背景色实现 ——
/// `Button::fill()` 会把四态一起锁死，所以这里不用它。
fn primary_button(ui: &mut egui::Ui, p: &Palette, text: &str) -> egui::Response {
    let hover = w::lerp(p.accent, p.on_accent, 0.10);
    let press = w::lerp(p.accent, p.on_accent, 0.24);
    ui.scope(|ui| {
        let v = &mut ui.visuals_mut().widgets;
        v.inactive.weak_bg_fill = p.accent;
        v.inactive.bg_fill = p.accent;
        v.inactive.bg_stroke = Stroke::NONE;
        v.hovered.weak_bg_fill = hover;
        v.hovered.bg_fill = hover;
        v.hovered.bg_stroke = Stroke::NONE;
        v.active.weak_bg_fill = press;
        v.active.bg_fill = press;
        v.active.bg_stroke = Stroke::NONE;
        ui.add(
            egui::Button::new(
                RichText::new(text)
                    .font(th::id_sb(12.5))
                    .color(p.on_accent),
            )
            .min_size(vec2(0.0, 24.0))
            .rounding(egui::Rounding::same(th::R_SM)),
        )
    })
    .inner
}

/// 三级按钮："幽灵"按钮 —— 无底无框，只有文字；悬停才浮出一层底色。
///
/// 改版前工具栏上所有按钮长得一模一样（白底 + 细框），于是「添加任务」这个主操作
/// 混在一堆同等重量的按钮里，扫视时找不到落点。分层之后：
///   一级 = 实心琥珀（添加任务）
///   二级 = 白底细框（暂停 / 恢复 / 删除 / 限速 / 连接设置）
///   三级 = 幽灵（日志 / 主题 / 清除 / 全选 …）
/// 信息层级是靠**重量差**建立的，不是靠加装饰 —— 所以三级按钮要真的"轻"。
fn ghost_button(ui: &mut egui::Ui, p: &Palette, text: &str, min_w: f32) -> egui::Response {
    ui.scope(|ui| {
        let v = &mut ui.visuals_mut().widgets;
        v.inactive.weak_bg_fill = Color32::TRANSPARENT;
        v.inactive.bg_fill = Color32::TRANSPARENT;
        v.inactive.bg_stroke = Stroke::NONE;
        v.inactive.fg_stroke = Stroke::new(1.0_f32, p.muted);
        v.hovered.weak_bg_fill = p.hover;
        v.hovered.bg_fill = p.hover;
        v.hovered.bg_stroke = Stroke::NONE;
        v.hovered.fg_stroke = Stroke::new(1.0_f32, p.text);
        v.active.weak_bg_fill = p.accent_soft;
        v.active.bg_fill = p.accent_soft;
        v.active.bg_stroke = Stroke::NONE;
        v.active.fg_stroke = Stroke::new(1.0_f32, p.text);
        ui.add(
            egui::Button::new(RichText::new(text).font(th::id(12.5)))
                .min_size(vec2(min_w, 24.0))
                .rounding(egui::Rounding::same(th::R_SM)),
        )
    })
    .inner
}

/// 开关按钮：承载**持久开关状态**的控件（如备用限速）。
///
/// 改版前它是个 `selectable_label`："关"时完全没有边框、和旁边的普通文字一样，
/// "开"时只多一层淡底 —— 于是既看不出这是个能点的控件，也分不清当前在哪一档。
/// 开关必须同时表达"可以点"和"现在是哪一档"，所以两档都带边框，靠色彩区分。
fn toggle_button(
    ui: &mut egui::Ui,
    p: &Palette,
    text: &str,
    on: bool,
    min_w: f32,
) -> egui::Response {
    ui.scope(|ui| {
        let v = &mut ui.visuals_mut().widgets;
        let (fill, stroke, fg) = if on {
            (p.accent_soft, p.accent, p.accent)
        } else {
            (Color32::TRANSPARENT, p.line, p.muted)
        };
        v.inactive.weak_bg_fill = fill;
        v.inactive.bg_fill = fill;
        v.inactive.bg_stroke = Stroke::new(1.0_f32, stroke);
        v.inactive.fg_stroke = Stroke::new(1.0_f32, fg);

        // 悬停时不要把状态色冲掉：开的还是开的样子，只是更实一点
        v.hovered.weak_bg_fill = if on {
            w::lerp(p.accent_soft, p.accent, 0.20)
        } else {
            p.hover
        };
        v.hovered.bg_fill = v.hovered.weak_bg_fill;
        v.hovered.bg_stroke = Stroke::new(1.0_f32, if on { p.accent } else { p.line_hi });
        v.hovered.fg_stroke = Stroke::new(1.0_f32, if on { p.accent } else { p.text });

        v.active.weak_bg_fill = if on {
            w::lerp(p.accent_soft, p.accent, 0.32)
        } else {
            p.hover
        };
        v.active.bg_fill = v.active.weak_bg_fill;
        v.active.bg_stroke = Stroke::new(1.0_f32, p.accent);
        v.active.fg_stroke = Stroke::new(1.0_f32, if on { p.accent } else { p.text });

        ui.add(
            egui::Button::new(RichText::new(text).font(th::id(12.5)))
                .min_size(vec2(min_w, 24.0))
                .rounding(egui::Rounding::same(th::R_SM)),
        )
    })
    .inner
}

/// 破坏性操作按钮：实心错误色底 + 白字。和主操作按钮同一套路数。
fn danger_button(ui: &mut egui::Ui, p: &Palette, text: &str) -> egui::Response {
    let hover = w::lerp(p.err, Color32::BLACK, 0.16);
    let press = w::lerp(p.err, Color32::BLACK, 0.30);
    ui.scope(|ui| {
        let v = &mut ui.visuals_mut().widgets;
        v.inactive.weak_bg_fill = p.err;
        v.inactive.bg_fill = p.err;
        v.inactive.bg_stroke = Stroke::NONE;
        v.hovered.weak_bg_fill = hover;
        v.hovered.bg_fill = hover;
        v.hovered.bg_stroke = Stroke::NONE;
        v.active.weak_bg_fill = press;
        v.active.bg_fill = press;
        v.active.bg_stroke = Stroke::NONE;
        ui.add(
            egui::Button::new(
                RichText::new(text)
                    .font(th::id_sb(12.5))
                    .color(Color32::WHITE),
            )
            .min_size(vec2(0.0, 24.0))
            .rounding(egui::Rounding::same(th::R_SM)),
        )
    })
    .inner
}

/// 表头单元格底色 + 底部分隔线。
/// 横向单元格是首尾相接的，每个表头格里画一遍就能拼出整行 —— 不需要知道表格总宽。
fn header_cell_bg(ui: &mut egui::Ui, p: &Palette) {
    let r = ui.max_rect();
    let painter = ui.painter();
    painter.rect_filled(r, 0.0, p.surface);
    painter.rect_filled(
        egui::Rect::from_min_max(
            pos2(r.left(), r.bottom() - 1.0),
            pos2(r.right(), r.bottom()),
        ),
        0.0,
        p.line_hi,
    );
}

/// 可排序列头：整格可点（点击热区大）、悬停变亮、当前排序列用强调色 + 三角标记。
/// 排序标记是现画的三角，不用 `▾ ▴` 字形 —— 那些字符在部分字体里缺字。
fn sortable_header(
    header: &mut egui_extras::TableRow<'_, '_>,
    label: &str,
    key: SortKey,
    cur_key: SortKey,
    cur_desc: bool,
    out: &Cell<Option<(SortKey, bool)>>,
    p: &Palette,
) {
    header.col(|ui| {
        header_cell_bg(ui, p);

        let active = cur_key == key;
        let font = th::id_sb(10.5);
        let galley = ui
            .painter()
            .layout_no_wrap(w::spaced(label), font, Color32::PLACEHOLDER);
        let gsize = galley.size();
        let tw = gsize.x;

        let w_avail = ui.available_width();
        let (rect, resp) =
            ui.allocate_exact_size(vec2(w_avail.max(20.0), 18.0), Sense::click());

        let k = ui.ctx().animate_bool_with_time(
            egui::Id::new(("qb-hd", label)),
            resp.hovered() || active,
            0.12,
        );
        let col = if active {
            p.accent
        } else {
            w::lerp(p.faint, p.muted, k)
        };

        let painter = ui.painter();
        painter.galley(
            pos2(rect.left(), rect.center().y - gsize.y * 0.5),
            galley,
            col,
        );
        if active {
            // 升序 → 箭头朝上；降序 → 朝下
            w::arrow(
                painter,
                pos2(rect.left() + tw + 7.0, rect.center().y),
                7.0,
                !cur_desc,
                col,
            );
        }
        if resp.has_focus() {
            painter.rect_stroke(
                rect,
                egui::Rounding::same(th::R_SM),
                Stroke::new(2.0_f32, p.accent),
            );
        }

        if resp.clicked() || kb_activated(ui, &resp) {
            // 同列再点 → 反向；换列 → 默认降序（新任务通常在最前）
            let desc = if cur_key == key { !cur_desc } else { true };
            out.set(Some((key, desc)));
        }
    });
}


fn conn_status_cn(s: &str) -> String {
    match s {
        "connected" => "连接正常".to_string(),
        "firewalled" => "受防火墙限制".to_string(),
        "disconnected" => "未连接".to_string(),
        "" => "-".to_string(),
        other => other.to_string(),
    }
}

// ---------------- 任务详情：五个模块 ----------------
//
// 版面按 qBittorrent 原生 WebUI 的「Torrent details」面板对齐：
// 列的顺序、含义、单位都照搬，只是换成本套设计语言的画法。
// 五个 Tab 共用同一份数据包（一次拉齐），切 Tab 不再发请求。

/// 常规页的标签列宽度。够放 7 个中文字，再长就换行。
const KV_LABEL_W: f32 = 88.0;

/// 表格专用：抹掉 `egui_extras` 在每条列边界画的**常态分隔线**。
///
/// `TableBuilder::resizable(true)` 会让库沿每根列边界画一条
/// `visuals.widgets.noninteractive.bg_stroke` 的线（见 egui_extras 0.29
/// `table.rs` 里 `ui.painter().line_segment([p0, p1], stroke)`）。
/// 那条线的本意是「这里可以拖」，但它常态下也一直画着 —— 多列数据表里
/// 就变成一排无来由的竖线，跟 qB 原生 WebUI 的表格式样也对不上。
///
/// 设成 `Stroke::NONE` 之后：常态干净；鼠标移近边界时库自动改用
/// `widgets.hovered.bg_stroke`、按住拖动时改用 `widgets.active.bg_stroke`，
/// 所以「可以拖列宽」这个提示一点没丢，只是不再常驻。
fn table_borderless(ui: &mut egui::Ui) {
    ui.visuals_mut().widgets.noninteractive.bg_stroke = egui::Stroke::NONE;
}

/// 表格表头格（左对齐小字）
fn thead(row: &mut egui_extras::TableRow<'_, '_>, p: &Palette, label: &str) {
    row.col(|ui| {
        header_cell_bg(ui, p);
        let _ = ui.label(
            RichText::new(w::spaced(label))
                .font(th::id_sb(10.0))
                .color(p.faint),
        );
    });
}

/// 表格表头格（右对齐 —— 数值列的表头也要右对齐，否则和数字对不上）
fn thead_r(row: &mut egui_extras::TableRow<'_, '_>, p: &Palette, label: &str) {
    row.col(|ui| {
        header_cell_bg(ui, p);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let _ = ui.label(
                RichText::new(w::spaced(label))
                    .font(th::id_sb(10.0))
                    .color(p.faint),
            );
        });
    });
}

fn dash_if_empty(s: &str) -> String {
    if s.trim().is_empty() {
        "-".to_string()
    } else {
        s.to_string()
    }
}

/// 单任务限速的三态：`-1` 跟随全局、`0` 不限、`>0` 具体值。
fn limit_cn(v: i64) -> String {
    if v < 0 {
        "跟随全局".to_string()
    } else if v == 0 {
        "不限".to_string()
    } else {
        fmt_speed(v)
    }
}

/// 常规页的一行：左标签定宽、右值可换行。
fn kv_line(ui: &mut egui::Ui, p: &Palette, label: &str, value: &str, mono: bool, value_w: f32) {
    ui.horizontal_top(|ui| {
        ui.allocate_ui_with_layout(
            vec2(KV_LABEL_W, 18.0),
            egui::Layout::top_down(egui::Align::LEFT),
            |ui| {
                ui.add_space(2.0);
                let _ = ui.label(
                    RichText::new(label)
                        .font(th::id(11.0))
                        .color(p.faint),
                );
            },
        );
        ui.allocate_ui_with_layout(
            vec2(value_w.max(80.0), 0.0),
            egui::Layout::top_down(egui::Align::LEFT),
            |ui| {
                let rt = if mono {
                    RichText::new(value).font(th::id_mono(11.5)).color(p.text)
                } else {
                    RichText::new(value).font(th::id(12.0)).color(p.text)
                };
                // 值一律允许换行：保存路径、注释、Tracker 地址都可能很长，
                // 截断会让"详情"这个功能本身失去意义。
                ui.add(egui::Label::new(rt).wrap());
            },
        );
    });
    ui.add_space(4.0);
}

fn kv_block(ui: &mut egui::Ui, p: &Palette, items: &[(&'static str, String, bool)], value_w: f32) {
    for (k, v, mono) in items {
        kv_line(ui, p, k, v, *mono, value_w);
    }
}

/// 取 properties 里的字段；没拿到 properties 时统一显示 `-`。
macro_rules! pp {
    ($pr:expr, $f:ident, $fmt:expr) => {
        match $pr {
            Some(x) => $fmt(x),
            None => "-".to_string(),
        }
    };
}

/// **常规** —— 字段与顺序对齐 WebUI 的 General 页。
fn detail_general(ui: &mut egui::Ui, p: &Palette, t: &Torrent, pr: Option<&TorrentProperties>) {
    let save_path = match pr {
        Some(x) if !x.save_path.is_empty() => x.save_path.clone(),
        _ => t.save_path.clone(),
    };
    let pieces = pp!(pr, pieces, |x: &TorrentProperties| format!(
        "{} × {}（已有 {}）",
        x.pieces_num,
        fmt_size(x.piece_size),
        x.pieces_have
    ));
    let conn = pp!(pr, nb_connections, |x: &TorrentProperties| format!(
        "{} / {}",
        x.nb_connections,
        if x.nb_connections_limit < 0 {
            "不限".to_string()
        } else {
            x.nb_connections_limit.to_string()
        }
    ));
    let peers_txt = pp!(pr, peers, |x: &TorrentProperties| format!(
        "已连 {} / 共 {}",
        x.peers, x.peers_total
    ));
    let seeds_txt = pp!(pr, seeds, |x: &TorrentProperties| format!(
        "已连 {} / 共 {}",
        x.seeds, x.seeds_total
    ));
    let dl_time = pp!(pr, time_elapsed, |x: &TorrentProperties| fmt_duration(
        (x.time_elapsed - x.seeding_time).max(0)
    ));

    let items: Vec<(&'static str, String, bool)> = vec![
        ("保存路径", save_path, false),
        ("分片", pieces, false),
        ("创建者", pp!(pr, created_by, |x: &TorrentProperties| dash_if_empty(&x.created_by)), false),
        ("创建于", pp!(pr, creation_date, |x: &TorrentProperties| fmt_ts(x.creation_date)), false),
        ("添加于", pp!(pr, addition_date, |x: &TorrentProperties| fmt_ts(x.addition_date)), false),
        ("完成于", pp!(pr, completion_date, |x: &TorrentProperties| fmt_ts(x.completion_date)), false),
        ("最近看完整", pp!(pr, last_seen, |x: &TorrentProperties| fmt_ts(x.last_seen)), false),
        ("最近活动", fmt_ts(t.last_activity), false),
        ("总大小", pp!(pr, total_size, |x: &TorrentProperties| fmt_size(x.total_size)), false),
        ("已下载", pp!(pr, total_downloaded, |x: &TorrentProperties| fmt_size(x.total_downloaded)), false),
        ("已上传", pp!(pr, total_uploaded, |x: &TorrentProperties| fmt_size(x.total_uploaded)), false),
        ("浪费", pp!(pr, total_wasted, |x: &TorrentProperties| fmt_size(x.total_wasted)), false),
        ("活动时间", pp!(pr, time_elapsed, |x: &TorrentProperties| fmt_duration(x.time_elapsed)), false),
        ("下载耗时", dl_time, false),
        ("做种时间", pp!(pr, seeding_time, |x: &TorrentProperties| fmt_duration(x.seeding_time)), false),
        ("连接数", conn, false),
        ("节点", peers_txt, false),
        ("做种者", seeds_txt, false),
        ("下载限速", pp!(pr, dl_limit, |x: &TorrentProperties| limit_cn(x.dl_limit)), false),
        ("上传限速", pp!(pr, up_limit, |x: &TorrentProperties| limit_cn(x.up_limit)), false),
        ("剩余时间", pp!(pr, eta, |x: &TorrentProperties| fmt_eta(x.eta)), false),
        ("下载速度", pp!(pr, dl_speed, |x: &TorrentProperties| fmt_speed(x.dl_speed)), false),
        ("上传速度", pp!(pr, up_speed, |x: &TorrentProperties| fmt_speed(x.up_speed)), false),
        ("平均下载", pp!(pr, dl_speed_avg, |x: &TorrentProperties| fmt_speed(x.dl_speed_avg)), false),
        ("平均上传", pp!(pr, up_speed_avg, |x: &TorrentProperties| fmt_speed(x.up_speed_avg)), false),
        ("分享率", pp!(pr, share_ratio, |x: &TorrentProperties| fmt_ratio(x.share_ratio)), false),
        ("热度", pp!(pr, popularity, |x: &TorrentProperties| format!("{:.2}", x.popularity)), false),
        ("私有种子", pp!(pr, is_private, |x: &TorrentProperties| if x.is_private { "是".to_string() } else { "否".to_string() }), false),
        ("信息哈希 v1", pp!(pr, infohash_v1, |x: &TorrentProperties| dash_if_empty(&x.infohash_v1)), true),
        ("信息哈希 v2", pp!(pr, infohash_v2, |x: &TorrentProperties| dash_if_empty(&x.infohash_v2)), true),
        ("Tracker", dash_if_empty(&t.tracker), false),
        ("内容路径", dash_if_empty(&t.content_path), false),
        ("分类", dash_if_empty(&t.category), false),
        ("标签", dash_if_empty(&t.tags), false),
        ("注释", pp!(pr, comment, |x: &TorrentProperties| dash_if_empty(&x.comment)), false),
    ];

    egui::ScrollArea::vertical()
        .id_salt("d_general")
        .auto_shrink([false, false])
        .max_height(420.0)
        .show(ui, |ui| {
            // 宽窗两栏（读起来少滚动），窄窗自动降成单栏
            if ui.available_width() >= 660.0 {
                let half = items.len().div_ceil(2);
                let (left, right) = items.split_at(half);
                ui.columns(2, |cols| {
                    let c0 = cols[0].available_width() - KV_LABEL_W - 10.0;
                    let c1 = cols[1].available_width() - KV_LABEL_W - 10.0;
                    kv_block(&mut cols[0], p, left, c0);
                    kv_block(&mut cols[1], p, right, c1);
                });
            } else {
                let cw = ui.available_width() - KV_LABEL_W - 10.0;
                kv_block(ui, p, &items, cw);
            }
        });
}

fn tracker_color(p: &Palette, status: i32) -> Color32 {
    match status {
        2 => p.up,
        3 => p.warn,
        4 => p.err,
        _ => p.faint,
    }
}

/// **Trackers** —— 列对齐 WebUI：URL / Status / Peers / Seeds / Downloaded / Message
fn detail_trackers(ui: &mut egui::Ui, p: &Palette, list: &[Tracker]) {
    if list.is_empty() {
        empty_hint(ui, p, "该任务没有 Tracker 数据");
        return;
    }
    table_borderless(ui);
    TableBuilder::new(ui)
        .id_salt("d_trackers")
        .striped(false)
        .resizable(true)
        .sense(Sense::hover())
        .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
        .auto_shrink([false, false])
        .min_scrolled_height(120.0)
        .max_scroll_height(420.0)
        .column(Column::remainder().at_least(210.0).clip(true))
        .column(Column::initial(80.0).at_least(66.0))
        .column(Column::initial(60.0).at_least(50.0))
        .column(Column::initial(60.0).at_least(50.0))
        .column(Column::initial(92.0).at_least(74.0))
        .column(Column::initial(220.0).at_least(90.0).clip(true))
        .header(th::HEADER_H, |mut h| {
            thead(&mut h, p, "URL");
            thead(&mut h, p, "状态");
            thead_r(&mut h, p, "节点");
            thead_r(&mut h, p, "做种");
            thead_r(&mut h, p, "已下载");
            thead(&mut h, p, "消息");
        })
        .body(|body| {
            body.rows(24.0, list.len(), |mut row| {
                let tr = &list[row.index()];
                let pseudo = tr.url.starts_with('*');
                row.col(|ui| {
                    w::text_cell(ui, p, &tr.url, if pseudo { p.muted } else { p.text }, 11.5);
                });
                row.col(|ui| {
                    w::text_cell(ui, p, tr.status_cn(), tracker_color(p, tr.status), 11.5);
                });
                row.col(|ui| w::num_cell(ui, p, &tr.num_peers.to_string(), p.muted));
                row.col(|ui| w::num_cell(ui, p, &tr.num_seeds.to_string(), p.muted));
                row.col(|ui| {
                    w::num_cell(ui, p, &tr.num_downloaded.to_string(), p.muted);
                });
                row.col(|ui| {
                    let msg = if tr.msg.is_empty() { "-" } else { tr.msg.as_str() };
                    let c = if tr.msg.is_empty() { p.faint } else { p.muted };
                    w::text_cell(ui, p, msg, c, 11.0);
                });
            });
        });
}

/// **节点** —— 列对齐 WebUI：
/// IP / Port / Connection / Flags / Client / Progress / DL / UP / Downloaded / Uploaded / Relevance / Files
fn detail_peers(ui: &mut egui::Ui, p: &Palette, list: &[Peer]) {
    if list.is_empty() {
        empty_hint(ui, p, "当前没有已连接的节点");
        return;
    }
    table_borderless(ui);
    TableBuilder::new(ui)
        .id_salt("d_peers")
        .striped(false)
        .resizable(true)
        .sense(Sense::hover())
        .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
        .auto_shrink([false, false])
        .min_scrolled_height(120.0)
        .max_scroll_height(420.0)
        .column(Column::remainder().at_least(110.0).clip(true))
        .column(Column::initial(52.0).at_least(46.0))
        .column(Column::initial(56.0).at_least(48.0))
        .column(Column::initial(126.0).at_least(70.0).clip(true))
        .column(Column::initial(116.0).at_least(70.0).clip(true))
        .column(Column::initial(96.0).at_least(72.0))
        .column(Column::initial(78.0).at_least(64.0))
        .column(Column::initial(78.0).at_least(64.0))
        .column(Column::initial(78.0).at_least(62.0))
        .column(Column::initial(78.0).at_least(62.0))
        .column(Column::initial(58.0).at_least(48.0))
        .column(Column::initial(74.0).at_least(50.0).clip(true))
        .header(th::HEADER_H, |mut h| {
            thead(&mut h, p, "IP");
            thead_r(&mut h, p, "端口");
            thead(&mut h, p, "连接");
            thead(&mut h, p, "标志");
            thead(&mut h, p, "客户端");
            thead(&mut h, p, "进度");
            thead_r(&mut h, p, "下载");
            thead_r(&mut h, p, "上传");
            thead_r(&mut h, p, "已下载");
            thead_r(&mut h, p, "已上传");
            thead_r(&mut h, p, "相关度");
            thead(&mut h, p, "文件");
        })
        .body(|body| {
            body.rows(24.0, list.len(), |mut row| {
                let i = row.index();
                let pe = &list[i];
                row.col(|ui| w::text_cell(ui, p, &pe.ip, p.text, 12.0));
                row.col(|ui| w::num_cell(ui, p, &pe.port.to_string(), p.faint));
                row.col(|ui| {
                    let c = if pe.is_utp() {
                        p.dl
                    } else if pe.connection == "Web" {
                        p.accent
                    } else {
                        p.muted
                    };
                    w::text_cell(ui, p, dash_if_empty(&pe.connection).as_str(), c, 11.5);
                });
                row.col(|ui| {
                    w::text_cell(ui, p, &pe.flags_text(), p.muted, 11.0);
                });
                row.col(|ui| {
                    w::text_cell(ui, p, dash_if_empty(&pe.client).as_str(), p.text, 11.5);
                });
                row.col(|ui| {
                    w::progress(
                        ui,
                        p,
                        egui::Id::new(("d_peer_pg", i)),
                        pe.progress as f32,
                        if pe.progress >= 1.0 { p.up } else { p.dl },
                    );
                });
                row.col(|ui| {
                    let c = if pe.dl_speed > 0 { p.dl } else { p.faint };
                    w::num_cell(ui, p, &fmt_speed(pe.dl_speed), c);
                });
                row.col(|ui| {
                    let c = if pe.up_speed > 0 { p.up } else { p.faint };
                    w::num_cell(ui, p, &fmt_speed(pe.up_speed), c);
                });
                row.col(|ui| w::num_cell(ui, p, &fmt_size(pe.downloaded), p.muted));
                row.col(|ui| w::num_cell(ui, p, &fmt_size(pe.uploaded), p.muted));
                row.col(|ui| {
                    w::num_cell(ui, p, &format!("{:.2}", pe.relevance), p.muted);
                });
                row.col(|ui| {
                    w::text_cell(ui, p, dash_if_empty(&pe.files).as_str(), p.faint, 11.0);
                });
            });
        });
}

/// **HTTP 源** —— 列对齐 WebUI：URL / Status
fn detail_webseeds(ui: &mut egui::Ui, p: &Palette, list: &[WebSeed]) {
    if list.is_empty() {
        empty_hint(ui, p, "该任务没有 HTTP 源（web seed）");
        return;
    }
    table_borderless(ui);
    TableBuilder::new(ui)
        .id_salt("d_webseeds")
        .striped(false)
        .resizable(true)
        .sense(Sense::hover())
        .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
        .auto_shrink([false, false])
        .min_scrolled_height(120.0)
        .max_scroll_height(420.0)
        .column(Column::remainder().at_least(300.0).clip(true))
        .column(Column::initial(110.0).at_least(80.0))
        .header(th::HEADER_H, |mut h| {
            thead(&mut h, p, "URL");
            thead(&mut h, p, "状态");
        })
        .body(|body| {
            body.rows(24.0, list.len(), |mut row| {
                let ws = &list[row.index()];
                row.col(|ui| w::text_cell(ui, p, &ws.url, p.text, 11.5));
                // qB 这个字段是 libtorrent 的状态码，WebUI 也是原样显示；
                // 我们不猜它的含义，只把 0 画成常规色、非 0 提醒一下。
                row.col(|ui| {
                    let c = if ws.status == 0 { p.muted } else { p.warn };
                    w::num_cell(ui, p, &ws.status.to_string(), c);
                });
            });
        });
}

/// **内容** —— 列对齐 WebUI：Name / Size / Progress / Priority / Remaining
fn detail_files(ui: &mut egui::Ui, p: &Palette, list: &[TorrentFile]) {
    if list.is_empty() {
        empty_hint(ui, p, "该任务没有文件信息（可能是纯 magnet，元数据还没拿到）");
        return;
    }
    table_borderless(ui);
    TableBuilder::new(ui)
        .id_salt("d_files")
        .striped(false)
        .resizable(true)
        .sense(Sense::hover())
        .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
        .auto_shrink([false, false])
        .min_scrolled_height(120.0)
        .max_scroll_height(420.0)
        .column(Column::remainder().at_least(240.0).clip(true))
        .column(Column::initial(96.0).at_least(80.0))
        .column(Column::initial(136.0).at_least(100.0))
        .column(Column::initial(80.0).at_least(66.0))
        .column(Column::initial(96.0).at_least(80.0))
        .header(th::HEADER_H, |mut h| {
            thead(&mut h, p, "名称");
            thead_r(&mut h, p, "大小");
            thead(&mut h, p, "进度");
            thead(&mut h, p, "优先级");
            thead_r(&mut h, p, "剩余");
        })
        .body(|body| {
            body.rows(24.0, list.len(), |mut row| {
                let i = row.index();
                let f = &list[i];
                row.col(|ui| path_cell(ui, p, &f.name));
                row.col(|ui| w::num_cell(ui, p, &fmt_size(f.size), p.muted));
                row.col(|ui| {
                    w::progress(
                        ui,
                        p,
                        egui::Id::new(("d_file_pg", i)),
                        f.progress as f32,
                        if f.progress >= 1.0 { p.up } else { p.dl },
                    );
                });
                row.col(|ui| {
                    let c = match f.priority {
                        0 => p.faint,
                        6 => p.warn,
                        7 => p.accent,
                        _ => p.muted,
                    };
                    w::text_cell(ui, p, f.priority_cn(), c, 11.5);
                });
                row.col(|ui| {
                    let left = ((f.size as f64) * (1.0 - f.progress).clamp(0.0, 1.0)) as i64;
                    w::num_cell(ui, p, &fmt_size(left), p.muted);
                });
            });
        });
}

/// 内容页的文件名单元：目录部分压淡、文件名正常色。
/// 一个种子里常常是一堆同前缀文件，把目录压下去之后名字才连成一片看得清。
fn path_cell(ui: &mut egui::Ui, p: &Palette, path: &str) {
    let full = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(full.max(1.0), 17.0), Sense::hover());
    if rect.width() < 8.0 {
        return;
    }
    let painter = ui.painter();
    let (dir, name) = match path.rsplit_once('/') {
        Some((d, n)) => (Some(d), n),
        None => (None, path),
    };
    let ng = painter.layout_no_wrap(name.to_owned(), th::id(12.0), Color32::PLACEHOLDER);
    let mut x = rect.left();

    if let Some(d) = dir {
        let budget = rect.right() - x - ng.size().x - 4.0;
        if budget > 20.0 {
            let dg = painter.layout_no_wrap(
                w::fit(&format!("{}/", d), budget, 11.5),
                th::id(11.5),
                Color32::PLACEHOLDER,
            );
            x += dg.size().x;
            let dy = rect.center().y - dg.size().y * 0.5;
            painter.galley(pos2(x - dg.size().x, dy), dg, p.faint);
        }
    }

    let room = rect.right() - x;
    let shown = if ng.size().x > room {
        painter.layout_no_wrap(w::fit(name, room, 12.0), th::id(12.0), Color32::PLACEHOLDER)
    } else {
        ng
    };
    let ny = rect.center().y - shown.size().y * 0.5;
    painter.galley(pos2(x, ny), shown, p.text);
}

/// 空态提示（详情各页共用）
fn empty_hint(ui: &mut egui::Ui, p: &Palette, text: &str) {
    ui.add_space(56.0);
    ui.vertical_centered(|ui| {
        ui.label(
            RichText::new(text)
                .font(th::id(12.0))
                .color(p.faint),
        );
    });
}
