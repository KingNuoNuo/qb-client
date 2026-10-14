# qb-client

[![build](https://github.com/KingNuoNuo/qb-client/actions/workflows/build.yml/badge.svg)](https://github.com/KingNuoNuo/qb-client/actions/workflows/build.yml)

[简体中文](README.md) | **English**

A **qBittorrent desktop client** written in Rust + egui (Windows / macOS). It talks directly to the
qBittorrent WebAPI v2 and is compatible with **qBittorrent 5.x** (including 5.1.0 — the rename of the
`paused*` states to `stopped*` in 5.0 is handled in code).

The output is a **single executable** (one `.exe` on Windows, one `.app` on macOS) with no WebView /
Node / runtime dependency. The fonts (~800 KB) are embedded into the binary.

---

## Screenshots

> Names and other private details are redacted in these screenshots.

**Dark**

![Dark theme](docs/screenshot-dark.png)

**Light**

![Light theme](docs/screenshot-light.png)

---

## Interface design

The visual direction is **industrial / utilitarian** — dashboard density, monospaced numbers, hairline
borders — deliberately avoiding the "generic admin template" look of rounded cards and evenly
distributed color.

| Layer | Decision |
|---|---|
| **Display face** | Archivo Black (industrial grotesque), used only for the brand wordmark |
| **UI body** | Barlow (DIN-family skeleton, with real R / M / SB weights) |
| **Numerals** | IBM Plex Mono (SemiBold / Bold) — every value in the tables is tabular-aligned |
| **CJK** | Not embedded; loaded at runtime from the system font directory via a per-platform candidate list (Microsoft YaHei on Windows / PingFang on macOS / Noto CJK on Linux), to avoid bloating the exe by a dozen MB |
| **Base color** | Warm near-black `#0C0D0F` — neither pure black nor blue-black; the light theme uses a warm-grey canvas `#EDEAE3` with off-white panels `#F8F6F2` |
| **Accent** | Amber `#FF9F1C` (switched to `#B45309` in the light theme to keep contrast ≥ 4.5:1), **only one on screen**, reserved for selection and the primary action |
| **State colors** | Download cyan `#3FC1FF` / upload green `#4ADE80` / checking amber `#F5A524` / error red `#FF5C5C` — an independent semantic layer |
| **Corner radius** | Three steps: 2 / 4 / 6 pt. The key to the industrial feel is hairlines and right angles, not roundness |

The **ambient layer** (`widgets::backdrop`) is painted on the `Order::Background` layer while the central
panel stays transparent, so the ambience shows through from underneath the content. The content has only
two layers: a **vertical gradient floor plus an amber radial glow in the top-left corner** (simulated with
a triangle fan — epaint has no radial-gradient primitive).

In the light theme the **glow is deliberately disabled** (token set fully transparent). Amber over a cream
canvas just turns into a smear of dirty yellow, warming both the canvas and the row backgrounds and making
the canvas brighter than the panels, which flattens the "panels float above the canvas" hierarchy. The light
theme builds depth from surface layering instead (canvas → panel → white).

> **No textures in the background.** (There used to be a 48 pt hairline grid; after tuning its strength for
> both themes it was **retired for good**.) In a data-dense interface a background pattern inevitably
> competes with the content for attention: dialed down until invisible it may as well not exist, and dialed
> up until visible it becomes graph paper — there is no "just right" notch in between. Express texture with
> spacing, hairlines, and font weight instead.

**Accessibility**: interactive controls (buttons / inputs / dropdowns / checkboxes) all use egui's built-in
implementations and only swap the visuals, so keyboard focus, Tab traversal, and focus rings are native.
Only purely presentational elements (telemetry strip, progress bars, status dots, section headers, nav bar)
are custom-drawn; the custom nav bar additionally gets a `Sense::click` focus chain, Enter/Space activation,
and a 2 pt focus ring. Every dot, arrow, plus, and warning triangle is drawn as geometry by the painter
rather than relying on `● ↑ ↓ ＋ ⚠` glyphs — those characters are missing from some fonts and are the most
common source of "tofu" boxes in a CJK interface.

**Motion**: 120 ms row hover, 130 ms nav selection, 320 ms smooth interpolation for progress values (a jump
in progress "flashes"; smoothing it lets you read the speed difference directly). All via `ctx.animate_*`;
theme and durations can be changed in one place, `theme.rs`.

---

## Features

| Module | Contents |
|---|---|
| Connection | Host / port / username / password / HTTPS / ignore certificate; auto-connect on launch; persisted config |
| Torrent list | Name, size, progress bar, status, up/down speed, ratio, ETA, category, tags; click headers to sort; keyword search |
| Filters | Status (all / downloading / seeding / completed / stopped / errored / active), category, tag, with live counts in the left sidebar |
| Add | Batch submit of multiline magnets / URLs; upload local `.torrent` files; custom save path, category, tags, add-paused, sequential download |
| Control | Pause / resume / delete (optionally with data) / force start / sequential download / recheck / reannounce to trackers / set location |
| Rate limits | Global up/down limits, per-torrent limits, alternative rate-limit mode toggle |
| Categories & tags | Create categories, bulk change category, bulk add/remove tags (also per-torrent from the context menu) |
| Other | Torrent details (five tabs: General / Trackers / Peers / HTTP sources / Content, columns aligned with the native WebUI), operation log, light & dark themes, live telemetry strip in the top bar, remembered theme and window size |
| Languages | Simplified Chinese / English, **following the qBittorrent WebUI language setting** — the client reads `locale` from `/app/preferences` after connecting and switches automatically; changing the language in the WebUI takes effect within ~15s. Untranslated strings fall back to Chinese. States, priorities, peer flags, the operation log and error messages are all translated |
| Display adaptation | All coordinates in logical points, following system DPI scaling (100%–200%) without blur or distortion; window size is automatically clamped into the usable area across monitors; interface zoom is an additional independent layer (Settings → Display, or Ctrl + / − / 0) |

---

## Download a prebuilt binary

| Platform | Artifact | Notes |
|---|---|---|
| Windows x64 | `qb-client-windows-x64.exe` | Single file, double-click to run |
| macOS Apple Silicon | `qb-client-macos-arm64.zip` | Unzip to get `qb-client.app`, drag into Applications |
| macOS Intel | `qb-client-macos-x86_64.zip` | Same as above |

- **Every push to `main` builds all three targets automatically**; artifacts are in the Actions run.
- Pushing a `v*` tag (`git tag v0.1.0 && git push --tags`) additionally **attaches those artifacts to the
  GitHub Release**.
- The macOS build is unsigned — see the Gatekeeper note under "Known limitations".

---

## Build

### 1. Install Rust

Download `rustup-init.exe` from <https://rustup.rs> and install with the defaults (this also installs the
MSVC toolchain). If it complains about missing C++ build tools, install **Visual Studio Build Tools**
(check "Desktop development with C++").

Verify:

```bash
cargo --version
```

### 2. In mainland China: configure a crates mirror (optional but strongly recommended)

Edit `C:\Users\<you>\.cargo\config.toml` (create it if it doesn't exist):

```toml
[source.crates-io]
replace-with = "aliyun"

[source.aliyun]
registry = "sparse+https://mirrors.aliyun.com/crates.io-index/"
```

> Alternatives: `sparse+https://rsproxy.cn/index/` (ByteDance), `sparse+https://mirrors.ustc.edu.cn/crates.io-index/`.
> Probe whichever you pick first with `curl -s -o /dev/null -w "%{http_code}" -m 12 <URL>` and use it if it returns 200.

### 3. Compile

```bash
cd D:/workbuddy/qb/qb-client
cargo build --release
```

The first build pulls dependencies and compiles the whole dependency tree. With a mirror configured it
measured **about 100 seconds** (the egui family is 400+ crates); without a mirror, a direct connection to
crates.io can take 3–8 minutes. Incremental builds afterwards take **around 30 seconds**.

Artifact: `target\release\qb-client.exe` (about 8–12 MB, already stripped).

You can also just double-click `build.bat`.

### Verification record

- **Compile**: `cargo build --release` (rustc 1.99.0 / cargo 1.99.0 / Windows MSVC) passed with **zero
  errors and zero warnings**; the artifact `qb-client.exe` is **10.6 MB** (`strip` + thin LTO).
- **Run**: connected to qBittorrent **5.1.0** (WebAPI 2.11.6) and read all 12 torrents correctly; the table,
  filters, telemetry strip, status bar, and light/dark themes all render correctly — see the screenshots above.
- **Verification of the background-texture removal**: a pixel-by-pixel scan of the same window rectangle — before
  removal it picked up three vertical lines at x=2026 / 2098 / 2170 (spacing 72 px = 48 pt at 150% scaling);
  after removal the same scan line reported `jumps=0`, and the four horizontal lines in the vertical scan were
  gone as well.

---

## Usage

1. Run `qb-client.exe` (on macOS, `qb-client.app`).
2. Click "Connection Settings" in the top-right and fill in the qBittorrent WebUI address, port, username,
   and password.
3. Click "Save & Connect". Once connected, the top-left shows `Connected qBittorrent v5.1.0 · WebAPI 2.11.x`.
4. The config lives at `%APPDATA%\qb-client\config.json` (**the password is stored in plain text** — keep it safe).

### Tips

- **Double-click a torrent row** to open its details; **right-click a row** for the quick-action menu (set
  category / add-remove tags / set location / copy hash).
- **Selection**: a single click on a row selects only that row; clicking the checkbox, holding Ctrl (⌘),
  holding Shift, or pressing Enter toggles that row's selection state. With several rows selected, use the
  "Pause / Resume / Delete" buttons in the toolbar, or the bulk bar that floats up on the second row to set
  category / tags.
- The right side of the top bar is the telemetry strip: peer count, torrent count (with active count), and
  live up/down speed. The thin bar under each block is a "reading bar" — if a rate limit is set it shows
  utilization; otherwise it is scaled against 20 MiB/s (down) / 10 MiB/s (up).
- Press **Esc** in the search box to clear the search immediately.
- **The light/dark theme and window size are remembered** and restored on the next launch (stored in eframe's
  local config, separate from `config.json`).
- The polling interval defaults to 2 seconds and is adjustable in Settings (1–60 s). After changing it, click
  "Save only" to apply immediately — it **does not drop the session**.

---

## Performance: polling over two channels

In qB's WebAPI, the torrent list and transfer rates change every second, but **categories, tags, the
alternative rate-limit toggle, and the global limit values barely change within seconds**. Putting them in
the same polling cycle means most requests are asking a set of questions whose answers never change.

So it is split into two:

| Channel | Contents | Frequency |
|---|---|---|
| Fast | `torrents/info` + `transfer/info` | Every polling cycle (default 2 s) |
| Slow | categories / tags / alternative limit / global limits | Every 15 s, **and forced once after any write operation** |

The steady-state request volume drops from **7 per cycle to 2 per cycle** (about −70%). The forced full
refresh after a write guarantees that "you just created a category or set a tag, and it shows up on the very
next frame" without waiting those 15 seconds.

---

## Code structure

```
src/
├── main.rs     Entry point, eframe startup, window sizing
├── config.rs   Connection config read/write (%APPDATA%\qb-client\config.json)
├── theme.rs    Design system: palette + type scale + spacing/radius tokens + global style install
├── fonts.rs    Font loading: embedded Archivo Black / Barlow / IBM Plex Mono, runtime CJK fallback
├── widgets.rs  Custom-painted elements: ambient background, telemetry strip, progress bar, status dot, nav bar, row background
├── model.rs    WebAPI data structures (Torrent / TransferInfo / Tracker / Category) + status mapping
├── api.rs      WebAPI v2 wrapper (auth, torrents, rate limits, categories & tags, trackers)
├── worker.rs   Background network thread: polling refresh + serialized command execution + auto re-login on session expiry
├── app.rs      UI layout and interaction: top bar / left column / toolbar / table / status bar / dialogs
└── util.rs     Formatting (size / speed / duration / timestamps)
assets/fonts/   Three SIL OFL font families + full license text (OFL.txt)
```

Architectural key point: **the UI thread never blocks**. All HTTP requests run serially on a single
background thread in `worker.rs`; each frame the UI only takes one short lock to read a snapshot (`Snap`),
and user actions are dispatched through an `mpsc` channel.

Design key point: **no color values appear in layout code**. Every color comes from `theme::pal(self.dark)`,
so switching themes is just swapping a palette and reinstalling the style once (`theme::install`). The
palette is a `&'static Palette`, so it can be copied into closures freely without fighting `&mut self`.

`_static_check.py` is a lightweight static health-check script: it checks bracket balance, duplicate
definitions (distinguished by `impl` block), whether symbols referenced across modules exist, and whether
any function missed its palette binding. It does not need the Rust toolchain, so it is handy for a quick
scan after a change (the real check is still `cargo check`). Run it with:

```bash
python _static_check.py
```

---

## Compatibility notes

- Targets **WebAPI v2** (qBittorrent 4.1+).
- Status strings recognize both `pausedDL/pausedUP` (4.x) and `stoppedDL/stoppedUP` (5.x).
- All struct fields carry `#[serde(default)]`, so adding or removing fields in qB won't break parsing.
- The `pause` / `resume` endpoints are kept with a fallback: the newer `stop` / `start` endpoints are tried
  first and the legacy ones are used on 404, so both 4.x and 5.x work without version branching.
- If tracker field names change in a future qB version, they will simply show as empty without affecting
  other features.

## Known limitations

- Dragging a file onto the window to add a torrent is not supported (use the "Add torrent" dialog to pick a file).
- RSS subscription management, search plugins, and full Preferences editing are not supported (part of the
  "full feature set" scope, not implemented).
- In an IPv6 environment, if the qB WebUI only listens on IPv4, enter an IPv4 address.
- **The macOS artifacts are built by GitHub Actions and have not been run locally.** The UI is cross-platform
  (egui); the platform differences are concentrated in two places: the title-bar coloring only takes effect on
  Windows, and the CJK font comes from a per-platform candidate list (PingFang on macOS). If the first macOS
  run has issues, check the Actions log.
- **The macOS build is unsigned**: the first launch is blocked by Gatekeeper — right-click → Open, or run
  `xattr -cr /Applications/qb-client.app` once to allow it.
- Scrollbar thickness uses egui's defaults without fine-tuning (the `ScrollStyle` fields are unstable across
  versions, so it is left alone for now).
- The top-bar telemetry strip has a fixed width (about 460 pt total) and gets cramped below ~1100 pt;
  the main interface is designed for 1440 pt+.
- When IPv6 trackers are unreachable, the status shows "firewalled" — that is qB's own connection status,
  not a problem with this client.

---

## License

This project is open source under the [MIT License](LICENSE).
