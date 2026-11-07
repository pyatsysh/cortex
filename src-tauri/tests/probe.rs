//! Runs the built binary in probe mode: the real platform sampler, no window, under a
//! relocated home whose path holds spaces, a quote and non-ASCII letters.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn fake_home() -> PathBuf {
    let home = Path::new(env!("CARGO_TARGET_TMPDIR")).join("home with spaces 'q' ünïcode");
    std::fs::create_dir_all(&home).unwrap();
    home
}

/// The binary under test never sees a display or a session bus: a regression that opened a
/// window would then fail here instead of appearing on the desktop of whoever runs the tests.
fn cortex() -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_cortex"));
    for v in ["DISPLAY", "WAYLAND_DISPLAY", "XAUTHORITY", "DBUS_SESSION_BUS_ADDRESS"] {
        c.env_remove(v);
    }
    c
}

fn probe(extra_env: &[(&str, &OsStr)], args: &[&str]) -> Output {
    let home = fake_home();
    let mut c = cortex();
    c.arg("--probe")
        .args(args)
        .env("HOME", &home)
        .env("XDG_CONFIG_HOME", home.join(".config"))
        .env("XDG_DATA_HOME", home.join(".local/share"))
        .env("XDG_STATE_HOME", home.join(".local/state"))
        .env("XDG_CACHE_HOME", home.join(".cache"));
    for (k, v) in extra_env {
        c.env(k, v);
    }
    c.output().expect("probe runs")
}

fn json_lines(out: &Output) -> Vec<serde_json::Value> {
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(|l| serde_json::from_str(l).expect("each probe line is one JSON object"))
        .collect()
}

#[test]
fn probe_without_nvidia_library_reports_gpu_unavailable_and_no_stand_in_zeros() {
    let missing = Path::new(env!("CARGO_TARGET_TMPDIR")).join("no nvml here").join("libnvidia-ml.so.1");
    let out = probe(&[("CORTEX_NVML_LIB", missing.as_os_str())], &["--samples", "3", "--interval-ms", "200"]);
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let v = json_lines(&out);
    assert_eq!(v.len(), 5, "sysinfo, capabilities, three samples");
    assert_eq!(v[0]["kind"], "sysinfo");
    assert_eq!(v[0]["data"]["gpu"], "—");
    assert_eq!(v[1]["kind"], "capabilities");
    let caps = v[1]["items"].as_array().unwrap();
    let gpu = caps.iter().find(|c| c["capability"] == "gpu-telemetry").expect("gpu capability listed");
    assert_eq!(gpu["state"], "unavailable");
    assert!(gpu["reason"].as_str().unwrap().contains("no nvml here"));
    for c in caps {
        let state = c["state"].as_str().unwrap();
        assert!(["available", "unavailable", "degraded", "unsupported", "disabled", "unknown"].contains(&state));
        if state != "available" {
            assert!(!c["reason"].as_str().unwrap().is_empty(), "{c}");
        }
    }
    let samples: Vec<_> = v.iter().filter(|x| x["kind"] == "metrics").collect();
    assert_eq!(samples.len(), 3);
    for m in samples {
        let d = &m["data"];
        assert!(d["gpus"].as_array().unwrap().is_empty());
        assert!(d["interval_ms"].as_u64().unwrap() >= 150, "{}", d["interval_ms"]);
        for core in d["cores"].as_array().unwrap() {
            assert_ne!(core["freq"], 0, "a clock is reported or null, never zero");
        }
        if let Some(t) = d["cpu"]["temp"].as_f64() {
            assert!(t > 0.0 && t < 150.0);
        }
        assert!(d["ram"]["total"].as_f64().unwrap() > 0.0);
        assert!(!d["disks"].as_array().unwrap().is_empty());
        for p in d["procs"].as_array().unwrap() {
            assert!(p["gpu"].is_null());
        }
    }
}

#[test]
fn help_prints_usage_and_opens_no_window() {
    for flag in ["--help", "-h"] {
        let out = cortex().arg(flag).env("HOME", fake_home()).output().expect("cortex runs");
        assert_eq!(out.status.code(), Some(0), "{flag}: {}", String::from_utf8_lossy(&out.stderr));
        let usage = String::from_utf8_lossy(&out.stdout);
        assert!(usage.starts_with("usage: cortex") && usage.contains("--probe"), "{flag}: {usage}");
        assert!(out.stderr.is_empty(), "{flag}: {}", String::from_utf8_lossy(&out.stderr));
    }
}

#[test]
fn any_other_argument_is_refused_with_usage_and_exit_2() {
    for args in [&["--version"][..], &["--helpp"], &["--help", "extra"], &["file.txt"], &["-x"]] {
        let out = cortex().args(args).env("HOME", fake_home()).output().expect("cortex runs");
        assert_eq!(out.status.code(), Some(2), "{args:?}");
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(err.contains("unknown arguments") && err.contains("usage: cortex"), "{args:?}: {err}");
        assert!(out.stdout.is_empty(), "{args:?}");
    }
}

#[test]
fn probe_refuses_bad_arguments_with_exit_2() {
    let out = probe(&[], &["--samples", "zero"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("--samples"));
    assert!(out.stdout.is_empty());
}

/// Each sample re-reads the per-core clocks (they once stayed at the start-up values for the whole
/// run). The kernel's cpufreq files are read as each sample arrives; if they moved during the run,
/// the probe's clocks must have moved too. Skips where there is no cpufreq or nothing moved.
#[cfg(target_os = "linux")]
#[test]
fn probe_clocks_move_when_the_kernel_clocks_move() {
    use std::io::{BufRead, BufReader};
    use std::process::Stdio;
    let kernel = || -> Option<Vec<u64>> {
        (0..)
            .map_while(|i| {
                let cpu = format!("/sys/devices/system/cpu/cpu{i}");
                Path::new(&cpu).exists().then(|| {
                    std::fs::read_to_string(format!("{cpu}/cpufreq/scaling_cur_freq"))
                        .ok()
                        .and_then(|s| s.trim().parse::<u64>().ok())
                })
            })
            .collect()
    };
    if kernel().map_or(true, |v| v.is_empty()) {
        eprintln!("skipped: no cpufreq clocks on this machine");
        return;
    }
    let home = fake_home();
    let mut child = cortex()
        .args(["--probe", "--samples", "8", "--interval-ms", "150"])
        .env("HOME", &home)
        .env("XDG_CONFIG_HOME", home.join(".config"))
        .env("XDG_DATA_HOME", home.join(".local/share"))
        .stdout(Stdio::piped())
        .spawn()
        .expect("probe runs");
    let (mut probe_clocks, mut kernel_clocks) = (Vec::new(), Vec::new());
    for line in BufReader::new(child.stdout.take().unwrap()).lines() {
        let v: serde_json::Value = serde_json::from_str(&line.unwrap()).unwrap();
        if v["kind"] == "metrics" {
            kernel_clocks.push(kernel());
            let cores: Vec<Option<u64>> = v["data"]["cores"].as_array().unwrap().iter().map(|c| c["freq"].as_u64()).collect();
            probe_clocks.push(cores);
        }
    }
    assert!(child.wait().unwrap().success());
    assert_eq!(probe_clocks.len(), 8);
    fn distinct<T: std::hash::Hash + Eq>(xs: &[T]) -> usize {
        xs.iter().collect::<std::collections::HashSet<_>>().len()
    }
    if distinct(&kernel_clocks) < 2 {
        eprintln!("skipped: the kernel's clocks did not move during the run");
        return;
    }
    assert!(
        distinct(&probe_clocks) > 1,
        "the probe repeated {:?} for every sample while the kernel's clocks moved: {:?}",
        probe_clocks[0],
        kernel_clocks
    );
}
