//! 让 Windows 原生死标题栏的配色跟随应用主题。
//!
//! 为什么需要这么一层：应用切到浅色主题后，窗口最上面那条标题栏还是深色的
//! （系统"深色模式"下 Windows 给的默认值就是深色，和应用自己的主题无关），
//! 于是整个窗口最醒目的第一像素就和内容对不上。
//!
//! 为什么不走 egui：`ViewportCommand::SetTheme(SystemTheme)` 这个枚举在 egui 0.29
//! 里是**存在但没人处理**的 —— 我翻过 eframe 0.29 的原生后端，没有任何 `SetTheme`
//! 分支。所以只能自己调 DWM。
//!
//! 也不引 `windows` / `window-vibrancy` 这类 crate：只需要一个函数，
//! 手写 `extern` 声明比拖进来一棵依赖树干净。

/// 把窗口标题栏设成与 `dark` 匹配的配色。
///
/// 失败（老系统不支持、句柄失效）静默忽略 —— 这只是外观微调，
/// 不值得为它中断任何流程。
#[cfg(windows)]
pub fn apply(hwnd: isize, dark: bool) {
    // 20 是 Win10 1903 / Win11 的属性号；更老的系统用 19。
    // 两个都发一遍：发错的那个会返回错误码，没有副作用。
    const ATTR_IMMERSIVE_DARK: u32 = 20;
    const ATTR_IMMERSIVE_DARK_LEGACY: u32 = 19;

    let val: i32 = if dark { 1 } else { 0 };
    let p = &val as *const i32;
    // SAFETY: `hwnd` 由 `eframe::Frame::window_handle()` 给出，调用时窗口一定还活着；
    // `p` 指向栈上刚初始化的 i32，长度 4 与声明的 `cbAttribute` 一致。
    unsafe {
        let _ = DwmSetWindowAttribute(hwnd, ATTR_IMMERSIVE_DARK, p, 4);
        let _ = DwmSetWindowAttribute(hwnd, ATTR_IMMERSIVE_DARK_LEGACY, p, 4);
    }
}

#[cfg(windows)]
extern "system" {
    /// <https://learn.microsoft.com/windows/win32/api/dwmapi/nf-dwmapi-dwmsetwindowattribute>
    fn DwmSetWindowAttribute(hwnd: isize, attr: u32, value: *const i32, size: u32) -> i32;
}

/// 非 Windows 平台：没有原生死标题栏这件问题，空实现即可。
#[cfg(not(windows))]
pub fn apply(_hwnd: isize, _dark: bool) {}
