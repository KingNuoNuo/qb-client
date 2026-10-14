@echo off
chcp 65001 >nul
setlocal

echo ============================================
echo   qb-client 构建脚本
echo ============================================
echo.

where cargo >nul 2>nul
if errorlevel 1 (
    echo [错误] 未检测到 cargo。
    echo 请先安装 Rust: https://rustup.rs
    echo.
    pause
    exit /b 1
)

echo 正在编译 release 版本，首次需要 3-8 分钟...
echo.
cargo build --release
if errorlevel 1 (
    echo.
    echo [失败] 编译出错，请看上面的错误信息。
    pause
    exit /b 1
)

echo.
echo [成功] 产物: %CD%\target\release\qb-client.exe
echo.
pause
