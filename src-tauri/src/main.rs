// Prevent an extra console window on Windows in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use serde::Serialize;
use std::sync::Mutex;
use std::time::Instant;
use sysinfo::{Components, Disks, Networks, ProcessesToUpdate, System};
use tauri::State;

use nvml_wrapper::enum_wrappers::device::{Clock, TemperatureSensor};
use nvml_wrapper::Nvml;

// ---- shape sent to the frontend (mirrors the mock object in src/index.html) ----

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
    util: f32,  // %
    freq: u64,  // MHz
}

#[derive(Serialize)]
struct SysInfo {
    os: String,
    kernel: String,
    hostname: String,
    cpu: String,
    cpu_threads: usize,
    cpu_mhz: u64,
    gpu: String,
    gpu_driver: String,
    cuda: String, // CUDA driver version, e.g. "12.4"
    ram_total: f32, // GB
    uptime: u64,    // seconds
    arch: String,
}
#[derive(Serialize)]
struct Cpu {
    util: f32,         // %
    temp: Option<f32>, // °C — often None on Windows without a sensor driver
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
    cpu: f32,
    gpu: f32,   // per-process GPU% is not portable yet — reported as 0 for now
    ram: u64,   // MB
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
}

// ---- persistent state (keeps deltas for CPU% and network rates) ----

struct AppState {
    sys: System,
    networks: Networks,
    nvml: Option<Nvml>,
    last: Instant,
    #[cfg(windows)]
    clock: Option<win_perf::ClockReader>,
}

const CHIP_PALETTE: [&str; 5] = ["#88C0D0", "#81A1C1", "#B48EAD", "#A3BE8C", "#D08770"];

/// Bytes per binary gigabyte (GiB). Windows Explorer, Task Manager and the OMEN
/// Gaming Hub all divide by 1024³ but *label* the result "GB", so we match that
/// convention (rather than decimal 1000³) to line up with what users see elsewhere.
const GIB: f64 = 1024.0 * 1024.0 * 1024.0;

#[tauri::command]
fn get_metrics(state: State<Mutex<AppState>>) -> Metrics {
    let mut s = state.lock().unwrap();

    let elapsed = s.last.elapsed().as_secs_f32().max(0.001);
    s.last = Instant::now();

    s.sys.refresh_cpu_usage();
    s.sys.refresh_memory();
    s.sys.refresh_processes(ProcessesToUpdate::All, true);
    s.networks.refresh(true);

    // CPU (global + per-core). On Windows sysinfo returns only the static *base*
    // clock, so we overlay the live aggregate clock (turbo) from PDH — the same
    // "base × % performance" figure Task Manager shows. All cores share it, which
    // is also how OMEN reports per-core clocks.
    let cpu_util = s.sys.global_cpu_usage();
    let mut cores: Vec<Core> = s
        .sys
        .cpus()
        .iter()
        .map(|c| Core { util: c.cpu_usage(), freq: c.frequency() })
        .collect();
    #[cfg(windows)]
    if let Some(live) = s.clock.as_ref().and_then(|c| c.read()) {
        if live > 0.0 {
            let mhz = live.round() as u64;
            for c in cores.iter_mut() {
                c.freq = mhz;
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
    let (mut rx, mut tx) = (0u64, 0u64);
    for (_, data) in &s.networks {
        rx += data.received();
        tx += data.transmitted();
    }
    let to_mbps = |bytes: u64| (bytes as f32 * 8.0 / 1e6) / elapsed;
    let net = Net { up: to_mbps(tx), down: to_mbps(rx) };

    // Processes — top 5 by CPU
    let mut procs: Vec<(String, f32, u64)> = s
        .sys
        .processes()
        .values()
        .map(|p| (p.name().to_string_lossy().to_string(), p.cpu_usage(), p.memory()))
        .collect();
    procs.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    let top: Vec<Proc> = procs
        .into_iter()
        .take(5)
        .enumerate()
        .map(|(i, (name, cpu, mem))| Proc {
            name,
            cpu,
            gpu: 0.0,
            ram: mem / 1_000_000, // bytes -> MB
            color: CHIP_PALETTE[i % CHIP_PALETTE.len()].to_string(),
        })
        .collect();

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
                util = Some(u.gpu as f32);
                mem_util = Some(u.memory as f32);
            }
            let (mut vram_pct, mut vram_used, mut vram_total) = (None, None, None);
            if let Ok(mem) = dev.memory_info() {
                vram_used = Some((mem.used as f64 / GIB) as f32);
                vram_total = Some((mem.total as f64 / GIB) as f32);
                if mem.total > 0 {
                    vram_pct = Some((mem.used as f64 / mem.total as f64 * 100.0) as f32);
                }
            }
            gpus.push(Gpu {
                name: dev.name().unwrap_or_else(|_| "GPU".into()),
                util,
                temp: dev.temperature(TemperatureSensor::Gpu).ok().map(|t| t as f32),
                power: dev.power_usage().ok().map(|mw| mw as f32 / 1000.0),
                mem_util,
                vram_pct,
                vram_used,
                vram_total,
                clock_core: dev.clock_info(Clock::Graphics).ok(),
                clock_mem: dev.clock_info(Clock::Memory).ok(),
            });
        }
    }

    // Temperatures (built fresh each tick; cheap at ~1 Hz).
    // CPU temp: max across CPU/package sensors. On Windows this is often empty
    // without a sensor driver — that's expected and handled as None in the UI.
    let components = Components::new_with_refreshed_list();
    let cpu_temp = components
        .iter()
        .filter(|c| {
            let l = c.label().to_lowercase();
            l.contains("cpu") || l.contains("package") || l.contains("tctl") || l.contains("core")
        })
        .filter_map(|c| c.temperature())
        .fold(None, |acc: Option<f32>, t| Some(acc.map_or(t, |m| m.max(t))));

    // Disks — skip removable; dedupe by mount point.
    let disks = Disks::new_with_refreshed_list();
    let mut seen = std::collections::HashSet::new();
    let disk_info: Vec<DiskInfo> = disks
        .iter()
        .filter(|d| !d.is_removable())
        .filter_map(|d| {
            let name = d.mount_point().to_string_lossy().to_string();
            if !seen.insert(name.clone()) || d.total_space() == 0 {
                return None;
            }
            Some(DiskInfo {
                name,
                free: (d.available_space() as f64 / GIB) as f32,
                total: (d.total_space() as f64 / GIB) as f32,
            })
        })
        .collect();

    Metrics {
        gpus,
        cpu: Cpu { util: cpu_util, temp: cpu_temp },
        ram,
        disks: disk_info,
        net,
        procs: top,
        cores,
    }
}

#[tauri::command]
fn get_sysinfo(state: State<Mutex<AppState>>) -> SysInfo {
    let s = state.lock().unwrap();
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
        cpu_mhz: cpu0.map(|c| c.frequency()).unwrap_or(0),
        gpu,
        gpu_driver,
        cuda,
        ram_total: (s.sys.total_memory() as f64 / GIB) as f32,
        uptime: System::uptime(),
        arch: std::env::consts::ARCH.to_string(),
    }
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

fn main() {
    let mut sys = System::new_all();
    sys.refresh_all();

    let state = AppState {
        sys,
        networks: Networks::new_with_refreshed_list(),
        nvml: Nvml::init().ok(), // None on machines without an NVIDIA driver
        last: Instant::now(),
        #[cfg(windows)]
        clock: win_perf::ClockReader::new(),
    };

    tauri::Builder::default()
        .manage(Mutex::new(state))
        .invoke_handler(tauri::generate_handler![get_metrics, get_sysinfo])
        .run(tauri::generate_context!())
        .expect("error while running Cortex");
}
