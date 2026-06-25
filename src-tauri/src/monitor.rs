//! Снимок метрик Linux-сервера одной командой (порт monitor.ts).

use serde_json::{json, Value};

pub const SAMPLE_CMD: &str = concat!(
    "echo \"N:$(nproc 2>/dev/null || echo 1)\"; ",
    "echo \"L:$(cat /proc/loadavg 2>/dev/null)\"; ",
    "echo \"MT:$(grep MemTotal /proc/meminfo 2>/dev/null | awk '{print $2}')\"; ",
    "echo \"MA:$(grep MemAvailable /proc/meminfo 2>/dev/null | awk '{print $2}')\"; ",
    "echo \"D:$(df -P / 2>/dev/null | tail -1 | awk '{print $5}')\"; ",
    "echo \"U:$(cat /proc/uptime 2>/dev/null | awk '{print $1}')\"; ",
    "A=$(head -1 /proc/stat 2>/dev/null); sleep 0.4; B=$(head -1 /proc/stat 2>/dev/null); ",
    "echo \"CA:$A\"; echo \"CB:$B\""
);

fn cpu_from_stat(a: &str, b: &str) -> u32 {
    let pa: Vec<f64> = a.split_whitespace().skip(1).filter_map(|x| x.parse().ok()).collect();
    let pb: Vec<f64> = b.split_whitespace().skip(1).filter_map(|x| x.parse().ok()).collect();
    if pa.len() < 5 || pb.len() < 5 {
        return 0;
    }
    let total_a: f64 = pa.iter().sum();
    let total_b: f64 = pb.iter().sum();
    let idle_a = pa[3] + pa[4];
    let idle_b = pb[3] + pb[4];
    let d_total = total_b - total_a;
    let d_idle = idle_b - idle_a;
    if d_total <= 0.0 {
        return 0;
    }
    (((1.0 - d_idle / d_total) * 100.0).round()).clamp(0.0, 100.0) as u32
}

pub fn parse(stdout: &str) -> Value {
    let mut tags: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    for line in stdout.lines() {
        if let Some(idx) = line.find(':') {
            tags.insert(line[..idx].trim().to_string(), line[idx + 1..].trim().to_string());
        }
    }
    let get = |k: &str| tags.get(k).cloned().unwrap_or_default();
    let cores: u64 = get("N").parse().unwrap_or(1).max(1);
    let load: Vec<f64> = get("L").split_whitespace().filter_map(|x| x.parse().ok()).collect();
    let load3 = [
        load.first().copied().unwrap_or(0.0),
        load.get(1).copied().unwrap_or(0.0),
        load.get(2).copied().unwrap_or(0.0),
    ];
    let mem_total: u64 = get("MT").parse().unwrap_or(0);
    let mem_avail: u64 = get("MA").parse().unwrap_or(0);
    let mem_used = if mem_total > 0 { mem_total.saturating_sub(mem_avail) } else { 0 };
    let disk_pct: u32 = get("D").replace('%', "").parse().unwrap_or(0);
    let uptime: f64 = get("U").parse().unwrap_or(0.0);
    let cpu = cpu_from_stat(&get("CA"), &get("CB"));

    json!({
        "ok": true,
        "cores": cores,
        "cpuPct": cpu,
        "load": load3,
        "memTotalKb": mem_total,
        "memUsedKb": mem_used,
        "diskPct": disk_pct,
        "uptimeSec": uptime.round() as u64,
    })
}
