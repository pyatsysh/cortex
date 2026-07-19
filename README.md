# Cortex

Cortex reads your machine and draws it. One small window tracks CPU, memory, disk, network and every NVIDIA GPU (utilisation, temperature, power, VRAM and clocks) on a board of cards you arrange yourself, and nothing it measures ever leaves the computer.

It is built in Rust on Tauri, so it ships as a native window instead of a bundled browser.

> Status: built and tested so far only on Windows 11, developed under WSL. The Linux and macOS code paths are written and compile in principle but are unproven. See [Status](#status).

## Why it exists

The system monitors that look good run on one operating system: iStat Menus and `stats` on macOS, HWiNFO and Rainmeter on Windows, Mission Center and Conky on Linux. The ones that run everywhere are terminal programs (btop, glances) or server dashboards (Netdata). The ones that look like gaming software are locked to the vendor's own hardware. The middle is empty: one lightweight, cross-platform window that treats the GPU as a first-class part and phones nobody.

Cortex is that window.

## What it does

- **It leaves nothing behind.** The gaming hub it replaces measured 863 MB across 15 background processes on the same machine, and those processes run whether or not you open the app. Cortex registers no background services: close the window and nothing of it is left running.
- **It says nothing to the network.** It reads the operating system's own counters and the NVIDIA driver, then draws them. The only address it will ever load is a TensorBoard server you point it at yourself, on localhost.
- **It puts the GPU first.** Every NVIDIA device gets its own card: utilisation, temperature, power in watts, VRAM used against total, core and memory clocks, and the installed driver and CUDA version. Two GPUs give two cards, four give four.
- **It rearranges to fit your head.** Cards move, hide and drop into empty slots on a grid; the layout is saved between sessions. Keep the cards you read, close the ones you do not.
- **It watches training runs.** A Training tab embeds TensorBoard once you connect it, and when nothing is connected it still reports whether the GPU is working or idle.
- **It has three depths.** A dashboard of ring gauges, a detailed view with per-core tables and live charts, and a system-information sheet you can copy or print.

## Status

So far Cortex has been built and run only on **Windows 11**, developed under WSL. The Linux and macOS paths are written and should compile, but they have not been built or tested, so treat them as unproven. Two known gaps on Windows: CPU temperature needs a sensor helper and reads `--` until one is added, and GPU coverage is NVIDIA-only through NVML (AMD and Intel are not read yet).

## Build

Prerequisites: the Rust MSVC toolchain (<https://rustup.rs>), the Visual C++ build tools, Node.js, and WebView2 (already present on Windows 10/11).

```bash
npm install
npm run dev      # hot-reloading dev window
npm run build    # native installers under src-tauri/target/release/bundle/
```

The full Windows walkthrough is in [BUILD_WINDOWS.md](BUILD_WINDOWS.md).

## Roadmap

- **Now:** the card dashboard, the detailed per-core view with live charts, the system-info sheet, and the Training tab.
- **Next:** Windows CPU and motherboard temperature through a sensor helper; per-process VRAM (which process owns the GPU); alerts for imminent VRAM exhaustion, thermal throttling and stalled runs.
- **Then:** AMD and Intel GPUs; native ingestion of TensorBoard scalar files so loss curves sit beside the telemetry; remote monitoring of a headless training box over the network.
- **Eventually:** real Linux and macOS builds, and one-line installs (`winget`, `brew`, Flathub).

## Licence

MIT.
