//! 构建脚本：Windows 下把图标编进 exe 的资源段。
//!
//! 只在 Windows 主机上跑 —— `embed-resource` 底层要调 MSVC 的 `rc.exe`
//! （GNU 工具链则是 `windres`），macOS / Linux 的 CI 里没有这些工具，
//! 硬跑会直接失败。对应地，`Cargo.toml` 里 `embed-resource` 也只挂在
//! `[target.'cfg(windows)'.build-dependencies]` 下，非 Windows 目标根本不拉取。
fn main() {
    #[cfg(windows)]
    {
        embed_resource::compile("assets/app.rc", embed_resource::NONE);
    }
}
