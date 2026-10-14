//! qBittorrent WebAPI v2 客户端封装。
//!
//! 覆盖核心管理所需的端点，并统一处理「会话失效」这一情况
//! （qB 在 WebUI 重启 / 超时后会丢弃 SID，此时所有请求返回 403）。

// 部分端点暂时没接 UI，保留以便后续扩展。
#![allow(dead_code)]

use anyhow::{anyhow, Result};
use reqwest::blocking::{multipart, Client, Response};
use std::collections::HashMap;
use std::time::Duration;

use crate::i18n::{tf, tr};
use crate::model::{
    CategoryMap, Peer, Torrent, TorrentFile, TorrentProperties, Tracker, TransferInfo, WebSeed,
};

/// 会话失效标记。上层 worker 捕获后会重新登录并重试一次。
#[derive(Debug)]
pub struct AuthExpired;

impl std::fmt::Display for AuthExpired {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", tr("会话已失效，需要重新登录"))
    }
}
impl std::error::Error for AuthExpired {}

pub fn is_auth_error(e: &anyhow::Error) -> bool {
    e.downcast_ref::<AuthExpired>().is_some()
}

/// 添加任务时的可选参数。
#[derive(Debug, Clone, Default)]
pub struct AddOptions {
    /// 保存路径，空串 = 用默认
    pub savepath: String,
    /// 分类，空串 = 不分类
    pub category: String,
    /// 逗号分隔的标签
    pub tags: String,
    /// 添加后暂停
    pub paused: bool,
    /// 顺序下载
    pub sequential: bool,
}

pub struct QbClient {
    http: Client,
    base: String,
    username: String,
    password: String,
}

impl QbClient {
    pub fn new(base_url: &str, username: &str, password: &str, ignore_cert: bool) -> Result<Self> {
        let http = Client::builder()
            .timeout(Duration::from_secs(20))
            .connect_timeout(Duration::from_secs(8))
            .danger_accept_invalid_certs(ignore_cert)
            .cookie_store(true)
            .user_agent("qb-client/0.1 (rust)")
            .build()?;

        Ok(Self {
            http,
            base: base_url.trim_end_matches('/').to_string(),
            username: username.to_string(),
            password: password.to_string(),
        })
    }

    pub fn base(&self) -> &str {
        &self.base
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base, path)
    }

    fn ensure_ok(r: &Response) -> Result<()> {
        let st = r.status();
        if st.is_success() {
            return Ok(());
        }
        if st == reqwest::StatusCode::FORBIDDEN || st == reqwest::StatusCode::UNAUTHORIZED {
            return Err(anyhow::Error::new(AuthExpired));
        }
        Err(anyhow!("{}", tf(tr("服务器返回 HTTP {}"), &[&st.as_u16()])))
    }

    // ---------- 认证 ----------

    /// `POST /api/v2/auth/login`。成功时 body 为 `Ok.`，凭据错误为 `Fails.`。
    pub fn login(&self) -> Result<()> {
        let r = self
            .http
            .post(self.url("/api/v2/auth/login"))
            .form(&[
                ("username", self.username.as_str()),
                ("password", self.password.as_str()),
            ])
            .send()?;

        if r.status() == reqwest::StatusCode::FORBIDDEN {
            return Err(anyhow!(
                tr("登录被拒绝：用户名或密码错误，或该 IP 已被临时封禁（qB 默认失败 5 次封 1 小时）")
            ));
        }
        if !r.status().is_success() {
            return Err(anyhow!("{}", tf(tr("登录请求失败：HTTP {}"), &[&r.status().as_u16()])));
        }

        let body = r.text().unwrap_or_default();
        let body = body.trim();
        if body != "Ok." {
            return Err(anyhow!(
                "{}",
                tf(tr("登录失败（返回 \"{}\"）：请检查用户名 / 密码"), &[&body])
            ));
        }
        Ok(())
    }

    pub fn logout(&self) -> Result<()> {
        let _ = self.http.post(self.url("/api/v2/auth/logout")).send();
        Ok(())
    }

    // ---------- 应用信息 ----------

    /// 返回形如 `v5.1.0`
    pub fn app_version(&self) -> Result<String> {
        let r = self.http.get(self.url("/api/v2/app/version")).send()?;
        Self::ensure_ok(&r)?;
        Ok(r.text()?.trim().to_string())
    }

    /// 返回形如 `2.11.4`
    pub fn webapi_version(&self) -> Result<String> {
        let r = self.http.get(self.url("/api/v2/app/webapiVersion")).send()?;
        Self::ensure_ok(&r)?;
        Ok(r.text()?.trim().to_string())
    }

    /// `GET /api/v2/app/preferences`（原样 JSON）
    pub fn preferences(&self) -> Result<serde_json::Value> {
        let r = self.http.get(self.url("/api/v2/app/preferences")).send()?;
        Self::ensure_ok(&r)?;
        Ok(r.json()?)
    }

    // ---------- 传输 / 任务 ----------

    pub fn torrents(&self) -> Result<Vec<Torrent>> {
        let r = self.http.get(self.url("/api/v2/torrents/info")).send()?;
        Self::ensure_ok(&r)?;
        Ok(r.json()?)
    }

    pub fn transfer_info(&self) -> Result<TransferInfo> {
        let r = self.http.get(self.url("/api/v2/transfer/info")).send()?;
        Self::ensure_ok(&r)?;
        Ok(r.json()?)
    }

    pub fn categories(&self) -> Result<CategoryMap> {
        let r = self.http.get(self.url("/api/v2/torrents/categories")).send()?;
        Self::ensure_ok(&r)?;
        Ok(r.json()?)
    }

    pub fn tags(&self) -> Result<Vec<String>> {
        let r = self.http.get(self.url("/api/v2/torrents/tags")).send()?;
        Self::ensure_ok(&r)?;
        Ok(r.json()?)
    }

    pub fn trackers(&self, hash: &str) -> Result<Vec<Tracker>> {
        let r = self
            .http
            .get(self.url("/api/v2/torrents/trackers"))
            .query(&[("hash", hash)])
            .send()?;
        Self::ensure_ok(&r)?;
        Ok(r.json()?)
    }

    // ---------- 任务详情（原生 WebUI 的五个模块） ----------

    /// `GET /api/v2/torrents/properties`
    pub fn properties(&self, hash: &str) -> Result<TorrentProperties> {
        let r = self
            .http
            .get(self.url("/api/v2/torrents/properties"))
            .query(&[("hash", hash)])
            .send()?;
        Self::ensure_ok(&r)?;
        Ok(r.json()?)
    }

    /// `GET /api/v2/torrents/files`（内容）
    pub fn torrent_files(&self, hash: &str) -> Result<Vec<TorrentFile>> {
        let r = self
            .http
            .get(self.url("/api/v2/torrents/files"))
            .query(&[("hash", hash)])
            .send()?;
        Self::ensure_ok(&r)?;
        Ok(r.json()?)
    }

    /// `GET /api/v2/torrents/webseeds`（HTTP 源）
    pub fn webseeds(&self, hash: &str) -> Result<Vec<WebSeed>> {
        let r = self
            .http
            .get(self.url("/api/v2/torrents/webseeds"))
            .query(&[("hash", hash)])
            .send()?;
        Self::ensure_ok(&r)?;
        Ok(r.json()?)
    }

    /// `GET /api/v2/sync/torrentPeers`（节点）。
    ///
    /// ⚠️ **路径在 `sync` 命名空间下**，不是 `torrents`。
    /// `/api/v2/torrents/peers` 这个端点**根本不存在** ——
    /// 在 qB 5.1.0 上实测直接返回 404（`Not Found`）。
    /// qB 的文档里 peers 一直挂在 sync 下：`/api/v2/sync/torrentPeers`。
    ///
    /// `rid=0` 表示要全量快照；`rid` 非 0 时 qB 只回来自那次请求之后的**增量**，
    /// 我们不做增量合并，所以恒定传 0。
    ///
    /// 返回形状：`{"full_update":bool,"peers":{"ip:port":{...}},"rid":N,"show_flags":bool}`
    /// —— 以 `ip:port` 为键的对象，而不是数组。
    ///
    /// 容错两处历史差异：
    ///   * 4.1 以前直接返回数组，没有 `peers` 外壳；
    ///   * 个别版本的条目里没带 `ip` / `port`，只能从键里拆。
    pub fn peers(&self, hash: &str) -> Result<Vec<Peer>> {
        let r = self
            .http
            .get(self.url("/api/v2/sync/torrentPeers"))
            .query(&[("hash", hash), ("rid", "0")])
            .send()?;
        Self::ensure_ok(&r)?;

        let v: serde_json::Value = r.json()?;
        let mut out: Vec<Peer> = Vec::new();

        let mut push = |key: Option<&str>, val: &serde_json::Value| {
            let mut p: Peer = serde_json::from_value(val.clone()).unwrap_or_default();
            if let Some(k) = key {
                if p.ip.is_empty() || p.port == 0 {
                    // IPv6 的键是 `[::1]:6881`，所以从**最后一个**冒号切
                    if let Some((host, port)) = k.rsplit_once(':') {
                        if p.ip.is_empty() {
                            p.ip = host.trim_start_matches('[').trim_end_matches(']').to_string();
                        }
                        if p.port == 0 {
                            p.port = port.parse().unwrap_or(0);
                        }
                    }
                }
            }
            out.push(p);
        };

        if let Some(map) = v.get("peers").and_then(|x| x.as_object()) {
            for (k, pv) in map {
                push(Some(k.as_str()), pv);
            }
        } else if let Some(arr) = v.as_array() {
            for pv in arr {
                push(None, pv);
            }
        }

        // 相关度高的排前面；同档再按下载速度。
        // （JSON 对象到这里是无序的，不排一次每次刷新顺序都不一样。）
        out.sort_by(|a, b| {
            b.relevance
                .partial_cmp(&a.relevance)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(b.dl_speed.cmp(&a.dl_speed))
                .then(b.up_speed.cmp(&a.up_speed))
                .then_with(|| a.ip.cmp(&b.ip))
        });
        Ok(out)
    }

    // ---------- 添加 ----------

    /// 通过 magnet / URL 添加。
    pub fn add_urls(&self, urls: &str, o: &AddOptions) -> Result<()> {
        let mut form: Vec<(String, String)> = vec![("urls".into(), urls.to_string())];
        push_common_opts(&mut form, o);

        let r = self
            .http
            .post(self.url("/api/v2/torrents/add"))
            .form(&form)
            .send()?;
        Self::ensure_ok(&r)?;
        Ok(())
    }

    /// 通过上传 `.torrent` 文件添加。
    pub fn add_file(&self, filename: &str, data: Vec<u8>, o: &AddOptions) -> Result<()> {
        let part = multipart::Part::bytes(data)
            .file_name(filename.to_string())
            .mime_str("application/x-bittorrent")?;

        let mut form = multipart::Form::new().part("torrents", part);
        if !o.savepath.trim().is_empty() {
            form = form.text("savepath", o.savepath.clone());
        }
        if !o.category.trim().is_empty() {
            form = form.text("category", o.category.clone());
        }
        if !o.tags.trim().is_empty() {
            form = form.text("tags", o.tags.clone());
        }
        form = form.text("paused", bool_str(o.paused));
        form = form.text("sequentialDownload", bool_str(o.sequential));

        let r = self
            .http
            .post(self.url("/api/v2/torrents/add"))
            .multipart(form)
            .send()?;
        Self::ensure_ok(&r)?;
        Ok(())
    }

    // ---------- 批量操作（hashes 支持传 `all`） ----------

    /// 批量操作公共路径。
    ///
    /// **空集合直接短路**：qB 对空 `hashes` 的处理在不同版本里不一致
    /// （4.x 会返回 400，5.x 部分端点会静默成功），与其赌它的行为，
    /// 不如在客户端就把「什么都没选」这件事当作 no-op。
    fn post_hashes(&self, path: &str, hashes: &[String]) -> Result<()> {
        if hashes.is_empty() {
            return Ok(());
        }
        let joined = hashes.join("|");
        let r = self
            .http
            .post(self.url(path))
            .form(&[("hashes", joined)])
            .send()?;
        Self::ensure_ok(&r)?;
        Ok(())
    }

    /// 提交一个普通表单（键值对顺序即提交顺序）。
    fn post_kv(&self, path: &str, fields: Vec<(&str, String)>) -> Result<()> {
        let r = self.http.post(self.url(path)).form(&fields).send()?;
        Self::ensure_ok(&r)?;
        Ok(())
    }

    /// 暂停 / 恢复。
    ///
    /// ⚠️ 端点在 qB 5.x 变过名：5.0 把 `pause`/`resume` 改成 `stop`/`start`，
    /// 当时旧的还留着作兼容别名；**5.1 把别名删了** —— 直接打
    /// `/torrents/pause` 在 5.1.0 上实测返回 404，暂停就这么失效的。
    ///
    /// 所以这里**新端点优先、404 回退旧端点**：5.x 走 stop/start，
    /// 4.x 第一发 404 后落到 pause/resume。不用先探测版本，一发请求
    /// 就把版本差异消化掉了。
    pub fn pause(&self, hashes: &[String]) -> Result<()> {
        self.post_hashes_compat("/api/v2/torrents/stop", "/api/v2/torrents/pause", hashes)
    }

    pub fn resume(&self, hashes: &[String]) -> Result<()> {
        self.post_hashes_compat("/api/v2/torrents/start", "/api/v2/torrents/resume", hashes)
    }

    /// 同 `post_hashes`，但允许在 404 时换一条路径重试。
    ///
    /// 判 404 用的是错误信息里的 "HTTP 404" —— `ensure_ok` 的报错是我们
    /// 自己拼的固定格式，不依赖 qB 的响应体，所以这个匹配是稳的。
    fn post_hashes_compat(&self, new_path: &str, old_path: &str, hashes: &[String]) -> Result<()> {
        if hashes.is_empty() {
            return Ok(());
        }
        if let Err(e) = self.post_hashes(new_path, hashes) {
            if e.to_string().contains("HTTP 404") {
                return self.post_hashes(old_path, hashes);
            }
            return Err(e);
        }
        Ok(())
    }

    /// 批量设置**文件级**下载优先级（`POST /torrents/filePrio`）。
    ///
    /// ⚠️ 参数名是 **`id`**（不是 `ids`），多个下标用 **`|`** 分隔 ——
    /// 用 `ids=0,1` 会直接 400（实测）。
    ///
    /// `ids` 是文件在 `/torrents/files` 返回数组里的**下标**（0 起）——
    /// qB 没有单独的文件 id，就是按数组顺序对应的。
    /// priority 按 WebAPI 文档：0 = 不下载，1 = 普通，6 = 高，7 = 最高。
    pub fn set_file_priority(&self, hash: &str, ids: &[usize], priority: i32) -> Result<()> {
        if ids.is_empty() {
            return Ok(());
        }
        let ids = ids
            .iter()
            .map(|i| i.to_string())
            .collect::<Vec<_>>()
            .join("|");
        self.post_kv(
            "/api/v2/torrents/filePrio",
            vec![
                ("hash", hash.to_string()),
                ("id", ids),
                ("priority", priority.to_string()),
            ],
        )
    }

    pub fn delete(&self, hashes: &[String], delete_files: bool) -> Result<()> {
        if hashes.is_empty() {
            return Ok(());
        }
        let r = self
            .http
            .post(self.url("/api/v2/torrents/delete"))
            .form(&[
                ("hashes", hashes.join("|")),
                ("deleteFiles", bool_str(delete_files)),
            ])
            .send()?;
        Self::ensure_ok(&r)?;
        Ok(())
    }

    pub fn recheck(&self, hashes: &[String]) -> Result<()> {
        self.post_hashes("/api/v2/torrents/recheck", hashes)
    }

    pub fn reannounce(&self, hashes: &[String]) -> Result<()> {
        self.post_hashes("/api/v2/torrents/reannounce", hashes)
    }

    pub fn set_force_start(&self, hashes: &[String], value: bool) -> Result<()> {
        let Some(h) = join_hashes(hashes) else {
            return Ok(());
        };
        self.post_kv(
            "/api/v2/torrents/setForceStart",
            vec![("hashes", h), ("value", bool_str(value))],
        )
    }

    pub fn set_sequential(&self, hashes: &[String], value: bool) -> Result<()> {
        let Some(h) = join_hashes(hashes) else {
            return Ok(());
        };
        self.post_kv(
            "/api/v2/torrents/setSequentialDownload",
            vec![("hashes", h), ("value", bool_str(value))],
        )
    }

    /// 移动任务的保存路径。空路径 qB 会返回 400，这里提前拦掉。
    pub fn set_location(&self, hashes: &[String], path: &str) -> Result<()> {
        let Some(h) = join_hashes(hashes) else {
            return Ok(());
        };
        if path.trim().is_empty() {
            return Err(anyhow!("{}", tr("目标路径不能为空")));
        }
        self.post_kv(
            "/api/v2/torrents/setLocation",
            vec![("hashes", h), ("location", path.to_string())],
        )
    }

    // ---------- 分类 / 标签 ----------

    pub fn set_category(&self, hashes: &[String], category: &str) -> Result<()> {
        let Some(h) = join_hashes(hashes) else {
            return Ok(());
        };
        self.post_kv(
            "/api/v2/torrents/setCategory",
            vec![("hashes", h), ("category", category.to_string())],
        )
    }

    pub fn create_category(&self, name: &str, save_path: &str) -> Result<()> {
        if name.trim().is_empty() {
            return Err(anyhow!("{}", tr("分类名不能为空")));
        }
        self.post_kv(
            "/api/v2/torrents/createCategory",
            vec![
                ("category", name.to_string()),
                ("savePath", save_path.to_string()),
            ],
        )
    }

    /// 注意 qB 这个端点要求多个分类名用**换行**分隔（不是 `|`）。
    pub fn remove_categories(&self, names: &[String]) -> Result<()> {
        if names.is_empty() {
            return Ok(());
        }
        self.post_kv(
            "/api/v2/torrents/removeCategories",
            vec![("categories", names.join("\n"))],
        )
    }

    /// 注意 qB 这个端点要求多个标签名用**逗号**分隔。
    pub fn create_tags(&self, tags: &str) -> Result<()> {
        if tags.trim().is_empty() {
            return Ok(());
        }
        self.post_kv("/api/v2/torrents/createTags", vec![("tags", tags.to_string())])
    }

    pub fn add_tags(&self, hashes: &[String], tags: &str) -> Result<()> {
        let Some(h) = join_hashes(hashes) else {
            return Ok(());
        };
        if tags.trim().is_empty() {
            return Ok(());
        }
        self.post_kv(
            "/api/v2/torrents/addTags",
            vec![("hashes", h), ("tags", tags.to_string())],
        )
    }

    pub fn remove_tags(&self, hashes: &[String], tags: &str) -> Result<()> {
        let Some(h) = join_hashes(hashes) else {
            return Ok(());
        };
        if tags.trim().is_empty() {
            return Ok(());
        }
        self.post_kv(
            "/api/v2/torrents/removeTags",
            vec![("hashes", h), ("tags", tags.to_string())],
        )
    }

    // ---------- 限速 ----------

    /// 备用限速是否开启
    pub fn speed_limits_mode(&self) -> Result<bool> {
        let r = self
            .http
            .get(self.url("/api/v2/transfer/speedLimitsMode"))
            .send()?;
        Self::ensure_ok(&r)?;
        Ok(r.text()?.trim() == "1")
    }

    pub fn toggle_speed_limits_mode(&self) -> Result<()> {
        let r = self
            .http
            .post(self.url("/api/v2/transfer/toggleSpeedLimitsMode"))
            .send()?;
        Self::ensure_ok(&r)?;
        Ok(())
    }

    pub fn global_download_limit(&self) -> Result<i64> {
        let r = self
            .http
            .get(self.url("/api/v2/transfer/downloadLimit"))
            .send()?;
        Self::ensure_ok(&r)?;
        Ok(r.text()?.trim().parse().unwrap_or(0))
    }

    pub fn global_upload_limit(&self) -> Result<i64> {
        let r = self
            .http
            .get(self.url("/api/v2/transfer/uploadLimit"))
            .send()?;
        Self::ensure_ok(&r)?;
        Ok(r.text()?.trim().parse().unwrap_or(0))
    }

    /// limit 单位为 bytes/s，0 = 不限
    pub fn set_global_download_limit(&self, limit: i64) -> Result<()> {
        let r = self
            .http
            .post(self.url("/api/v2/transfer/setDownloadLimit"))
            .form(&[("limit", limit.to_string())])
            .send()?;
        Self::ensure_ok(&r)?;
        Ok(())
    }

    pub fn set_global_upload_limit(&self, limit: i64) -> Result<()> {
        let r = self
            .http
            .post(self.url("/api/v2/transfer/setUploadLimit"))
            .form(&[("limit", limit.to_string())])
            .send()?;
        Self::ensure_ok(&r)?;
        Ok(())
    }

    /// 单任务下载限速。`-1` = 跟随全局，`0` = 该任务不限，`>0` = bytes/s。
    pub fn set_torrent_download_limit(&self, hashes: &[String], limit: i64) -> Result<()> {
        let Some(h) = join_hashes(hashes) else {
            return Ok(());
        };
        self.post_kv(
            "/api/v2/torrents/setDownloadLimit",
            vec![("hashes", h), ("limit", limit.to_string())],
        )
    }

    pub fn set_torrent_upload_limit(&self, hashes: &[String], limit: i64) -> Result<()> {
        let Some(h) = join_hashes(hashes) else {
            return Ok(());
        };
        self.post_kv(
            "/api/v2/torrents/setUploadLimit",
            vec![("hashes", h), ("limit", limit.to_string())],
        )
    }
}

/// 把 hashes 拼成 qB 要的 `a|b|c`。空集合返回 `None`，调用方直接短路。
fn join_hashes(hashes: &[String]) -> Option<String> {
    if hashes.is_empty() {
        None
    } else {
        Some(hashes.join("|"))
    }
}

fn bool_str(b: bool) -> String {
    if b { "true" } else { "false" }.to_string()
}

fn push_common_opts(form: &mut Vec<(String, String)>, o: &AddOptions) {
    if !o.savepath.trim().is_empty() {
        form.push(("savepath".into(), o.savepath.clone()));
    }
    if !o.category.trim().is_empty() {
        form.push(("category".into(), o.category.clone()));
    }
    if !o.tags.trim().is_empty() {
        form.push(("tags".into(), o.tags.clone()));
    }
    form.push(("paused".into(), bool_str(o.paused)));
    form.push(("sequentialDownload".into(), bool_str(o.sequential)));
}

/// 从响应里安全取 JSON（供调试使用）
#[allow(dead_code)]
pub fn json_of(v: &serde_json::Value) -> HashMap<String, serde_json::Value> {
    v.as_object()
        .map(|m| m.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
        .unwrap_or_default()
}
