//! 后台网络线程。
//!
//! UI 线程只读共享状态、只通过 channel 下发指令；所有网络 IO 都在这里串行执行，
//! 保证 UI 永不卡顿，也避免多线程同时操作 qB 会话。
//!
//! # 轮询策略：快慢分层
//!
//! 一轮完整刷新要打 **7 个**端点。但其中只有 `torrents/info` 与 `transfer/info`
//! 是秒级变化的（进度、速度）；分类、标签、备用限速开关、全局限速值
//! 在几秒内根本不会动。原来它们跟着同一个间隔一起拉，
//! 等于**大部分请求都在问一批不会变的答案**。
//!
//! 现在拆成两条通道：
//!   * **快通道**（每个轮询间隔）：`torrents/info` + `transfer/info` → 2 个请求
//!   * **慢通道**（[`SLOW_REFRESH`]）：分类 / 标签 / 备用限速 / 全局限速 → 5 个请求
//!
//! 另外：**任何写操作完成后强制走一次慢通道**。用户刚建完分类、刚打完标签，
//! 界面必须在下一帧就看到结果，不能等 15 秒。

use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::api::{is_auth_error, AddOptions, QbClient};
use crate::config::Config;
use crate::model::{
    Peer, Torrent, TorrentFile, TorrentProperties, Tracker, TransferInfo, WebSeed,
};

/// 日志环形缓冲上限
const MAX_LOG: usize = 150;

/// 慢变数据的刷新间隔
const SLOW_REFRESH: Duration = Duration::from_secs(15);

/// 退出时留给网络线程收尾（发一次 logout）的时间上限
const SHUTDOWN_GRACE: Duration = Duration::from_millis(1500);

#[derive(Debug, Clone, PartialEq)]
pub enum Conn {
    Disconnected,
    Connecting,
    Connected,
    Error(String),
}

/// 详情面板的一份数据包。
///
/// 五个模块一次拉齐（原生 WebUI 也是这么干的：切 Tab 不再发请求），
/// 所以这里是一个整体而不是五个各自独立的字段 —— 否则很容易出现
/// 「常规是 A 任务的、Trackers 还是 B 任务的」这种错位。
#[derive(Default, Clone)]
pub struct DetailData {
    /// 这份数据属于哪个任务。空串 = 还没有数据。
    pub hash: String,
    pub props: Option<TorrentProperties>,
    pub trackers: Vec<Tracker>,
    pub peers: Vec<Peer>,
    pub webseeds: Vec<WebSeed>,
    pub files: Vec<TorrentFile>,
    pub loading: bool,
    /// 部分端点失败的说明（成功的部分照常显示，不清空）
    pub error: Option<String>,
    /// 拉取完成时刻
    pub at: Option<Instant>,
}

impl DetailData {
    pub fn is_for(&self, hash: &str) -> bool {
        self.hash == hash && !self.hash.is_empty()
    }
}

/// UI 与后台线程共享的状态。
pub struct Shared {
    pub conn: Conn,
    pub version: String,
    pub webapi: String,
    pub transfer: TransferInfo,
    /// `Arc` 包一层：UI 每帧只做一次引用计数自增，不必深拷贝整份任务列表。
    pub torrents: Arc<Vec<Torrent>>,
    pub categories: Vec<String>,
    pub tags: Vec<String>,
    pub alt_speed: bool,
    pub global_dl_limit: i64,
    pub global_up_limit: i64,
    pub last_error: Option<String>,
    pub last_update: Option<Instant>,
    /// 同样用 `Arc`，避免每帧克隆最多 150 行日志字符串。
    pub log: Arc<Vec<String>>,
    pub detail: DetailData,
}

impl Default for Shared {
    fn default() -> Self {
        Self {
            conn: Conn::Disconnected,
            version: String::new(),
            webapi: String::new(),
            transfer: TransferInfo::default(),
            torrents: Arc::new(Vec::new()),
            categories: Vec::new(),
            tags: Vec::new(),
            alt_speed: false,
            global_dl_limit: 0,
            global_up_limit: 0,
            last_error: None,
            last_update: None,
            log: Arc::new(Vec::new()),
            detail: DetailData::default(),
        }
    }
}

impl Shared {
    pub fn log_insert(&mut self, line: String) {
        let ts = chrono::Local::now().format("%H:%M:%S").to_string();
        // `Arc::make_mut`：只有 UI 恰好正持有快照（引用计数 > 1）时才真的克隆一次，
        // 平时就是原地 push。
        let log = Arc::make_mut(&mut self.log);
        log.push(format!("[{}] {}", ts, line));
        if log.len() > MAX_LOG {
            let drop = log.len() - MAX_LOG;
            log.drain(0..drop);
        }
    }
}

/// UI → 后台线程的指令。
pub enum Cmd {
    Connect(Config),
    Disconnect,
    /// 只改轮询间隔，不重连（改设置时不该把会话踢掉）
    UpdateRefresh(u64),
    RefreshNow,
    AddUrls { urls: String, opts: AddOptions },
    AddFile { name: String, data: Vec<u8>, opts: AddOptions },
    Pause(Vec<String>),
    Resume(Vec<String>),
    Delete { hashes: Vec<String>, files: bool },
    Recheck(Vec<String>),
    Reannounce(Vec<String>),
    SetCategory { hashes: Vec<String>, category: String },
    CreateCategory { name: String, save_path: String },
    AddTags { hashes: Vec<String>, tags: String },
    RemoveTags { hashes: Vec<String>, tags: String },
    SetForceStart { hashes: Vec<String>, value: bool },
    SetSequential { hashes: Vec<String>, value: bool },
    SetLocation { hashes: Vec<String>, path: String },
    SetGlobalDlLimit(i64),
    SetGlobalUpLimit(i64),
    SetTorrentLimit { hashes: Vec<String>, dl: Option<i64>, up: Option<i64> },
    ToggleAltSpeed,
    /// 拉一次完整详情（五个模块）
    FetchDetail(String),
    Shutdown,
}

pub struct Worker {
    pub shared: Arc<Mutex<Shared>>,
    tx: Sender<Cmd>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl Worker {
    pub fn spawn(initial: Config) -> Self {
        let shared = Arc::new(Mutex::new(Shared::default()));
        let (tx, rx) = mpsc::channel::<Cmd>();
        let sh = shared.clone();
        let handle = std::thread::Builder::new()
            .name("qb-net".into())
            .spawn(move || run(sh, rx, initial))
            .expect("无法创建网络线程");

        Self {
            shared,
            tx,
            handle: Some(handle),
        }
    }

    pub fn send(&self, cmd: Cmd) {
        let _ = self.tx.send(cmd);
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.tx.send(Cmd::Shutdown);
        if let Some(h) = self.handle.take() {
            // 网络线程可能正卡在一个最长 20s 超时的 HTTP 请求上。
            // 给它一点时间体面收尾（发 logout），但**不能无限等** ——
            // 否则用户关窗口会看到界面冻十几秒。
            let deadline = Instant::now() + SHUTDOWN_GRACE;
            while !h.is_finished() && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(20));
            }
            if h.is_finished() {
                let _ = h.join();
            }
            // 未能及时退出就放弃 join，让进程退出把它带走 ——
            // 此时真正重要的那次 logout 请求早就发出去了。
        }
    }
}

// ---------------- 内部实现 ----------------

fn set<F: FnOnce(&mut Shared)>(shared: &Arc<Mutex<Shared>>, f: F) {
    if let Ok(mut g) = shared.lock() {
        f(&mut g);
    }
}

/// 慢变数据的本地缓存。放在网络线程的栈上，不必进 `Shared`。
#[derive(Default)]
struct SlowCache {
    /// 上次拉取时刻；`None` = 从没拉过，或已被标记为需要强制全量
    at: Option<Instant>,
    categories: Vec<String>,
    tags: Vec<String>,
    alt: bool,
    dl: i64,
    up: i64,
}

impl SlowCache {
    /// 让下一次 `refresh` 强制走全量
    fn invalidate(&mut self) {
        self.at = None;
    }
}

fn run(shared: Arc<Mutex<Shared>>, rx: Receiver<Cmd>, initial: Config) {
    let mut cfg = initial;
    let mut client: Option<QbClient> = None;
    let mut slow = SlowCache::default();

    if cfg.auto_connect {
        do_connect(&shared, &mut client, &cfg);
    }

    loop {
        let timeout = Duration::from_secs(cfg.refresh_secs.clamp(1, 60));

        let cmd = match rx.recv_timeout(timeout) {
            Ok(c) => c,
            Err(RecvTimeoutError::Timeout) => {
                if client.is_some() {
                    refresh(&shared, &mut client, &mut slow);
                }
                continue;
            }
            Err(RecvTimeoutError::Disconnected) => break,
        };

        match cmd {
            Cmd::Shutdown => {
                if let Some(c) = client.as_ref() {
                    let _ = c.logout();
                }
                break;
            }
            Cmd::Connect(c) => {
                cfg = c;
                do_connect(&shared, &mut client, &cfg);
                // 换了一台服务器，缓存里的分类 / 标签全部作废
                slow = SlowCache::default();
                if client.is_some() {
                    refresh(&shared, &mut client, &mut slow);
                }
            }
            Cmd::Disconnect => {
                if let Some(c) = client.as_ref() {
                    let _ = c.logout();
                }
                client = None;
                slow = SlowCache::default();
                set(&shared, |s| {
                    s.conn = Conn::Disconnected;
                    s.torrents = Arc::new(Vec::new());
                    s.version.clear();
                    s.webapi.clear();
                    s.detail = DetailData::default();
                    s.log_insert("已断开连接".into());
                });
            }
            Cmd::UpdateRefresh(secs) => {
                // 只改节奏，不动会话 —— 用户改个数字不该被踢下线
                cfg.refresh_secs = secs;
            }
            Cmd::RefreshNow => {
                if client.is_some() {
                    slow.invalidate(); // 手动刷新 = 拉全量
                    refresh(&shared, &mut client, &mut slow);
                }
            }
            Cmd::FetchDetail(hash) => {
                fetch_detail(&shared, &client, hash);
            }
            other => {
                if client.is_some() {
                    run_op(&shared, &mut client, other);
                    // 写操作可能改了分类 / 标签 / 限速，强制下一次走慢通道
                    slow.invalidate();
                    refresh(&shared, &mut client, &mut slow);
                } else {
                    set(&shared, |s| s.log_insert("未连接，指令已忽略".into()));
                }
            }
        }
    }
}

fn do_connect(shared: &Arc<Mutex<Shared>>, client: &mut Option<QbClient>, cfg: &Config) {
    set(shared, |s| {
        s.conn = Conn::Connecting;
        s.last_error = None;
    });

    let base = cfg.base_url();
    let result = (|| -> anyhow::Result<(QbClient, String, String)> {
        let c = QbClient::new(&base, &cfg.username, &cfg.password, cfg.ignore_cert)?;
        c.login()?;
        let ver = c.app_version().unwrap_or_else(|_| "未知".into());
        let web = c.webapi_version().unwrap_or_else(|_| "未知".into());
        Ok((c, ver, web))
    })();

    match result {
        Ok((c, ver, web)) => {
            *client = Some(c);
            set(shared, |s| {
                s.conn = Conn::Connected;
                s.version = ver.clone();
                s.webapi = web.clone();
                s.last_error = None;
                s.log_insert(format!(
                    "已连接到 {} — qBittorrent {} / WebAPI {}",
                    base, ver, web
                ));
            });
        }
        Err(e) => {
            *client = None;
            let msg = format!("{:#}", e);
            set(shared, |s| {
                s.conn = Conn::Error(msg.clone());
                s.last_error = Some(msg.clone());
                s.log_insert(format!("连接失败：{}", msg));
            });
        }
    }
}

/// 拉一次数据。会话失效时自动重登一次。
///
/// 快通道每轮都走；慢通道按 [`SLOW_REFRESH`] 节流（见文件头）。
fn refresh(shared: &Arc<Mutex<Shared>>, client: &mut Option<QbClient>, slow: &mut SlowCache) {
    let Some(c) = client.as_mut() else { return };

    let list = match c.torrents() {
        Ok(l) => l,
        Err(e) => {
            if is_auth_error(&e) && c.login().is_ok() {
                return; // 重登成功，下一轮再取
            }
            if is_auth_error(&e) {
                *client = None;
                set(shared, |s| {
                    s.conn = Conn::Error("会话失效且重新登录失败".into());
                    s.log_insert("会话失效，重新登录失败".into());
                });
                return;
            }
            let msg = format!("{:#}", e);
            set(shared, |s| {
                s.last_error = Some(msg.clone());
                s.conn = Conn::Error(msg);
            });
            return;
        }
    };

    let transfer = c.transfer_info().unwrap_or_default();

    // ---- 慢通道 ----
    let due = slow.at.map_or(true, |t| t.elapsed() >= SLOW_REFRESH);
    if due {
        slow.at = Some(Instant::now());
        // 每一项单独取：某一路失败就沿用上一次的值，不至于整块被清空
        if let Ok(m) = c.categories() {
            let mut v: Vec<String> = m.keys().cloned().collect();
            v.sort();
            slow.categories = v;
        }
        if let Ok(mut v) = c.tags() {
            v.sort();
            slow.tags = v;
        }
        if let Ok(b) = c.speed_limits_mode() {
            slow.alt = b;
        }
        if let Ok(n) = c.global_download_limit() {
            slow.dl = n;
        }
        if let Ok(n) = c.global_upload_limit() {
            slow.up = n;
        }
    }

    // 先把要写进 shared 的值取出来，闭包里就不必再借用 slow
    let cats = slow.categories.clone();
    let tags = slow.tags.clone();
    let alt = slow.alt;
    let dl = slow.dl;
    let up = slow.up;

    set(shared, |s| {
        s.torrents = Arc::new(list);
        s.transfer = transfer;
        s.categories = cats;
        s.tags = tags;
        s.alt_speed = alt;
        s.global_dl_limit = dl;
        s.global_up_limit = up;
        s.conn = Conn::Connected;
        s.last_error = None;
        s.last_update = Some(Instant::now());
    });
}

fn fetch_detail(shared: &Arc<Mutex<Shared>>, client: &Option<QbClient>, hash: String) {
    // 标记加载中，但**先不删旧数据** —— 用户点「刷新」时界面不该闪成空白。
    set(shared, |s| {
        s.detail.hash = hash.clone();
        s.detail.loading = true;
        s.detail.error = None;
    });

    let Some(c) = client.as_ref() else {
        set(shared, |s| {
            s.detail.loading = false;
            s.detail.error = Some("未连接".into());
        });
        return;
    };

    // 五个端点逐个拉，各自失败只记一笔 —— 某一个端点不支持（比如老版本
    // 没有 infohash_v2）不该把整块详情都判死。
    let mut errs: Vec<String> = Vec::new();
    let props = c.properties(&hash).map_err(|e| errs.push(format!("常规：{:#}", e))).ok();
    let trackers = c.trackers(&hash).map_err(|e| errs.push(format!("Trackers：{:#}", e))).ok();
    let peers = c.peers(&hash).map_err(|e| errs.push(format!("节点：{:#}", e))).ok();
    let webseeds = c.webseeds(&hash).map_err(|e| errs.push(format!("HTTP 源：{:#}", e))).ok();
    let files = c.torrent_files(&hash).map_err(|e| errs.push(format!("内容：{:#}", e))).ok();

    let err = if errs.is_empty() { None } else { Some(errs.join(" / ")) };

    set(shared, move |s| {
        // 中途用户切了任务，这批数据直接丢弃（`fetch_detail` 是串行的，
        // 但一个请求要 20s 超时，期间完全可能已经切走）
        if s.detail.hash != hash {
            return;
        }
        if let Some(v) = props {
            s.detail.props = Some(v);
        }
        if let Some(v) = trackers {
            s.detail.trackers = v;
        }
        if let Some(v) = peers {
            s.detail.peers = v;
        }
        if let Some(v) = webseeds {
            s.detail.webseeds = v;
        }
        if let Some(v) = files {
            s.detail.files = v;
        }
        s.detail.loading = false;
        s.detail.error = err;
        s.detail.at = Some(Instant::now());
    });
}

fn report_ok(shared: &Arc<Mutex<Shared>>, msg: impl Into<String>) {
    let m = msg.into();
    set(shared, |s| {
        s.log_insert(m);
        s.last_error = None;
    });
}

fn report_err(shared: &Arc<Mutex<Shared>>, what: &str, e: anyhow::Error) {
    let m = format!("{}：{:#}", what, e);
    set(shared, |s| {
        s.log_insert(m.clone());
        s.last_error = Some(m);
    });
}

fn named(target: &[String], what: &str) -> String {
    if target.len() == 1 && target[0] == "all" {
        format!("全部任务（{}）", what)
    } else {
        format!("{} 个任务（{}）", target.len(), what)
    }
}

/// 执行一次操作指令。
///
/// 到达这里的只可能是**写操作**（连接 / 断开 / 手动刷新 / Tracker 查询
/// 在 `run` 里已单独处理）。
fn run_op(shared: &Arc<Mutex<Shared>>, client: &mut Option<QbClient>, cmd: Cmd) {
    let c = match client.as_ref() {
        Some(c) => c,
        None => return,
    };

    let r: anyhow::Result<()> = match cmd {
        Cmd::AddUrls { urls, opts } => {
            let n = urls.lines().filter(|l| !l.trim().is_empty()).count();
            c.add_urls(&urls, &opts)
                .map(|_| report_ok(shared, format!("已提交 {} 个 URL / magnet 任务", n)))
        }
        Cmd::AddFile { name, data, opts } => c
            .add_file(&name, data, &opts)
            .map(|_| report_ok(shared, format!("已提交种子文件 {}", name))),

        Cmd::Pause(h) => c.pause(&h).map(|_| report_ok(shared, format!("已暂停 {}", named(&h, "暂停")))),
        Cmd::Resume(h) => c.resume(&h).map(|_| report_ok(shared, format!("已恢复 {}", named(&h, "恢复")))),

        Cmd::Delete { hashes, files } => c.delete(&hashes, files).map(|_| {
            report_ok(
                shared,
                format!(
                    "已删除 {}{}",
                    named(&hashes, "删除"),
                    if files { "，并删除数据文件" } else { "" }
                ),
            )
        }),

        Cmd::Recheck(h) => c.recheck(&h).map(|_| report_ok(shared, "已开始重新校验")),
        Cmd::Reannounce(h) => c.reannounce(&h).map(|_| report_ok(shared, "已请求重新汇报 Tracker")),

        Cmd::SetCategory { hashes, category } => c.set_category(&hashes, &category).map(|_| {
            report_ok(
                shared,
                format!(
                    "已设置分类为「{}」",
                    if category.trim().is_empty() { "无" } else { category.as_str() }
                ),
            )
        }),

        Cmd::CreateCategory { name, save_path } => {
            c.create_category(&name, &save_path).map(|_| {
                report_ok(shared, format!("已创建分类「{}」", name));
            })
        }

        Cmd::AddTags { hashes, tags } => c
            .add_tags(&hashes, &tags)
            .map(|_| report_ok(shared, format!("已添加标签 {}", tags))),

        Cmd::RemoveTags { hashes, tags } => c
            .remove_tags(&hashes, &tags)
            .map(|_| report_ok(shared, format!("已移除标签 {}", tags))),

        Cmd::SetForceStart { hashes, value } => c.set_force_start(&hashes, value).map(|_| {
            report_ok(shared, if value { "已开启强制开始" } else { "已关闭强制开始" })
        }),

        Cmd::SetSequential { hashes, value } => c.set_sequential(&hashes, value).map(|_| {
            report_ok(shared, if value { "已开启顺序下载" } else { "已关闭顺序下载" })
        }),

        Cmd::SetLocation { hashes, path } => c
            .set_location(&hashes, &path)
            .map(|_| report_ok(shared, format!("已移动至 {}", path))),

        Cmd::SetGlobalDlLimit(v) => c.set_global_download_limit(v).map(|_| {
            report_ok(
                shared,
                if v <= 0 {
                    "已取消全局下载限速".to_string()
                } else {
                    format!("全局下载限速 {} KiB/s", v / 1024)
                },
            )
        }),

        Cmd::SetGlobalUpLimit(v) => c.set_global_upload_limit(v).map(|_| {
            report_ok(
                shared,
                if v <= 0 {
                    "已取消全局上传限速".to_string()
                } else {
                    format!("全局上传限速 {} KiB/s", v / 1024)
                },
            )
        }),

        Cmd::SetTorrentLimit { hashes, dl, up } => {
            let mut res: anyhow::Result<()> = Ok(());
            if let Some(d) = dl {
                res = c.set_torrent_download_limit(&hashes, d);
            }
            if res.is_ok() {
                if let Some(u) = up {
                    res = c.set_torrent_upload_limit(&hashes, u);
                }
            }
            res.map(|_| report_ok(shared, format!("已更新 {} 个任务的限速", hashes.len())))
        }

        Cmd::ToggleAltSpeed => c
            .toggle_speed_limits_mode()
            .map(|_| report_ok(shared, "已切换备用限速模式")),

        // 这几条在 run() 里已单独处理，不会走到这里
        Cmd::Connect(_)
        | Cmd::Disconnect
        | Cmd::UpdateRefresh(_)
        | Cmd::RefreshNow
        | Cmd::FetchDetail(_)
        | Cmd::Shutdown => Ok(()),
    };

    if let Err(e) = r {
        if is_auth_error(&e) {
            if c.login().is_ok() {
                report_ok(shared, "会话已刷新，请重试刚才的操作");
                return;
            }
            set(shared, |s| {
                s.conn = Conn::Error("会话失效".into());
            });
        }
        report_err(shared, "操作失败", e);
    }
}
