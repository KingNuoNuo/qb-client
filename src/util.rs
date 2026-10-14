//! 显示层的小工具函数。

#![allow(dead_code)]

use chrono::{Local, TimeZone};

/// 字节数 → 人类可读（KiB / MiB / GiB …）
pub fn fmt_size(bytes: i64) -> String {
    let b = bytes.max(0) as f64;
    const UNITS: [&str; 6] = ["B", "KiB", "MiB", "GiB", "TiB", "PiB"];
    let mut v = b;
    let mut i = 0usize;
    while v >= 1024.0 && i < UNITS.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{:.0} {}", v, UNITS[i])
    } else if v >= 100.0 {
        format!("{:.0} {}", v, UNITS[i])
    } else {
        format!("{:.2} {}", v, UNITS[i])
    }
}

/// B/s → 人类可读速度
pub fn fmt_speed(bps: i64) -> String {
    if bps <= 0 {
        "0 B/s".to_string()
    } else {
        format!("{}/s", fmt_size(bps))
    }
}

/// 剩余时间。qB 用 8640000 秒（100 天）表示「无限」。
pub fn fmt_eta(secs: i64) -> String {
    if secs < 0 || secs >= 8640000 {
        return "∞".to_string();
    }
    let d = secs / 86400;
    let h = (secs % 86400) / 3600;
    let m = (secs % 3600) / 60;
    let s = secs % 60;
    if d > 0 {
        format!("{}天{}时", d, h)
    } else if h > 0 {
        format!("{}:{:02}:{:02}", h, m, s)
    } else {
        format!("{}:{:02}", m, s)
    }
}

/// 秒数 → 时长，如 `3天2小时`，用于「活动时间」
pub fn fmt_duration(secs: i64) -> String {
    if secs <= 0 {
        return "-".to_string();
    }
    let d = secs / 86400;
    let h = (secs % 86400) / 3600;
    let m = (secs % 3600) / 60;
    if d > 0 {
        format!("{}天{}时", d, h)
    } else if h > 0 {
        format!("{}时{}分", h, m)
    } else {
        format!("{}分", m)
    }
}

pub fn fmt_ratio(r: f64) -> String {
    if r.is_finite() {
        format!("{:.2}", r)
    } else {
        "∞".to_string()
    }
}

/// UNIX 秒 → 本地时间 `YYYY-MM-DD HH:MM`
pub fn fmt_ts(secs: i64) -> String {
    if secs <= 0 {
        return "-".to_string();
    }
    Local
        .timestamp_opt(secs, 0)
        .single()
        .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_else(|| "-".to_string())
}

/// 把「KiB/s」输入框的值转成 qB 要的 bytes/s。0 或负数 → qB 语义的「不限速」。
/// 用于**全局限速**（全局没有「跟随」这一态，0 就是不限）。
pub fn kib_to_bytes(kib: i64) -> i64 {
    if kib <= 0 {
        0
    } else {
        kib.saturating_mul(1024)
    }
}

/// 单任务限速专用：**负数原样保留**。
///
/// qB 的单任务限速是三态的：`-1` = 跟随全局、`0` = 该任务不限、`>0` = 独立限速。
/// 所以这里不能用 [`kib_to_bytes`] —— 它会把 `-1` 压成 `0`，
/// 「跟随全局」就会被悄悄改成「不限」，这是一个静默的语义错误。
pub fn kib_to_bytes_pm(kib: i64) -> i64 {
    if kib < 0 {
        -1
    } else {
        kib.saturating_mul(1024)
    }
}

/// bytes/s → KiB/s（用于回填输入框）
pub fn bytes_to_kib(bps: i64) -> i64 {
    if bps <= 0 {
        0
    } else {
        bps / 1024
    }
}
