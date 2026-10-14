//! qBittorrent WebAPI v2 的数据结构。
//!
//! 所有字段都带 `#[serde(default)]`，这样跨 qB 版本（4.x / 5.x）字段增减都不会导致
//! 反序列化失败 —— 缺的字段自动取默认值。

// 这里刻意完整声明 API 字段（含暂时没用到的），便于后续扩展。
#![allow(dead_code)]

use serde::Deserialize;
use std::collections::HashMap;

use crate::i18n::tr;

/// `GET /api/v2/torrents/info` 的单个条目。
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Torrent {
    pub hash: String,
    pub name: String,
    /// 总字节数
    pub size: i64,
    /// 0.0 ~ 1.0
    pub progress: f64,
    /// 当前下载速度 B/s
    pub dlspeed: i64,
    /// 当前上传速度 B/s
    pub upspeed: i64,
    pub priority: i32,
    /// 已连接的做种者
    pub num_seeds: i32,
    /// tracker 上报的做种者总数
    pub num_complete: i32,
    /// 已连接的下载者
    pub num_leechs: i32,
    pub num_incomplete: i32,
    pub ratio: f64,
    /// 剩余秒数；8640000 表示「无限」
    pub eta: i64,
    /// 见 [`Torrent::state_cn`]
    pub state: String,
    pub seq_dl: bool,
    pub f_l_piece_prio: bool,
    pub category: String,
    /// qB 用逗号分隔的字符串返回标签
    pub tags: String,
    pub super_seeding: bool,
    pub force_start: bool,
    pub save_path: String,
    pub added_on: i64,
    pub completion_on: i64,
    /// 最后一次有数据传输的时刻。详情窗口的「最近活动」用它
    pub last_activity: i64,
    pub downloaded: i64,
    pub uploaded: i64,
    /// 单任务限速，-1 表示不限
    pub dl_limit: i64,
    pub up_limit: i64,
    pub amount_left: i64,
    pub completed: i64,
    pub content_path: String,
    pub tracker: String,
    pub dl_speed_avg: i64,
    pub up_speed_avg: i64,
    pub time_active: i64,
    pub seeding_time: i64,
    pub availability: f64,
}

/// 语义化的任务状态分组，用于侧栏筛选与配色。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StateGroup {
    Downloading,
    Seeding,
    Stopped,
    Error,
    Checking,
    Other,
}

impl Torrent {
    /// 状态的中文描述。兼容 qB 5.x 把 `paused*` 改名为 `stopped*` 的变更。
    pub fn state_cn(&self) -> &'static str {
        match self.state.as_str() {
            "error" => tr("错误"),
            "missingFiles" => tr("文件丢失"),
            "uploading" => tr("做种中"),
            "forcedUP" => tr("做种中(强制)"),
            "stalledUP" => tr("做种中(无连接)"),
            "checkingUP" => tr("校验中"),
            "queuedUP" => tr("排队做种"),
            "pausedUP" | "stoppedUP" => tr("已完成(已停止)"),
            "allocating" => tr("分配空间"),
            "downloading" => tr("下载中"),
            "forcedDL" => tr("下载中(强制)"),
            "metaDL" => tr("获取元数据"),
            "stalledDL" => tr("下载中(无连接)"),
            "checkingDL" => tr("校验中"),
            "queuedDL" => tr("排队下载"),
            "pausedDL" | "stoppedDL" => tr("已暂停"),
            "stopped" => tr("已停止"),
            "checkingResumeData" => tr("校验恢复数据"),
            "moving" => tr("移动中"),
            "unknown" => tr("未知"),
            _ => tr("未知"),
        }
    }

    pub fn is_stopped(&self) -> bool {
        matches!(
            self.state.as_str(),
            "pausedDL" | "pausedUP" | "stoppedDL" | "stoppedUP" | "stopped"
        )
    }

    pub fn is_completed(&self) -> bool {
        self.progress >= 1.0
    }

    pub fn group(&self) -> StateGroup {
        match self.state.as_str() {
            "error" | "missingFiles" => StateGroup::Error,
            "downloading" | "forcedDL" | "metaDL" | "stalledDL" | "queuedDL" | "allocating"
            | "moving" => StateGroup::Downloading,
            "uploading" | "forcedUP" | "stalledUP" | "queuedUP" => StateGroup::Seeding,
            "pausedDL" | "pausedUP" | "stoppedDL" | "stoppedUP" | "stopped" => StateGroup::Stopped,
            "checkingUP" | "checkingDL" | "checkingResumeData" => StateGroup::Checking,
            _ => StateGroup::Other,
        }
    }

    /// 是否处于「活动」（有实际传输）
    pub fn is_active(&self) -> bool {
        self.dlspeed > 0 || self.upspeed > 0
    }

    /// 标签列表
    pub fn tag_list(&self) -> Vec<&str> {
        self.tags
            .split(',')
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .collect()
    }
}

/// `GET /api/v2/transfer/info`
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct TransferInfo {
    pub dl_info_speed: i64,
    pub dl_info_data: i64,
    pub up_info_speed: i64,
    pub up_info_data: i64,
    pub dl_rate_limit: i64,
    pub up_rate_limit: i64,
    pub dht_nodes: i32,
    /// `connected` / `firewalled` / `disconnected`
    pub connection_status: String,
    pub queueing: bool,
}

/// `GET /api/v2/torrents/categories`
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Category {
    pub name: String,
    #[serde(rename = "savePath")]
    pub save_path: String,
}

/// `GET /api/v2/torrents/trackers?hash=...`
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Tracker {
    pub url: String,
    /// 0=禁用, 1=未联系, 2=工作中, 3=更新中, 4=不工作
    pub status: i32,
    pub tier: i32,
    pub num_peers: i32,
    pub num_seeds: i32,
    pub num_leeches: i32,
    /// tracker 回报的"该种子被完整下载过多少次"
    pub num_downloaded: i32,
    pub msg: String,
}

impl Tracker {
    pub fn status_cn(&self) -> &'static str {
        match self.status {
            0 => tr("已禁用"),
            1 => tr("未联系"),
            2 => tr("工作中"),
            3 => tr("更新中"),
            4 => tr("不工作"),
            _ => tr("未知"),
        }
    }
}

pub type CategoryMap = HashMap<String, Category>;

// ---------------------------------------------------------------- 任务详情
//
// 对应 qB 原生 WebUI 底部「任务详情」面板的五个模块：
// 常规 / Trackers / 节点 / HTTP 源 / 内容。

/// `GET /api/v2/torrents/properties?hash=...`
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct TorrentProperties {
    pub save_path: String,
    /// UNIX 秒，0 = 未提供
    pub creation_date: i64,
    /// 分片大小（字节）
    pub piece_size: i64,
    pub comment: String,
    pub total_wasted: i64,
    pub total_uploaded: i64,
    pub total_downloaded: i64,
    pub up_speed: i64,
    pub dl_speed: i64,
    pub up_limit: i64,
    pub dl_limit: i64,
    /// 总活动时长（秒）
    pub time_elapsed: i64,
    pub seeding_time: i64,
    pub nb_connections: i32,
    pub nb_connections_limit: i32,
    pub share_ratio: f64,
    pub addition_date: i64,
    pub completion_date: i64,
    pub created_by: String,
    pub dl_speed_avg: i64,
    pub up_speed_avg: i64,
    pub eta: i64,
    pub peers: i32,
    pub peers_total: i32,
    pub pieces_have: i32,
    pub pieces_num: i32,
    pub reannounce: i64,
    pub seeds: i32,
    pub seeds_total: i32,
    pub total_size: i64,
    pub last_seen: i64,
    pub popularity: f64,
    pub infohash_v1: String,
    pub infohash_v2: String,
    pub is_private: bool,
    pub torrent_name: String,
}

/// `/api/v2/sync/torrentPeers` 里的一条连接。
///
/// 字段名刻意用 qB 的**下划线**形式（`dl_speed` / `up_speed`）—— 和
/// `torrents/info` 保持一致，免得同一个概念在代码里出现两套拼法。
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Peer {
    pub ip: String,
    pub port: i32,
    /// 传输协议：`BT` / `μTP` / `Web`
    pub connection: String,
    /// ⚠️ **类型在跨版本之间变过**：4.x 是单字母串（`"D U"`），
    /// 5.x 变成了位域整数。用 `de_str_or_num` 统一转成字符串。
    #[serde(default, deserialize_with = "de_str_or_num")]
    pub flags: String,
    /// qB 4.4+ 才有：可读的标志描述（`"正在下载 · 允许上传"`）。
    #[serde(default, deserialize_with = "de_str_or_num")]
    pub flags_description: String,
    #[serde(default, deserialize_with = "de_str_or_num")]
    pub client: String,
    pub progress: f64,
    pub dl_speed: i64,
    pub up_speed: i64,
    pub downloaded: i64,
    pub uploaded: i64,
    pub relevance: f64,
    /// 有的版本给字符串（`"None"`），有的给数组，容错成字符串
    #[serde(default, deserialize_with = "de_str_or_num")]
    pub files: String,
    pub country: String,
}

/// 把「字符串 / 数字 / 数组」统一读成字符串。
///
/// 这是为了跨 qB 版本兼容：同一个字段在新旧版本里类型不一样，
/// 直接用 `String` 会**静默**反序列化失败（外层 `unwrap_or_default()`
/// 会让整条记录变成空对象，界面上就是"节点全都空着"这种鬼现象）。
fn de_str_or_num<'de, D>(d: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::Deserialize;
    let v = serde_json::Value::deserialize(d)?;
    Ok(match v {
        serde_json::Value::String(s) => s,
        serde_json::Value::Null => String::new(),
        serde_json::Value::Array(a) => {
            let parts: Vec<String> = a
                .into_iter()
                .map(|x| match x {
                    serde_json::Value::String(s) => s,
                    other => other.to_string(),
                })
                .collect();
            parts.join(", ")
        }
        other => other.to_string(),
    })
}

impl Peer {
    /// 优先用描述性的标志（`D U` → `已下载未上传`那种单字母串读不出意思），
    /// 旧版没有这个字段时退回 `flags`。
    pub fn flags_text(&self) -> String {
        if !self.flags_description.is_empty() {
            self.flags_description.clone()
        } else {
            flags_letters_cn(&self.flags)
        }
    }

    pub fn is_utp(&self) -> bool {
        self.connection == "μTP"
    }
}

/// qB 单字母连接标志 → 中文。多字母用空格分隔，逐个翻译。
fn flags_letters_cn(flags: &str) -> String {
    let mut out: Vec<&str> = Vec::new();
    for c in flags.chars() {
        let s = match c {
            'D' => tr("正在下载"),
            'd' => tr("想要下载"),
            'U' => tr("正在上传"),
            'u' => tr("允许上传"),
            'O' => tr("乐观解阻塞"),
            'S' => tr("被阻塞"),
            'I' => tr("对方感兴趣"),
            'i' => tr("表示感兴趣"),
            'K' => tr("已解除阻塞"),
            '?' => tr("未知"),
            'E' => tr("加密"),
            'H' => tr("来自 DHT"),
            'X' => tr("来自 PEX"),
            'L' => tr("来自本地"),
            _ => continue,
        };
        out.push(s);
    }
    if out.is_empty() {
        "-".to_string()
    } else {
        out.join(" · ")
    }
}

/// `GET /api/v2/torrents/webseeds?hash=...`（HTTP 源）
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct WebSeed {
    pub url: String,
    /// qB 原样返回的整数值（libtorrent 的 web seed 状态码），
    /// WebUI 也是原样显示 —— 这里不做「猜测式」翻译。
    pub status: i64,
}

/// `GET /api/v2/torrents/files?hash=...`（内容）
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct TorrentFile {
    /// 相对种子根目录的路径，用 `/` 分隔
    pub name: String,
    pub size: i64,
    /// 0.0 ~ 1.0
    pub progress: f64,
    /// 0 = 不下载，1 = 普通，6 = 高，7 = 最高
    pub priority: i32,
    pub is_seed: bool,
    pub availability: f64,
}

impl TorrentFile {
    pub fn priority_cn(&self) -> &'static str {
        match self.priority {
            0 => tr("不下载"),
            1 => tr("普通"),
            6 => tr("高"),
            7 => tr("最高"),
            _ => tr("普通"),
        }
    }
}

