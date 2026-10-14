//! 连接配置：读写 `%APPDATA%\qb-client\config.json`

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// 主机名或 IP，例如 `192.168.1.10`
    pub host: String,
    /// WebUI 端口，例如 `8080`
    pub port: u16,
    pub use_https: bool,
    pub username: String,
    /// 明文保存。配置文件在本机 `%APPDATA%\qb-client\config.json`，注意保管。
    pub password: String,
    /// 忽略自签名证书（对接 https 的 WebUI 时用）
    pub ignore_cert: bool,
    /// 启动即自动连接
    pub auto_connect: bool,
    /// 轮询间隔（秒）
    pub refresh_secs: u64,
}

impl Default for Config {
    /// **只放中性占位值，不要放任何真实环境信息。**
    ///
    /// 这份默认值会进仓库、进文档，等于公开发布 —— 曾经在这里预填过
    /// 某台 NAS 的地址和密码，公开前必须清掉。真实连接参数由用户在
    /// 「连接设置」里填，落到 `%APPDATA%\qb-client\config.json`。
    fn default() -> Self {
        Self {
            host: "127.0.0.1".into(),
            // qBittorrent WebUI 的出厂默认端口
            port: 8080,
            use_https: false,
            username: "admin".into(),
            password: String::new(),
            ignore_cert: true,
            // 没填对凭据就自动连只会徒增一条报错，默认关掉
            auto_connect: false,
            refresh_secs: 2,
        }
    }
}

impl Config {
    pub fn base_url(&self) -> String {
        let scheme = if self.use_https { "https" } else { "http" };
        format!("{}://{}:{}", scheme, self.host.trim(), self.port)
    }

    pub fn config_path() -> PathBuf {
        let dir = std::env::var("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("."))
            .join("qb-client");
        dir.join("config.json")
    }

    pub fn load() -> Self {
        match std::fs::read_to_string(Self::config_path()) {
            Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    pub fn save(&self) -> Result<()> {
        let p = Self::config_path();
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&p, serde_json::to_string_pretty(self)?)?;
        Ok(())
    }
}
