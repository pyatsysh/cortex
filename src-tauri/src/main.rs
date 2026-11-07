// Prevent an extra console window on Windows in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use serde::Serialize;
use std::ffi::OsString;
use std::io::Write;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use sysinfo::{Components, CpuRefreshKind, Disks, Networks, ProcessesToUpdate, System, ThreadKind};
use tauri::State;

use nvml_wrapper::enum_wrappers::device::{Clock, TemperatureSensor};
use nvml_wrapper::Nvml;

// ---- shape sent to the frontend (mirrors the mock object in src/index.html) ----
//
// Units are fixed per field and noted beside it. A reading the platform cannot supply
// is `None` (JSON null, rendered "N/A" or "—"), never a stand-in zero.

#[derive(Serialize)]
struct Gpu {
    name: String,
    util: Option<f32>,       // % engine busy — None => UI shows "N/A"
    temp: Option<f32>,       // °C
    power: Option<f32>,      // Watts (board power draw)
    mem_util: Option<f32>,   // % memory-controller busy (bandwidth activity)
    vram_pct: Option<f32>,   // % of VRAM in use
    vram_used: Option<f32>,  // GB of VRAM in use
    vram_total: Option<f32>, // GB of VRAM total
    clock_core: Option<u32>, // MHz
    clock_mem: Option<u32>,  // MHz
}

#[derive(Serialize)]
struct Core {
    util: f32,         // %
    freq: Option<u64>, // MHz — None when the platform reports no clock
}

#[derive(Serialize)]
struct SysInfo {
    os: String,
    kernel: String,
    hostname: String,
    cpu: String,
    cpu_threads: usize,
    cpu_mhz: Option<u64>, // MHz — None when the platform reports no clock
    gpu: String,
    gpu_driver: String,
    cuda: String, // CUDA driver version, e.g. "12.4"
    ram_total: f32, // GB
    uptime: u64,    // seconds
    arch: String,
    wsl: bool, // true when running under WSL: readings describe the VM, not native hardware
}
#[derive(Serialize)]
struct Cpu {
    util: f32,         // %
    temp: Option<f32>, // °C — None without a readable CPU sensor
}
#[derive(Serialize)]
struct Ram {
    util: f32,  // %
    used: f32,  // GB
    total: f32, // GB
}
#[derive(Serialize)]
struct DiskInfo {
    name: String, // mount point / drive letter
    free: f32,    // GB
    total: f32,   // GB
}
#[derive(Serialize)]
struct Net {
    up: f32,   // Mbps
    down: f32, // Mbps
}
#[derive(Serialize)]
struct Proc {
    name: String,
    cpu: f32,         // % of one core (can exceed 100 for multi-threaded processes)
    gpu: Option<f32>, // per-process GPU% is not sampled on any platform yet — always None
    ram: u64,         // MB
    color: String,
}
#[derive(Serialize)]
struct Metrics {
    gpus: Vec<Gpu>,
    cpu: Cpu,
    ram: Ram,
    disks: Vec<DiskInfo>,
    net: Net,
    procs: Vec<Proc>,
    cores: Vec<Core>,
    interval_ms: u64, // measured time since the previous sample; network rates use it
}

/// One platform capability, in the shared Lab-OS vocabulary
/// (available | unavailable | degraded | unsupported | disabled | unknown).
#[derive(Serialize, Clone, Debug)]
struct Capability {
    capability: &'static str,
    state: &'static str,
    reason: String,
    source: String,
    platform: &'static str,
}

fn capability(capability: &'static str, state: &'static str, reason: impl Into<String>, source: impl Into<String>) -> Capability {
    Capability { capability, state, reason: reason.into(), source: source.into(), platform: platform_label() }
}

fn platform_label() -> &'static str {
    if cfg!(target_os = "windows") {
        "windows"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else if std::env::var_os("WAYLAND_DISPLAY").is_some() {
        "linux-wayland"
    } else if std::env::var_os("DISPLAY").is_some() {
        "linux-x11"
    } else {
        "linux-headless"
    }
}

/// WSL runs a Linux kernel inside a VM: its readings are not native hardware evidence.
fn is_wsl() -> bool {
    #[cfg(target_os = "linux")]
    {
        std::env::var_os("WSL_DISTRO_NAME").is_some()
            || std::fs::read_to_string("/proc/sys/kernel/osrelease")
                .map(|s| s.to_lowercase().contains("microsoft"))
                .unwrap_or(false)
    }
    #[cfg(not(target_os = "linux"))]
    {
        false
    }
}

// ---- persistent state (keeps deltas for CPU% and network rates) ----

struct AppState {
    sys: System,
    networks: Networks,
    nvml: Option<Nvml>,
    nvml_status: Capability,
    last: Instant,
    #[cfg(windows)]
    clock: Option<win_perf::ClockReader>,
}

const CHIP_PALETTE: [&str; 5] = ["#88C0D0", "#81A1C1", "#B48EAD", "#A3BE8C", "#D08770"];

/// Bytes per binary gigabyte (GiB). Windows Explorer, Task Manager and the OMEN
/// Gaming Hub all divide by 1024³ but *label* the result "GB", so we match that
/// convention (rather than decimal 1000³) to line up with what users see elsewhere.
const GIB: f64 = 1024.0 * 1024.0 * 1024.0;

// ---- platform boundary: small pure helpers, unit tested below ----

/// NVML libraries to try, most specific first. Linux driver packages ship only the
/// versioned soname `libnvidia-ml.so.1`; the unversioned name nvml-wrapper tries by
/// default comes from development packages, so a plain driver install needs `.so.1`.
/// `CORTEX_NVML_LIB` replaces the search (a test double, or an unusual install).
/// An empty list means nvml-wrapper's own platform default (`nvml.dll` on Windows).
fn nvml_candidates(override_path: Option<OsString>) -> Vec<OsString> {
    if let Some(p) = override_path.filter(|p| !p.is_empty()) {
        return vec![p];
    }
    if cfg!(target_os = "linux") {
        vec!["libnvidia-ml.so.1".into(), "libnvidia-ml.so".into()]
    } else {
        Vec::new()
    }
}

fn load_nvml(candidates: &[OsString]) -> (Option<Nvml>, Capability) {
    let mut tried: Vec<String> = Vec::new();
    if candidates.is_empty() {
        match Nvml::init() {
            Ok(n) => return (Some(n), capability("gpu-telemetry", "available", "", "NVML (platform default library)")),
            Err(e) => tried.push(format!("platform default: {e}")),
        }
    }
    for c in candidates {
        match Nvml::builder().lib_path(c.as_os_str()).init() {
            Ok(n) => return (Some(n), capability("gpu-telemetry", "available", "", format!("NVML via {}", c.to_string_lossy()))),
            Err(e) => tried.push(format!("{}: {e}", c.to_string_lossy())),
        }
    }
    let reason = format!(
        "NVIDIA management library not loaded, GPU readings show N/A. Tried {}",
        tried.join("; ")
    );
    (None, capability("gpu-telemetry", "unavailable", reason, "NVML"))
}

/// A die temperature a real sensor could report. Broken or absent sensors surface as
/// NaN, 0, negative or absurd values; those become None rather than a fake reading.
fn plausible_temp(t: f32) -> Option<f32> {
    (t.is_finite() && t > 0.0 && t < 150.0).then_some(t)
}

/// CPU temperature: the hottest plausible reading among CPU/package sensors.
fn cpu_temp_from<'a>(readings: impl IntoIterator<Item = (&'a str, Option<f32>)>) -> Option<f32> {
    readings
        .into_iter()
        .filter(|(label, _)| {
            let l = label.to_lowercase();
            l.contains("cpu") || l.contains("package") || l.contains("tctl") || l.contains("core")
        })
        .filter_map(|(_, t)| t.and_then(plausible_temp))
        .fold(None, |acc: Option<f32>, t| Some(acc.map_or(t, |m| m.max(t))))
}

/// A clock of 0 MHz means "not reported" (no cpufreq, no cpuinfo field), not a stopped core.
fn reported_mhz(mhz: u64) -> Option<u64> {
    (mhz > 0).then_some(mhz)
}

/// Linux lists the loopback device with the physical interfaces; its traffic never leaves
/// the machine, so counting it would inflate the throughput shown.
fn is_loopback(name: &str) -> bool {
    cfg!(target_os = "linux") && name == "lo"
}

fn net_rates<'a>(ifaces: impl IntoIterator<Item = (&'a str, u64, u64)>, elapsed_s: f32) -> Net {
    let elapsed = if elapsed_s.is_finite() { elapsed_s.max(0.001) } else { 0.001 };
    let (mut rx, mut tx) = (0u64, 0u64);
    for (name, received, transmitted) in ifaces {
        if is_loopback(name) {
            continue;
        }
        rx = rx.saturating_add(received);
        tx = tx.saturating_add(transmitted);
    }
    let to_mbps = |bytes: u64| (bytes as f32 * 8.0 / 1e6) / elapsed;
    Net { up: to_mbps(tx), down: to_mbps(rx) }
}

struct ProcRow {
    name: String,
    cpu: f32,
    mem_bytes: u64,
    /// Linux lists every thread of a process as its own entry; the process already
    /// accounts for them, so listing both would double count.
    thread_of_process: bool,
}

fn top_processes(rows: impl IntoIterator<Item = ProcRow>) -> Vec<Proc> {
    let mut procs: Vec<ProcRow> = rows
        .into_iter()
        .filter(|r| !r.thread_of_process && r.cpu.is_finite())
        .collect();
    procs.sort_by(|a, b| b.cpu.partial_cmp(&a.cpu).unwrap_or(std::cmp::Ordering::Equal));
    procs
        .into_iter()
        .take(5)
        .enumerate()
        .map(|(i, r)| Proc {
            name: r.name,
            cpu: r.cpu,
            gpu: None,
            ram: r.mem_bytes / 1_000_000, // bytes -> MB
            color: CHIP_PALETTE[i % CHIP_PALETTE.len()].to_string(),
        })
        .collect()
}

struct DiskRow {
    device: String,
    mount: String,
    total: u64,
    available: u64,
    removable: bool,
}

/// Skip removable media and empty filesystems; one entry per mount point. On Linux a
/// filesystem bind-mounted in several places appears once, under its shortest path.
fn disk_list(rows: impl IntoIterator<Item = DiskRow>) -> Vec<DiskInfo> {
    let mut seen_mounts = std::collections::HashSet::new();
    let mut kept: Vec<DiskRow> = Vec::new();
    for r in rows {
        if r.removable || r.total == 0 || !seen_mounts.insert(r.mount.clone()) {
            continue;
        }
        if cfg!(target_os = "linux") {
            if let Some(k) = kept.iter_mut().find(|k| k.device == r.device && k.total == r.total) {
                if r.mount.len() < k.mount.len() {
                    k.mount = r.mount;
                    k.available = r.available;
                }
                continue;
            }
        }
        kept.push(r);
    }
    kept.into_iter()
        .map(|r| DiskInfo {
            name: r.mount,
            free: (r.available as f64 / GIB) as f32,
            total: (r.total as f64 / GIB) as f32,
        })
        .collect()
}

// ---- sampling ----

/// What every sample refreshes for the CPUs: usage and the per-core clocks. `refresh_cpu_usage()`
/// alone refreshes usage only, which left the clocks at the values read when the state was made.
fn cpu_refresh() -> CpuRefreshKind {
    CpuRefreshKind::nothing().with_cpu_usage().with_frequency()
}

fn new_state() -> AppState {
    let mut sys = System::new_all();
    sys.refresh_all();
    let (nvml, nvml_status) = load_nvml(&nvml_candidates(std::env::var_os("CORTEX_NVML_LIB")));
    AppState {
        sys,
        networks: Networks::new_with_refreshed_list(),
        nvml, // None on machines without a loadable NVIDIA driver
        nvml_status,
        last: Instant::now(),
        #[cfg(windows)]
        clock: win_perf::ClockReader::new(),
    }
}

fn sample(s: &mut AppState) -> Metrics {
    let elapsed = s.last.elapsed().as_secs_f32().max(0.001);
    s.last = Instant::now();

    s.sys.refresh_cpu_specifics(cpu_refresh());
    s.sys.refresh_memory();
    s.sys.refresh_processes(ProcessesToUpdate::All, true);
    s.networks.refresh(true);

    // CPU (global + per-core). On Windows sysinfo returns only the static *base*
    // clock, so we overlay the live aggregate clock (turbo) from PDH — the same
    // "base × % performance" figure Task Manager shows. All cores share it, which
    // is also how OMEN reports per-core clocks. Linux reads cpufreq (live) or, without
    // it, the `cpu MHz` field of /proc/cpuinfo.
    let cpu_util = s.sys.global_cpu_usage();
    #[allow(unused_mut)] // only the Windows clock overlay below mutates it
    let mut cores: Vec<Core> = s
        .sys
        .cpus()
        .iter()
        .map(|c| Core { util: c.cpu_usage(), freq: reported_mhz(c.frequency()) })
        .collect();
    #[cfg(windows)]
    if let Some(live) = s.clock.as_ref().and_then(|c| c.read()) {
        if live > 0.0 {
            let mhz = live.round() as u64;
            for c in cores.iter_mut() {
                c.freq = Some(mhz);
            }
        }
    }

    // Memory (bytes -> GB, GB = 1e9 to match how OSes label drives/RAM)
    let total_b = s.sys.total_memory() as f64;
    let used_b = s.sys.used_memory() as f64;
    let ram = Ram {
        util: if total_b > 0.0 { (used_b / total_b * 100.0) as f32 } else { 0.0 },
        used: (used_b / GIB) as f32,
        total: (total_b / GIB) as f32,
    };

    // Network — received()/transmitted() are bytes since the previous refresh.
    let net = net_rates(
        s.networks.iter().map(|(name, data)| (name.as_str(), data.received(), data.transmitted())),
        elapsed,
    );

    // Processes — top 5 by CPU
    let top = top_processes(s.sys.processes().values().map(|p| ProcRow {
        name: p.name().to_string_lossy().to_string(),
        cpu: p.cpu_usage(),
        mem_bytes: p.memory(),
        thread_of_process: matches!(p.thread_kind(), Some(ThreadKind::Userland)),
    }));

    // NVIDIA GPUs (0..N — empty on machines without NVML / non-NVIDIA cards)
    let mut gpus: Vec<Gpu> = Vec::new();
    if let Some(nvml) = &s.nvml {
        let count = nvml.device_count().unwrap_or(0);
        for i in 0..count {
            let dev = match nvml.device_by_index(i) {
                Ok(d) => d,
                Err(_) => continue,
            };
            let (mut util, mut mem_util) = (None, None);
            if let Ok(u) = dev.utilization_rates() {
                util = (u.gpu <= 100).then_some(u.gpu as f32);
                mem_util = (u.memory <= 100).then_some(u.memory as f32);
            }
            let (mut vram_pct, mut vram_used, mut vram_total) = (None, None, None);
            if let Ok(mem) = dev.memory_info() {
                if mem.total > 0 {
                    vram_used = Some((mem.used as f64 / GIB) as f32);
                    vram_total = Some((mem.total as f64 / GIB) as f32);
                    vram_pct = Some((mem.used as f64 / mem.total as f64 * 100.0) as f32);
                }
            }
            gpus.push(Gpu {
                name: dev.name().unwrap_or_else(|_| "GPU".into()),
                util,
                temp: dev.temperature(TemperatureSensor::Gpu).ok().and_then(|t| plausible_temp(t as f32)),
                power: dev.power_usage().ok().map(|mw| mw as f32 / 1000.0),
                mem_util,
                vram_pct,
                vram_used,
                vram_total,
                clock_core: dev.clock_info(Clock::Graphics).ok().filter(|&c| c > 0),
                clock_mem: dev.clock_info(Clock::Memory).ok().filter(|&c| c > 0),
            });
        }
    }

    // Temperatures (built fresh each tick; cheap at ~1 Hz). Windows often has no sensor
    // without a driver, WSL has none at all; both are handled as None in the UI.
    let components = Components::new_with_refreshed_list();
    let cpu_temp = cpu_temp_from(components.iter().map(|c| (c.label(), c.temperature())));

    let disks = Disks::new_with_refreshed_list();
    let disk_info = disk_list(disks.iter().map(|d| DiskRow {
        device: d.name().to_string_lossy().to_string(),
        mount: d.mount_point().to_string_lossy().to_string(),
        total: d.total_space(),
        available: d.available_space(),
        removable: d.is_removable(),
    }));

    Metrics {
        gpus,
        cpu: Cpu { util: cpu_util, temp: cpu_temp },
        ram,
        disks: disk_info,
        net,
        procs: top,
        cores,
        interval_ms: (elapsed * 1000.0).round() as u64,
    }
}

fn sysinfo_of(s: &AppState) -> SysInfo {
    let cpu0 = s.sys.cpus().first();

    let (gpu, gpu_driver, cuda) = match &s.nvml {
        Some(nvml) => {
            let name = nvml
                .device_by_index(0)
                .ok()
                .and_then(|d| d.name().ok())
                .unwrap_or_else(|| "—".into());
            let driver = nvml.sys_driver_version().unwrap_or_else(|_| "—".into());
            // NVML reports CUDA as an int like 12040 => "12.4"
            let cuda = nvml
                .sys_cuda_driver_version()
                .ok()
                .map(|v| format!("{}.{}", v / 1000, (v % 1000) / 10))
                .unwrap_or_else(|| "—".into());
            (name, driver, cuda)
        }
        None => ("—".into(), "—".into(), "—".into()),
    };

    SysInfo {
        os: System::long_os_version().unwrap_or_else(|| "Unknown OS".into()),
        kernel: System::kernel_version().unwrap_or_else(|| "—".into()),
        hostname: System::host_name().unwrap_or_else(|| "—".into()),
        cpu: cpu0.map(|c| c.brand().to_string()).unwrap_or_else(|| "—".into()),
        cpu_threads: s.sys.cpus().len(),
        cpu_mhz: cpu0.and_then(|c| reported_mhz(c.frequency())),
        gpu,
        gpu_driver,
        cuda,
        ram_total: (s.sys.total_memory() as f64 / GIB) as f32,
        uptime: System::uptime(),
        arch: std::env::consts::ARCH.to_string(),
        wsl: is_wsl(),
    }
}

/// What this machine can supply, in the shared capability vocabulary.
fn capabilities(s: &AppState, m: &Metrics) -> Vec<Capability> {
    let mut caps = vec![s.nvml_status.clone()];
    if let Some(nvml) = &s.nvml {
        if nvml.device_count().unwrap_or(0) == 0 {
            caps[0] = capability("gpu-telemetry", "unavailable", "NVML loaded but reports no NVIDIA device", s.nvml_status.source.clone());
        }
    }
    caps.push(if m.cpu.temp.is_some() {
        capability("cpu-temperature", "available", "", "sysinfo components (Linux: /sys/class/hwmon)")
    } else {
        capability(
            "cpu-temperature",
            "unavailable",
            "no readable CPU or package temperature sensor; the UI shows --",
            "sysinfo components (Linux: /sys/class/hwmon)",
        )
    });
    let clocks = m.cores.iter().filter(|c| c.freq.is_some()).count();
    caps.push(if cfg!(target_os = "linux") {
        if std::path::Path::new("/sys/devices/system/cpu/cpu0/cpufreq/scaling_cur_freq").exists() {
            capability("cpu-clock", "available", "", "/sys/devices/system/cpu/*/cpufreq/scaling_cur_freq")
        } else if clocks > 0 {
            capability("cpu-clock", "degraded", "no cpufreq driver; the /proc/cpuinfo figure may be a fixed nominal clock", "/proc/cpuinfo")
        } else {
            capability("cpu-clock", "unavailable", "no cpufreq driver and no clock in /proc/cpuinfo", "/proc/cpuinfo")
        }
    } else if clocks > 0 {
        capability("cpu-clock", "available", "", "sysinfo (Windows: PDH live clock when readable)")
    } else {
        capability("cpu-clock", "unavailable", "the platform reports no CPU clock", "sysinfo")
    });
    caps.push(if m.ram.total > 0.0 {
        capability("memory", "available", "", "sysinfo (Linux: /proc/meminfo)")
    } else {
        capability("memory", "unavailable", "total memory reads as zero; the memory source is not readable", "sysinfo")
    });
    caps.push(capability("network-throughput", "available", "", "sysinfo (Linux: /sys/class/net, loopback excluded)"));
    caps.push(capability("per-process-gpu", "disabled", "per-process GPU use is not sampled; the field is null", "none"));
    caps
}

#[tauri::command]
fn get_metrics(state: State<Mutex<AppState>>) -> Metrics {
    let mut s = state.lock().unwrap_or_else(|e| e.into_inner());
    sample(&mut s)
}

#[tauri::command]
fn get_sysinfo(state: State<Mutex<AppState>>) -> SysInfo {
    let s = state.lock().unwrap_or_else(|e| e.into_inner());
    sysinfo_of(&s)
}

/// Windows live-clock reader via PDH performance counters.
///
/// Live clock = `Processor Frequency` (nominal base, MHz) × `% Processor Performance`
/// (the turbo ratio, which exceeds 100 when boosting) ÷ 100 — the exact figure
/// Task Manager's "Speed" field shows. The query is kept alive and sampled once per
/// tick because `% Processor Performance` needs an interval between collections.
#[cfg(windows)]
mod win_perf {
    use windows_sys::Win32::System::Performance::{
        PdhAddEnglishCounterW, PdhCollectQueryData, PdhGetFormattedCounterValue, PdhOpenQueryW,
        PDH_FMT_COUNTERVALUE, PDH_FMT_DOUBLE,
    };

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    pub struct ClockReader {
        query: isize,
        perf: isize, // \Processor Information(_Total)\% Processor Performance
        freq: isize, // \Processor Information(_Total)\Processor Frequency  (nominal base)
    }

    impl ClockReader {
        pub fn new() -> Option<ClockReader> {
            unsafe {
                let mut query: isize = 0;
                if PdhOpenQueryW(std::ptr::null(), 0, &mut query) != 0 {
                    return None;
                }
                let mut perf: isize = 0;
                let p = wide("\\Processor Information(_Total)\\% Processor Performance");
                if PdhAddEnglishCounterW(query, p.as_ptr(), 0, &mut perf) != 0 {
                    return None;
                }
                let mut freq: isize = 0;
                let f = wide("\\Processor Information(_Total)\\Processor Frequency");
                if PdhAddEnglishCounterW(query, f.as_ptr(), 0, &mut freq) != 0 {
                    return None;
                }
                PdhCollectQueryData(query); // prime (rate counters need a first sample)
                Some(ClockReader { query, perf, freq })
            }
        }

        /// Live aggregate clock in MHz, or None if a collection failed this tick.
        pub fn read(&self) -> Option<f64> {
            unsafe {
                if PdhCollectQueryData(self.query) != 0 {
                    return None;
                }
                let mut vp: PDH_FMT_COUNTERVALUE = std::mem::zeroed();
                if PdhGetFormattedCounterValue(self.perf, PDH_FMT_DOUBLE, std::ptr::null_mut(), &mut vp) != 0 {
                    return None;
                }
                let mut vf: PDH_FMT_COUNTERVALUE = std::mem::zeroed();
                if PdhGetFormattedCounterValue(self.freq, PDH_FMT_DOUBLE, std::ptr::null_mut(), &mut vf) != 0 {
                    return None;
                }
                let base = vf.Anonymous.doubleValue;
                let perf_pct = vp.Anonymous.doubleValue;
                if base <= 0.0 {
                    return None;
                }
                Some(base * perf_pct / 100.0)
            }
        }
    }
}

// ---- headless probe: `cortex --probe [--samples N] [--interval-ms MS]` ----
//
// Prints one JSON object per line (sysinfo, capabilities, then N metric samples) using the
// same sampler as the window, then exits. No window, webview or child process is
// started, so it runs on a machine without a desktop session.

struct ProbeArgs {
    samples: u32,
    interval_ms: u64,
}

fn parse_probe_args(args: &[String]) -> Result<Option<ProbeArgs>, String> {
    if args.first().map(String::as_str) != Some("--probe") {
        return Ok(None);
    }
    let mut p = ProbeArgs { samples: 3, interval_ms: 1000 };
    let mut it = args[1..].iter();
    while let Some(a) = it.next() {
        let mut value = |flag: &str| it.next().ok_or_else(|| format!("{flag} needs a value"));
        match a.as_str() {
            "--samples" => {
                p.samples = value("--samples")?.parse().map_err(|_| "--samples takes a whole number".to_string())?;
            }
            "--interval-ms" => {
                p.interval_ms = value("--interval-ms")?.parse().map_err(|_| "--interval-ms takes a whole number".to_string())?;
            }
            other => return Err(format!("unknown probe argument: {other}")),
        }
    }
    if p.samples == 0 || p.samples > 3600 {
        return Err("--samples must be between 1 and 3600".into());
    }
    if p.interval_ms < 100 || p.interval_ms > 60_000 {
        return Err("--interval-ms must be between 100 and 60000".into());
    }
    Ok(Some(p))
}

fn run_probe(p: &ProbeArgs) -> std::io::Result<()> {
    let mut s = new_state();
    let mut out = std::io::stdout().lock();
    writeln!(out, "{}", serde_json::json!({ "kind": "sysinfo", "data": sysinfo_of(&s) }))?;
    // CPU usage needs two refreshes separated by sysinfo's minimum interval.
    std::thread::sleep(Duration::from_millis(p.interval_ms).max(sysinfo::MINIMUM_CPU_UPDATE_INTERVAL));
    for seq in 0..p.samples {
        let m = sample(&mut s);
        if seq == 0 {
            writeln!(out, "{}", serde_json::json!({ "kind": "capabilities", "items": capabilities(&s, &m) }))?;
        }
        writeln!(out, "{}", serde_json::json!({ "kind": "metrics", "seq": seq, "data": m }))?;
        out.flush()?;
        if seq + 1 < p.samples {
            std::thread::sleep(Duration::from_millis(p.interval_ms));
        }
    }
    Ok(())
}

const USAGE: &str = "\
usage: cortex                       open the monitor window
       cortex --probe [--samples N] [--interval-ms MS]
                                    print readings as JSON lines; no window
       cortex --help
";

/// What the command line asks for. Only an empty command line opens the window: anything not
/// understood is refused, so `cortex --help` or a mistyped flag never puts a window on a desktop.
enum Invocation {
    Window,
    Probe(ProbeArgs),
    Help,
}

fn parse_args(args: &[String]) -> Result<Invocation, String> {
    match args.first().map(String::as_str) {
        None => Ok(Invocation::Window),
        // Finder on older macOS passes a process serial number to an app it launches.
        Some(a) if cfg!(target_os = "macos") && a.starts_with("-psn_") && args.len() == 1 => Ok(Invocation::Window),
        Some("--probe") => parse_probe_args(args)
            .map_err(|e| format!("--probe: {e}"))?
            .map(Invocation::Probe)
            .ok_or_else(|| "--probe was not parsed".to_string()),
        Some("--help" | "-h") if args.len() == 1 => Ok(Invocation::Help),
        Some(_) => Err(format!("unknown arguments: {}", args.join(" "))),
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match parse_args(&args) {
        Ok(Invocation::Window) => {}
        Ok(Invocation::Probe(p)) => {
            if let Err(e) = run_probe(&p) {
                eprintln!("cortex --probe: {e}");
                std::process::exit(1);
            }
            return;
        }
        Ok(Invocation::Help) => {
            print!("{USAGE}");
            return;
        }
        Err(e) => {
            eprint!("cortex: {e}\n{USAGE}");
            std::process::exit(2);
        }
    }

    tauri::Builder::default()
        .manage(Mutex::new(new_state()))
        .invoke_handler(tauri::generate_handler![get_metrics, get_sysinfo])
        .run(tauri::generate_context!())
        .expect("error while running Cortex");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_sample_refreshes_cpu_clocks_as_well_as_usage() {
        let k = cpu_refresh();
        assert!(k.cpu_usage(), "usage is refreshed");
        assert!(k.frequency(), "clocks are refreshed, not kept from start-up");
    }

    #[test]
    fn missing_nvml_library_is_reported_unavailable_not_zero() {
        let dir = std::env::temp_dir().join("cortex tests ünïcode 'q'");
        let lib = dir.join("no such dir").join("libnvidia-ml.so.1");
        let (nvml, cap) = load_nvml(&[lib.clone().into_os_string()]);
        assert!(nvml.is_none());
        assert_eq!(cap.capability, "gpu-telemetry");
        assert_eq!(cap.state, "unavailable");
        assert!(cap.reason.contains(&*lib.to_string_lossy()), "reason names what was tried: {}", cap.reason);
    }

    #[test]
    fn malformed_nvml_library_is_refused_without_panic() {
        let dir = std::env::temp_dir().join(format!("cortex-fake-nvml-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let lib = dir.join("libnvidia-ml.so.1");
        std::fs::write(&lib, b"not an ELF shared object").unwrap();
        let (nvml, cap) = load_nvml(&[lib.into_os_string()]);
        std::fs::remove_dir_all(&dir).ok();
        assert!(nvml.is_none());
        assert_eq!(cap.state, "unavailable");
    }

    #[test]
    fn empty_override_falls_back_to_platform_search() {
        let with_empty = nvml_candidates(Some(OsString::new()));
        assert_eq!(with_empty, nvml_candidates(None));
        let explicit = nvml_candidates(Some("/opt/x/libnvidia-ml.so.1".into()));
        assert_eq!(explicit, vec![OsString::from("/opt/x/libnvidia-ml.so.1")]);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_searches_the_driver_soname_before_the_dev_symlink() {
        // A driver-only install has libnvidia-ml.so.1 and no unversioned libnvidia-ml.so.
        let c = nvml_candidates(None);
        let so1 = c.iter().position(|p| p == "libnvidia-ml.so.1").expect("versioned soname searched");
        let dev = c.iter().position(|p| p == "libnvidia-ml.so").unwrap_or(usize::MAX);
        assert!(so1 < dev);
    }

    #[test]
    fn broken_sensor_values_become_unavailable() {
        for bad in [f32::NAN, f32::INFINITY, -273.15, 0.0, -5.0, 255.0, 1000.0] {
            assert_eq!(plausible_temp(bad), None, "{bad} must not be shown as a temperature");
        }
        assert_eq!(plausible_temp(47.5), Some(47.5));
    }

    #[test]
    fn cpu_temperature_uses_hottest_valid_cpu_sensor_only() {
        let readings = [
            ("acpitz temp1", Some(27.8)),
            ("coretemp Package id 0", Some(55.0)),
            ("coretemp Core 3", Some(f32::NAN)),
            ("coretemp Core 5", Some(61.0)),
            ("nvme Composite", Some(70.0)),
            ("k10temp Tctl", None),
        ];
        assert_eq!(cpu_temp_from(readings), Some(61.0));
        assert_eq!(cpu_temp_from([("coretemp Core 0", Some(0.0)), ("k10temp Tctl", None)]), None);
        assert_eq!(cpu_temp_from(std::iter::empty()), None);
    }

    #[test]
    fn unreported_clock_is_null_not_zero() {
        assert_eq!(reported_mhz(0), None);
        assert_eq!(reported_mhz(3696), Some(3696));
    }

    #[test]
    fn network_rate_uses_elapsed_time_and_survives_bad_intervals() {
        let n = net_rates([("eth0", 1_000_000, 250_000)], 2.0);
        assert!((n.down - 4.0).abs() < 1e-4 && (n.up - 1.0).abs() < 1e-4);
        for bad in [0.0, -1.0, f32::NAN] {
            let n = net_rates([("eth0", 1_000, 1_000)], bad);
            assert!(n.up.is_finite() && n.down.is_finite());
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_loopback_traffic_is_not_network_throughput() {
        let n = net_rates([("lo", 9_000_000_000, 9_000_000_000), ("eth0", 1_000_000, 0)], 1.0);
        assert!((n.down - 8.0).abs() < 1e-4, "down {}", n.down);
        assert_eq!(n.up, 0.0);
    }

    #[test]
    fn threads_are_not_listed_beside_their_process() {
        let row = |name: &str, cpu: f32, thread: bool| ProcRow { name: name.into(), cpu, mem_bytes: 5_000_000, thread_of_process: thread };
        let top = top_processes([
            row("rustc", 390.0, false),
            row("rustc", 99.0, true),
            row("rustc", 98.0, true),
            row("node", 12.0, false),
            row("weird", f32::NAN, false),
        ]);
        let names: Vec<&str> = top.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["rustc", "node"]);
        assert!(top.iter().all(|p| p.gpu.is_none()));
        assert_eq!(top[0].ram, 5);
    }

    #[test]
    fn disks_skip_removable_empty_and_repeated_mounts() {
        let row = |device: &str, mount: &str, total: u64, removable: bool| DiskRow {
            device: device.into(),
            mount: mount.into(),
            total,
            available: total / 2,
            removable,
        };
        let list = disk_list([
            row("/dev/nvme0n1p2", "/", 1 << 40, false),
            row("/dev/nvme0n1p2", "/", 1 << 40, false),
            row("/dev/sdb1", "/media/usb", 1 << 34, true),
            row("none", "/empty", 0, false),
            row("/dev/nvme0n1p1", "/boot/efi", 1 << 29, false),
        ]);
        let names: Vec<&str> = list.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(names, ["/", "/boot/efi"]);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_bind_mounts_of_one_filesystem_show_once() {
        let row = |mount: &str| DiskRow { device: "/dev/sdc".into(), mount: mount.into(), total: 1 << 40, available: 1 << 39, removable: false };
        let list = disk_list([row("/mnt/wslg/distro"), row("/"), row("/var/lib/docker/overlay")]);
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "/");
    }

    #[test]
    fn only_an_empty_command_line_opens_the_window() {
        let a = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert!(matches!(parse_args(&a(&[])), Ok(Invocation::Window)));
        assert!(matches!(parse_args(&a(&["--probe", "--samples", "2"])), Ok(Invocation::Probe(_))));
        for help in [&["--help"][..], &["-h"]] {
            assert!(matches!(parse_args(&a(help)), Ok(Invocation::Help)), "{help:?}");
        }
        for refused in [&["--something-else"][..], &["--version"], &["--help", "extra"], &["file.txt"], &["--probe", "--rm"], &["-psn_0_12345", "x"]] {
            assert!(parse_args(&a(refused)).is_err(), "{refused:?}");
        }
        assert_eq!(parse_args(&a(&["-psn_0_12345"])).is_ok(), cfg!(target_os = "macos"));
    }

    #[test]
    fn probe_arguments_are_bounded() {
        let a = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert!(parse_probe_args(&a(&[])).unwrap().is_none());
        assert!(parse_probe_args(&a(&["--something-else"])).unwrap().is_none());
        let p = parse_probe_args(&a(&["--probe", "--samples", "2", "--interval-ms", "250"])).unwrap().unwrap();
        assert_eq!((p.samples, p.interval_ms), (2, 250));
        for bad in [&["--probe", "--samples", "0"][..], &["--probe", "--samples"], &["--probe", "--interval-ms", "5"], &["--probe", "--rm"]] {
            assert!(parse_probe_args(&a(bad)).is_err(), "{bad:?}");
        }
    }
}
