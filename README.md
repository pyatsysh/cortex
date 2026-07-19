# Cortex — a lightweight, cross-platform system monitor

A vendor-neutral desktop system monitor with a premium *gaming-hub* aesthetic —
inspired by the "System Vitals" screen of the HP OMEN Gaming Hub, but not tied to
any hardware brand. Windows / Linux / macOS from one codebase.

> Status: **v0 scaffold.** The UI (Dashboard mode) is complete and previewable; the
> Rust metrics backend is written but needs its first compile pass on a machine with
> the Rust toolchain (see *Build* below).

## Why this exists

The polished system monitors are single-OS (iStat Menus / `stats` on macOS,
Rainmeter / HWiNFO on Windows, Mission Center / Conky on Linux). The cross-platform
ones are terminal apps (btop, glances) or utilitarian. The tools that *look* like
this are locked to vendor hardware (OMEN, Armoury Crate, iCUE). Cortex aims at the
gap: **cross-platform + lightweight + looks like premium gaming software + open.**

## Design goals

1. **Lightweight** — Tauri (native OS webview, no bundled Chromium). Small binary,
   low RAM. Release profile is size-optimised.
2. **Looks cool** — pixel-matched to the OMEN "System Vitals" dashboard: magenta→purple
   ring gauges, pink→orange storage bars, dark neon panels. Built with SVG + CSS.
3. **Cross-platform** — Windows, Linux, macOS via Tauri v2 + the `sysinfo` crate.
4. **Unique** — see *Why this exists*.
5. **Easy to install** — Tauri emits native installers per OS (see *Distribution*).

## Stack

| Layer | Choice |
|-------|--------|
| Shell | Tauri v2 (Rust) |
| Metrics | `sysinfo` (CPU/RAM/disk/net/processes/temps) + `nvml-wrapper` (NVIDIA GPU) |
| Frontend | Single self-contained `src/index.html` (SVG/CSS/JS), no bundler |

The frontend auto-detects its host: inside Tauri it calls the `get_metrics` command
for **live** data; opened in a plain browser it animates **demo** data (so the look
can be reviewed with zero build).

## Project layout

```
System_Monitor/
├── src/index.html          # the whole UI (Dashboard mode) — live-or-demo
├── src-tauri/
│   ├── src/main.rs         # get_metrics command: sysinfo + NVML
│   ├── Cargo.toml
│   ├── tauri.conf.json
│   ├── capabilities/       # window permissions
│   └── icons/              # generated app icons (from app-icon.png)
├── app-icon.png            # source icon; regenerate the set with `npm run icon`
└── package.json
```

## Build & run (Windows — primary target)

Prerequisites:
1. **Rust** (MSVC toolchain): install from <https://rustup.rs>.
2. **Microsoft C++ Build Tools** (Visual Studio Build Tools → "Desktop development with C++").
3. **WebView2** — already present on Windows 10/11.
4. **Node.js** (for the Tauri CLI).

Then:

```bash
npm install
npm run dev      # hot-reloading dev window
npm run build    # produces installers in src-tauri/target/release/bundle/
```

> First compile pulls & builds the Rust crates (a few minutes). If `sysinfo`'s API
> has drifted from what `main.rs` expects, paste the compiler error — the fixes are
> small and version-local.

### Linux / macOS

Same flow. Linux additionally needs `webkit2gtk` + `libappindicator` dev packages
(see the Tauri prerequisites page). Note: plain WSL can't display GUI windows without
WSLg — build/run on a native desktop.

## What's live vs. hard (the honest scope)

- **Portable, no privileges:** CPU%, RAM, disk usage, network up/down, top processes. ✅
- **NVIDIA GPU** util/temp via NVML (no admin). ✅ AMD/Intel GPUs: later phase.
- **Temperatures:** Linux (`hwmon`) and macOS (SMC) work; **Windows CPU temp needs a
  sensor driver** and is shown as `--` until a helper is added. This is the one genuinely
  hard cross-platform bit and is deliberately deferred.
- Out of scope (vendor hardware control, not monitoring): fan control, overclocking, RGB.

## Distribution (requirement 5)

Tauri's bundler emits, from this one codebase:

- **Windows:** `.msi` + NSIS `.exe`
- **macOS:** `.dmg` / `.app` (notarise for Gatekeeper)
- **Linux:** `.AppImage` (portable) + `.deb` / `.rpm`

Later: `winget`, `brew --cask`, Flathub, AUR; plus Tauri's built-in auto-updater.

## Roadmap

- **v1 (now):** styled Dashboard on portable metrics + NVIDIA GPU.
- **v2:** Detailed mode (per-core tables + live line charts) and Info mode.
- **v3:** Windows sensor helper (LibreHardwareMonitor-style) for CPU/mobo temps; tray + always-on-top.
