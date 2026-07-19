# Cortex

Cortex is a system monitor for the machine you train models on. It reads your machine and draws it: CPU, memory, disk, network and every NVIDIA GPU on a board of cards you arrange yourself, keeps your TensorBoard in the same window, and lets nothing it measures leave the computer.

It is built in Rust on Tauri, so it ships as a native window instead of a bundled browser.

> Status: built and tested so far only on Windows 11, developed under WSL. The Linux and macOS code paths are written and compile in principle but are unproven. See [Status](#status).

## Why it exists

While a training run is going, you want two things on screen: what the GPUs are doing, and what the loss is doing. Today they live in different places. The vendor hubs that draw a GPU nicely (OMEN, Armoury Crate, iCUE) are built for gamers and locked to the vendor's hardware. The monitors that run everywhere are terminal programs (btop, glances) or server dashboards (Netdata), and none of them sit beside TensorBoard. So you watch one machine through a heavy vendor app, a terminal, and a browser tab at once.

Cortex is the one window: GPU-first telemetry and your TensorBoard together, lightweight, and local.

## For training runs

- **Every GPU, in full.** Each NVIDIA device gets its own card: utilisation, temperature, power in watts, VRAM used against total, core and memory clocks, and the installed driver and CUDA version (the first thing a framework checks). A four-GPU box gets four cards.
- **Alive or stalled, at a glance.** The Training tab reads GPU utilisation continuously, so you can see whether the GPUs are working or the run has died before you open a single loss curve.
- **TensorBoard, in the same window.** Point the Training tab at your local TensorBoard and it opens inside Cortex, next to the telemetry, instead of in yet another browser tab.
- **Nothing leaves the box.** Cortex reads the operating system's counters and the NVIDIA driver, then draws them. The only address it will ever load is the TensorBoard server you name yourself, on localhost.

## What else it does

- **It leaves nothing behind.** The gaming hub it replaces measured 863 MB across 15 background processes on the same machine, running whether or not you opened it. Cortex registers no background services: close the window and nothing of it is left running.
- **It rearranges to fit your head.** Cards move, hide and drop into empty slots on a grid, and the layout is saved between sessions. Keep the cards you read, close the ones you do not.
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

- **Now:** the GPU-first card dashboard, the detailed per-core view with live charts, the system-info sheet, and the Training tab with the TensorBoard embed and the alive-or-stalled GPU signal.
- **Next:** per-process VRAM, so you can see which training process owns which card; alerts for imminent VRAM exhaustion, thermal throttling and a stalled run; the Windows CPU and motherboard temperature helper.
- **Then:** native ingestion of TensorBoard's scalar files, so loss and accuracy curves sit on the board beside the GPU telemetry with no browser at all; AMD and Intel GPUs; remote monitoring of a headless training box over the network.
- **Eventually:** real Linux and macOS builds, and one-line installs (`winget`, `brew`, Flathub).

## Licence

MIT.
