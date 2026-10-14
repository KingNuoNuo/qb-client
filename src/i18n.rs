//! 多语言支持。
//!
//! 设计要点：
//! * 中文原文即「键」。`tr("中文")` 按当前语言返回对应文字；当前语言存于线程
//!   局部变量，在每帧 `update()` 开头用 [`set_lang`] 同步，因此调用处无需把
//!   `Lang` 透传进每一层函数。
//! * 语言跟随 qBittorrent WebUI 的设置：连接后读取
//!   `/api/v2/app/preferences` 的 `locale` 字段，用 [`Lang::from_qb_locale`]
//!   映射到我们支持的语言。
//! * `Lang::ZhCn` 下 `tr` 原样返回中文（零开销）；`Lang::En` 下查 [`en`]，
//!   查不到就回退中文（v1 未译全时不会崩，只是那一处仍显示中文）。
//! * 支持 **简体中文**（默认）与 **英文**。WebUI 设了其它语言而我们没翻译时，
//!   回退到英文（`Lang::En`）。

use std::cell::Cell;

/// 应用支持的语言。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    /// 简体中文（应用默认语言）
    ZhCn,
    /// 英文
    En,
}

impl Default for Lang {
    fn default() -> Self {
        Lang::ZhCn
    }
}

impl Lang {
    /// 由 qBittorrent WebUI 的 `locale` 字符串推断我们应采用的语言。
    ///
    /// qB 的 locale 形如 `en` / `es` / `zh` / `zh_CN` / `zh_TW` / `ja` …
    /// 命中中文簇 → 简体中文；其余（含我们不翻译的语言）→ 英文。
    pub fn from_qb_locale(locale: &str) -> Lang {
        let l = locale.trim().to_ascii_lowercase();
        if l == "zh"
            || l.starts_with("zh_")
            || l == "zh-cn"
            || l == "zh-tw"
            || l == "zh-hans"
            || l == "zh-hant"
        {
            Lang::ZhCn
        } else {
            Lang::En
        }
    }

    /// 该语言下「应用默认」这个兜底值的文字（设置页用）。
    #[allow(dead_code)]
    pub fn name(self) -> &'static str {
        match self {
            Lang::ZhCn => "简体中文",
            Lang::En => "English",
        }
    }
}

thread_local! {
    /// 当前语言。每帧 `update()` 开头由 [`set_lang`] 同步。
    static CUR: Cell<Lang> = Cell::new(Lang::ZhCn);
}

/// 每帧开始时调用，把当前语言同步到线程局部状态。
#[inline]
pub fn set_lang(l: Lang) {
    CUR.with(|c| c.set(l));
}

/// 读取当前语言（供少数需要按语言分支的逻辑使用）。
#[allow(dead_code)]
#[inline]
pub fn current() -> Lang {
    CUR.with(|c| c.get())
}

/// 翻译入口：中文为键，按**当前线程局部语言**返回对应字符串。
///
/// 供 UI 线程使用——`update()` 每帧开头已用 [`set_lang`] 把线程局部语言
/// 同步成 WebUI 的语言。对 `Lang::ZhCn` 直接返回 `zh`（零开销）；`Lang::En`
/// 走 [`en`] 查表。返回 `&'static str`，可直接用在 `&str` 上下文（含
/// `format!`）。
#[inline]
pub fn tr(zh: &'static str) -> &'static str {
    translate(CUR.with(|c| c.get()), zh)
}

/// 按**显式指定的语言**翻译，不依赖线程局部状态。
///
/// 后台 worker 线程没有每帧同步时可用；当前 worker 改用「连接时
/// `set_lang` 设置线程局部 + 普通幂 `tr`」的方案，此函数留作备用。
#[allow(dead_code)]
#[inline]
pub fn tr_for(lang: Lang, zh: &'static str) -> &'static str {
    translate(lang, zh)
}

#[inline]
fn translate(lang: Lang, zh: &'static str) -> &'static str {
    if lang == Lang::ZhCn {
        zh
    } else {
        en(zh)
    }
}

/// 把参数按 `{}` 占位依次填进翻译后的模板。
///
/// `format!` 的格式串必须是**字面量**，而翻译结果 `tr("…{}…")` 是运行时
/// 字符串，没法直接当格式串用；我们的模板又只使用 `{}`（不用 `{:…}`），
/// 所以自己做一个极简按序替换：
///
/// ```ignore
/// tf(tr("已选 {}"), &[&self.selected.len()])
/// ```
pub fn tf(template: &str, args: &[&dyn std::fmt::Display]) -> String {
    let mut out = String::with_capacity(template.len() + args.len() * 8);
    let mut rest = template;
    for a in args {
        match rest.find("{}") {
            Some(i) => {
                out.push_str(&rest[..i]);
                out.push_str(&a.to_string());
                rest = &rest[i + 2..];
            }
            None => break,
        }
    }
    out.push_str(rest);
    out
}

/// 英文对照表。键是中文原文，值是英文。
///
/// ⚠️ 每个中文原文只出现一次；含 `{}` 占位符的键，英文侧必须保留相同数量与
/// 顺序的 `{}`。查不到的键（含本表未列出的中文）由 `en` 末尾 `other => other`
/// 原样返回，不会编译报错也不需要穷举。
fn en(zh: &'static str) -> &'static str {
    match zh {
        // ---------- 顶栏 ----------
        "远程管理台" => "Remote Console",
        "已连接" => "Connected",
        "连接中" => "Connecting",
        "正在握手" => "Handshaking",
        "未连接" => "Disconnected",
        "点右侧「连接设置」" => "Open Connection settings on the right",
        "连接失败" => "Connection failed",
        "见底部提示" => "See hint at bottom",
        "节点" => "Nodes",
        "任务" => "Tasks",
        "上传" => "Up",
        "下载" => "Down",
        "活动 {}" => "Active {}",
        "qBittorrent {}" => "qBittorrent {}",

        // ---------- 侧栏 ----------
        "状态" => "Status",
        "全部" => "All",
        "下载中" => "Downloading",
        "做种中" => "Seeding",
        "已完成" => "Completed",
        "已停止" => "Stopped",
        "错误" => "Error",
        "活动中" => "Active",
        "分类" => "Categories",
        "（还没有分类）" => "(no categories yet)",
        "标签" => "Tags",
        "（还没有标签）" => "(no tags yet)",
        "维护" => "Maintenance",
        "+ 新建分类" => "+ New category",

        // ---------- 状态栏 ----------
        "就绪" => "Ready",
        "刷新" => "Refresh",
        "选中" => "Selected",
        "连接正常" => "Connected",
        "受防火墙限制" => "Firewalled",
        "-" => "-",

        // ---------- 工具条 ----------
        "搜索名称 / 分类 / 标签 / hash" => "Search name / category / tag / hash",
        "清除" => "Clear",
        "清空搜索条件（Esc）" => "Clear search (Esc)",
        "+ 添加任务" => "+ Add task",
        "暂停" => "Pause",
        "恢复" => "Resume",
        "删除" => "Delete",
        "已选 {}" => "Selected {}",
        "连接设置" => "Connection",
        "日志" => "Log",
        "浅色" => "Light",
        "深色" => "Dark",
        "备用限速" => "Alt speed",
        "临时切换到备用限速方案" => "Switch to alternative rate limits",
        "断开" => "Disconnect",
        "批量" => "Bulk",
        "设为分类…" => "Set category…",
        "（清空分类）" => "(clear category)",
        "标签名" => "Tag name",
        "+ 打标签" => "+ Tag",
        "全选" => "Select all",
        "取消选择" => "Clear selection",

        // ---------- 表格 ----------
        "名称" => "Name",
        "大小" => "Size",
        "进度" => "Progress",
        "分享率" => "Ratio",
        "剩余" => "ETA",
        "相关度" => "Relevance",

        // ---------- 右键菜单 ----------
        "详情" => "Details",
        "强制开始 开 / 关" => "Force start on/off",
        "顺序下载 开 / 关" => "Sequential download on/off",
        "设为分类（当前：{}）" => "Set category (current: {})",
        "该任务还没有标签" => "This task has no tags",
        "点击移除：" => "Click to remove:",
        "（没有可添加的标签）" => "(no tags to add)",
        "点击添加：" => "Click to add:",
        "复制 hash" => "Copy hash",
        "移动保存路径…" => "Move save path…",
        "重新校验" => "Recheck",
        "重新汇报 Tracker" => "Reannounce",
        "删除…" => "Delete…",

        // ---------- 空态 ----------
        "还没有任务" => "No tasks yet",
        "当前筛选条件下没有任务" => "No tasks match the current filter",
        "点工具条上的「+ 添加任务」，提交 magnet 链接或 .torrent 文件" => {
            "Click “+ Add task” to submit a magnet link or .torrent file"
        },
        "试着放宽左侧筛选，或清空搜索框" => "Loosen the filter on the left or clear the search box",

        // ---------- 弹窗：连接设置 ----------
        "直连 qBittorrent 的 WebUI，配置存在本地 JSON 里" => {
            "Connects directly to qBittorrent WebUI; config is stored in a local JSON file"
        },
        "服务器" => "Server",
        "主机 / IP" => "Host / IP",
        "端口" => "Port",
        "用户名" => "Username",
        "密码" => "Password",
        "轮询间隔" => "Poll interval",
        "秒" => "s",
        "行为" => "Behavior",
        "使用 HTTPS" => "Use HTTPS",
        "忽略证书校验（自签名证书）" => "Ignore certificate validation (self-signed)",
        "启动时自动连接" => "Auto-connect on startup",
        "显示" => "Display",
        "界面缩放" => "UI scale",
        "Ctrl + / − / 0" => "Ctrl + / − / 0",
        "配置文件：{}" => "Config file: {}",
        "注意：密码以明文保存在该文件中。" => "Note: the password is stored in plaintext in that file.",
        "保存并连接" => "Save & connect",
        "仅保存" => "Save only",
        "取消" => "Cancel",

        // ---------- 弹窗：添加 ----------
        "添加任务" => "Add task",
        "magnet / .torrent URL / 本地种子文件" => "magnet / .torrent URL / local torrent file",
        "每行一个：magnet:?xt=urn:btih:…  或  https://…/x.torrent" => {
            "One per line: magnet:?xt=urn:btih:…  or  https://…/x.torrent"
        },
        "从本地选择 .torrent 文件并添加…" => "Pick a local .torrent file to add…",
        "选项" => "Options",
        "保存路径" => "Save path",
        "留空 = 使用 qB 默认 / 分类目录" => "Leave empty = qB default / category folder",
        "（无）" => "(none)",
        "逗号分隔，如：电影, 1080p" => "Comma-separated, e.g. Movies, 1080p",
        "添加后暂停" => "Pause after adding",
        "顺序下载" => "Sequential download",
        "提交" => "Submit",
        "请先填入 magnet 链接或 URL" => "Enter a magnet link or URL first",

        // ---------- 弹窗：删除 ----------
        "删除任务" => "Delete task",
        "qBittorrent 侧的删除不可撤销，请确认范围" => {
            "Deletion on qBittorrent's side is irreversible — confirm the scope"
        },
        "即将删除 {} 个任务" => "About to delete {} task(s)",
        "… 以及另外 {} 个" => "… and {} more",
        "同时删除已下载的数据文件（不可恢复）" => "Also delete the downloaded data files (unrecoverable)",
        "删除任务与数据" => "Delete task and data",
        "仅删除任务" => "Delete task only",

        // ---------- 弹窗：限速 ----------
        "限速" => "Speed limits",
        "单位 KiB/s；0 = 不限，-1 = 跟随全局" => "Unit KiB/s; 0 = no limit, -1 = follow global",
        "全局限速" => "Global limits",
        "应用" => "Apply",
        "解除" => "Release",
        "当前 ↓ {}   ↑ {}" => "Now ↓ {}   ↑ {}",
        "选中任务的独立限速（{} 个）" => "Per-task limits for selected ({} task(s))",
        "应用到选中任务" => "Apply to selected",
        "关闭" => "Close",
        "关闭（Esc）" => "Close (Esc)",

        // ---------- 弹窗：日志 ----------
        "操作日志" => "Operation log",
        "只保留最近 150 条，重启即清空" => "Keeps only the latest 150 entries; cleared on restart",
        "（暂无日志）" => "(no log yet)",
        "共 {} 条" => "{} entries",

        // ---------- 弹窗：新建分类 ----------
        "新建分类" => "New category",
        "分类可以绑定独立的保存目录" => "A category can bind its own save folder",
        "留空 = 使用默认下载目录" => "Leave empty = default download folder",
        "创建" => "Create",

        // ---------- 弹窗：移动 ----------
        "移动保存路径" => "Move save path",
        "qB 会把已下载的数据文件一并搬到新目录" => {
            "qB will move the downloaded data files to the new folder"
        },
        "将移动 {} 个任务。" => "Will move {} task(s).",
        "当前：{}" => "Current: {}",
        "目标路径" => "Target path",
        "/downloads/new  或  D:\\下载\\new" => "/downloads/new  or  D:\\downloads\\new",
        "移动" => "Move",

        // ---------- 弹窗：详情 ----------
        "任务详情" => "Task details",
        "正在拉取任务详情…" => "Fetching task details…",
        "拉取中…" => "Fetching…",
        "刚刚更新" => "Just updated",
        "{} 秒前更新" => "{}s ago",

        // ---------- 详情页：常规字段 ----------
        "分片" => "Pieces",
        "创建者" => "Created by",
        "创建于" => "Created on",
        "添加于" => "Added on",
        "完成于" => "Completed on",
        "最近看完整" => "Last seen complete",
        "最近活动" => "Last activity",
        "总大小" => "Total size",
        "已下载" => "Downloaded",
        "已上传" => "Uploaded",
        "浪费" => "Wasted",
        "活动时间" => "Active time",
        "下载耗时" => "Download time",
        "做种时间" => "Seeding time",
        "连接数" => "Connections",
        "做种者" => "Seeds",
        "下载限速" => "DL limit",
        "上传限速" => "UP limit",
        "剩余时间" => "ETA",
        "下载速度" => "DL speed",
        "上传速度" => "UP speed",
        "平均下载" => "Avg DL",
        "平均上传" => "Avg UP",
        "热度" => "Popularity",
        "私有种子" => "Private",
        "信息哈希 v1" => "Info hash v1",
        "信息哈希 v2" => "Info hash v2",
        "Tracker" => "Tracker",
        "内容路径" => "Content path",
        "注释" => "Comment",
        "是" => "Yes",
        "否" => "No",

        // ---------- 详情页：Trackers / 节点 / HTTP 源 ----------
        "URL" => "URL",
        "做种" => "Seeds",
        "消息" => "Message",
        "连接" => "Conn",
        "标志" => "Flags",
        "客户端" => "Client",
        "文件" => "Files",
        "该任务没有 Tracker 数据" => "No tracker data for this task",
        "当前没有已连接的节点" => "No peers connected right now",
        "该任务没有 HTTP 源（web seed）" => "No HTTP sources (web seeds) for this task",

        // ---------- 详情页：内容 ----------
        "优先级" => "Priority",
        "该任务没有文件信息（可能是纯 magnet，元数据还没拿到）" => {
            "No file info yet (maybe a pure magnet — metadata not fetched)"
        },
        "最高" => "Max",
        "高" => "High",
        "普通" => "Normal",
        "不下载" => "Don't download",

        // ---------- 状态/优先级/标志（model.rs）----------
        "文件丢失" => "Missing files",
        "做种中(强制)" => "Seeding (forced)",
        "做种中(无连接)" => "Seeding (no connection)",
        "校验中" => "Checking",
        "排队做种" => "Queued seed",
        "已完成(已停止)" => "Completed (paused)",
        "分配空间" => "Allocating",
        "获取元数据" => "Fetching metadata",
        "下载中(无连接)" => "Downloading (no connection)",
        "排队下载" => "Queued download",
        "已暂停" => "Paused",
        "校验恢复数据" => "Checking resume data",
        "移动中" => "Moving",
        "未知" => "Unknown",
        "已禁用" => "Disabled",
        "未联系" => "Not contacted",
        "工作中" => "Working",
        "更新中" => "Updating",
        "不工作" => "Not working",
        "正在下载" => "Downloading",
        "想要下载" => "Want",
        "正在上传" => "Uploading",
        "允许上传" => "Allowed",
        "乐观解阻塞" => "Optimistic",
        "被阻塞" => "Blocked",
        "对方感兴趣" => "Interested",
        "表示感兴趣" => "Interest",
        "已解除阻塞" => "Unchoked",
        "加密" => "Encrypted",
        "来自 DHT" => "from DHT",
        "来自 PEX" => "from PEX",
        "来自本地" => "from local",
        "跟随全局" => "Follow global",
        "不限" => "Unlimited",
        "{}天{}时" => "{}d {}h",
        "{}时{}分" => "{}h {}m",
        "{}分" => "{}m",

        // ---------- worker 日志 ----------
        "已连接到 {} — qBittorrent {} / WebAPI {}" => "Connected to {} — qBittorrent {} / WebAPI {}",
        "连接失败：{}" => "Connection failed: {}",
        "会话失效，重新登录失败" => "Session expired and re-login failed",
        "会话失效且重新登录失败" => "Session expired and re-login failed",
        "已断开连接" => "Disconnected",
        "全部任务（{}）" => "All tasks ({})",
        "{} 个任务（{}）" => "{} task(s) ({})",
        "已提交 {} 个 URL / magnet 任务" => "Submitted {} URL / magnet task(s)",
        "已提交种子文件 {}" => "Submitted torrent file {}",
        "已暂停 {}" => "Paused {}",
        "已恢复 {}" => "Resumed {}",
        "已删除 {}{}" => "Deleted {}{}",
        "并删除数据文件" => "and data files deleted",
        "已开始重新校验" => "Recheck started",
        "已请求重新汇报 Tracker" => "Reannounce requested",
        "已设置分类为「{}」" => "Category set to “{}”",
        "已创建分类「{}」" => "Category “{}” created",
        "已添加标签 {}" => "Tagged {}",
        "已移除标签 {}" => "Tag “{}” removed",
        "已开启强制开始" => "Force start enabled",
        "已关闭强制开始" => "Force start disabled",
        "已开启顺序下载" => "Sequential download enabled",
        "已关闭顺序下载" => "Sequential download disabled",
        "已移动至 {}" => "Moved to {}",
        "已取消全局下载限速" => "Global DL limit cleared",
        "全局下载限速 {} KiB/s" => "Global DL limit {} KiB/s",
        "已取消全局上传限速" => "Global UP limit cleared",
        "全局上传限速 {} KiB/s" => "Global UP limit {} KiB/s",
        "已更新 {} 个任务的限速" => "Limits updated for {} task(s)",
        "已切换备用限速模式" => "Alternative speed mode toggled",
        "已调整 {} 个文件的优先级" => "Priority updated for {} file(s)",
        "会话已刷新，请重试刚才的操作" => "Session refreshed, retry the action",
        "常规：" => "General: ",
        "Trackers：" => "Trackers: ",
        "节点：" => "Peers: ",
        "HTTP 源：" => "HTTP sources: ",
        "内容：" => "Files: ",

        // ---------- 补漏（audit 找出的） ----------
        "常规" => "General",
        "内容" => "Files",
        "HTTP 源" => "HTTP sources",
        "设为分类" => "Set category",
        "重置" => "Reset",
        "无" => "None",
        "切换深浅色主题" => "Toggle light/dark theme",
        "已连 {} / 共 {}" => "Connected {} / total {}",
        "{} × {}（已有 {}）" => "{} × {} (have {})",
        "（该任务还没有标签）" => "(this task has no tags yet)",
        "下载中(强制)" => "Downloading (forced)",
        "，并删除数据文件" => ", and data files deleted",
        "{}\n保存路径：{}\nhash：{}" => "{}\nSave path: {}\nhash: {}",
        "选择 .torrent 文件" => "Choose a .torrent file",
        "BitTorrent 种子" => "BitTorrent torrent",
        "读取种子文件失败：{}" => "Failed to read torrent file: {}",
        "目标路径不能为空" => "Target path cannot be empty",
        "分类名不能为空" => "Category name cannot be empty",
        "服务器返回 HTTP {}" => "Server returned HTTP {}",
        "登录请求失败：HTTP {}" => "Login request failed: HTTP {}",
        "登录被拒绝：用户名或密码错误，或该 IP 已被临时封禁（qB 默认失败 5 次封 1 小时）" => {
            "Login rejected: wrong username/password, or the IP is temporarily banned (qB bans after 5 failures for 1 hour)"
        },
        "登录失败（返回 \"{}\"）：请检查用户名 / 密码" => {
            "Login failed (returned \"{}\"): check username / password"
        },
        "会话失效" => "Session expired",
        "会话已失效，需要重新登录" => "Session expired, re-login required",
        "无法创建网络线程" => "Failed to spawn the network thread",
        "操作失败" => "Operation failed",
        "未连接，指令已忽略" => "Not connected — command ignored",
        "配置已保存" => "Config saved",
        "保存配置失败：{}" => "Failed to save config: {}",
        "已复制 hash：{}" => "Hash copied: {}",

        // 未列出的中文原文 → 原样返回（英文模式下仍显示中文，v1 逐步补全）
        other => other,
    }
}
