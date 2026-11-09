# Cortex

Cortex is a lightweight system monitor for the machine you train models on. It reads your machine and draws it: CPU, memory, disk, network and every NVIDIA GPU on a board of cards you arrange yourself, connects to your TensorBoard alongside them, and lets nothing it measures leave the computer.

It is built in Rust on Tauri, so it ships as a native window instead of a bundled browser.

Homepage: <https://pyatsysh.github.io/cortex/>

> Status: built and used on Windows 11. On Linux it builds and its sampler is tested headless under WSL2; the Linux window on a native desktop and the macOS path are not yet proven. See [Status](#status).

## Why it exists

While a training run is going, you want two things on screen: what the GPUs are doing, and what the loss is doing. Today they live in different places. The vendor hubs that draw a GPU nicely (OMEN, Armoury Crate, iCUE) are built for gamers and locked to the vendor's hardware. The monitors that run everywhere are terminal programs (btop, glances) or server dashboards (Netdata), and none of them sit beside TensorBoard. So you watch one machine through a heavy vendor app, a terminal, and a browser tab at once.

Cortex is the one window: GPU-first telemetry and your TensorBoard together, lightweight, and local.

## For training runs

Readings update directly at the selected interval, without animated intermediate
numbers or gauges. The interval starts at two seconds, and one chosen in Settings
is remembered across restarts. Processes are sampled without enumerating their
threads, so the cost of a sample does not grow with every thread on the machine.
Only the visible view is redrawn, unchanged markup is retained,
and a slow sample cannot queue further samples. A hidden window pauses sampling
and refreshes when shown again; the charts show the hidden interval as a gap. A
sample that has not answered for three intervals turns the status from Live to
stale. CPU temperature is read each sample; new sensors
are discovered within 30 seconds. Top Processes reports CPU as a percentage of
one logical core, so 25% there is not a quarter of a multicore machine's capacity.

- **Every GPU, in full.** Each NVIDIA device gets its own card: utilisation, temperature, power in watts, VRAM used against total, core and memory clocks, and the installed driver and CUDA version (the first thing a framework checks). A four-GPU box gets four cards.
- **Alive or stalled, at a glance.** The Training tab reads GPU utilisation at every interval while Cortex is shown, and marks any interval it spent hidden as a gap, so you can see whether the GPUs are working or the run has died before you open a single loss curve.
- **Your TensorBoard, alongside.** The Training tab connects to the TensorBoard you run on localhost, so the training metrics live next to the hardware that produces them.
- **Nothing leaves the box.** Cortex reads the operating system's counters and the NVIDIA driver, then draws them. The only address it will ever load is the TensorBoard server you name yourself, on localhost.

## What else it does

- **It leaves nothing behind.** The gaming hub it replaces measured 863 MB across 15 background processes on the same machine, running whether or not you opened it. Cortex registers no background services: close the window and nothing of it is left running.
- **It rearranges to fit your head.** Cards move, hide and drop into empty slots on a grid, and the layout is saved between sessions. Keep the cards you read, close the ones you do not.
- **It has three depths.** A dashboard of ring gauges, a detailed view with per-core tables and live charts, and a system-information sheet you can copy or print.

## Status

So far Cortex has been built and run as a window only on **Windows 11**, developed under WSL. On **Linux** (Ubuntu 24.04, checked under WSL2 on 27 September 2026) it builds, and its sampler is tested without a window: CPU, memory, disks, network and processes come from `/proc` and `/sys`, NVIDIA readings from NVML through the driver's `libnvidia-ml.so.1`, and a missing library, device or sensor reads as N/A rather than zero. The Linux window has not yet been observed on a native desktop, and the macOS path is unproven. Two known gaps on Windows: CPU temperature needs a sensor helper and reads `--` until one is added, and GPU coverage is NVIDIA-only through NVML (AMD and Intel are not read yet).

## Build

Prerequisites: the Rust MSVC toolchain (<https://rustup.rs>), the Visual C++ build tools, Node.js, and WebView2 (already present on Windows 10/11).

```bash
npm ci
npm run dev      # hot-reloading dev window
npm run build    # native installers under src-tauri/target/release/bundle/
```

The full Windows walkthrough is in [BUILD_WINDOWS.md](BUILD_WINDOWS.md).

On Linux (Ubuntu 24.04) the prerequisites are the Rust toolchain, Node.js and these packages:

```bash
sudo apt install build-essential pkg-config libwebkit2gtk-4.1-dev libgtk-3-dev \
  libsoup-3.0-dev libjavascriptcoregtk-4.1-dev librsvg2-dev libssl-dev
```

NVIDIA readings need only the driver (it provides `libnvidia-ml.so.1`); without it the GPU cards stay hidden and the report says N/A. To check a machine without opening the window:

```bash
src-tauri/target/release/cortex --probe --samples 5 --interval-ms 1000
```

It prints JSON lines: the system report, a list of capabilities (each `available`, `degraded`, `unavailable` or `disabled`, with the reason), then one reading per sample. `CORTEX_NVML_LIB` points it at a particular NVML library. The probe starts no window and no background process, and exits when done.

See [RELEASING.md](RELEASING.md) for locked builds, installer artifacts and
draft releases. Current Linux GUI evidence is limited to a virtual X display
in WSL; a native desktop still needs testing.

## Roadmap

- **Now:** the GPU-first card dashboard, the detailed per-core view with live charts, the system-info sheet, and the Training tab with the TensorBoard embed and the alive-or-stalled GPU signal.
- **Next:** per-process VRAM, so you can see which training process owns which card; alerts for imminent VRAM exhaustion, thermal throttling and a stalled run; the Windows CPU and motherboard temperature helper.
- **Then:** native ingestion of TensorBoard's scalar files, so loss and accuracy curves sit on the board beside the GPU telemetry with no browser at all; AMD and Intel GPUs; remote monitoring of a headless training box over the network.
- **Further qualification:** observe the Linux window on a native desktop; establish the macOS build and runtime path. One-line installs (`winget`, `brew`, Flathub) remain proposals. Linux builds already exist; build success alone is not desktop acceptance.

## Licence

MIT.
