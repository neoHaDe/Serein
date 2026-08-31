//! Снимок метрик Linux-сервера одной командой (порт monitor.ts).
//! P1.3: OS, ядро, сеть, число процессов, упавшие сервисы, Docker — в том же exec.

use serde_json::{json, Value};

pub const SAMPLE_CMD: &str = concat!(
    "echo \"N:$(nproc 2>/dev/null || echo 1)\"; ",
    "echo \"L:$(cat /proc/loadavg 2>/dev/null)\"; ",
    "echo \"MT:$(grep MemTotal /proc/meminfo 2>/dev/null | awk '{print $2}')\"; ",
    "echo \"MA:$(grep MemAvailable /proc/meminfo 2>/dev/null | awk '{print $2}')\"; ",
    "echo \"D:$(df -P / 2>/dev/null | tail -1 | awk '{print $5}')\"; ",
    "echo \"U:$(cat /proc/uptime 2>/dev/null | awk '{print $1}')\"; ",
    "A=$(head -1 /proc/stat 2>/dev/null); sleep 0.4; B=$(head -1 /proc/stat 2>/dev/null); ",
    "echo \"CA:$A\"; echo \"CB:$B\"; ",
    "echo \"OS:$(. /etc/os-release 2>/dev/null; echo ${PRETTY_NAME:-$(uname -sr)}))\"; ",
    "echo \"K:$(uname -r 2>/dev/null)\"; ",
    "echo \"P:$(ps -e --no-headers 2>/dev/null | wc -l | tr -d ' \\n')\"; ",
    "IF=$(ip -o route get 1.1.1.1 2>/dev/null | awk '{for(i=1;i<=NF;i++) if($i==\"dev\") print $(i+1)}'); ",
    "if [ -n \"$IF\" ] && [ -r /proc/net/dev ]; then ",
    "  echo \"IF:$IF\"; ",
    "  awk -v d=\"$IF:\" '$1==d {print \"RX:\"$2; print \"TX:\"$10}' /proc/net/dev; ",
    "fi; ",
    "echo \"SF:$(systemctl --failed --no-legend --no-pager 2>/dev/null | wc -l | tr -d ' \\n')\"; ",
    "if command -v docker >/dev/null 2>&1; then ",
    "  echo \"DR:$(docker ps -q 2>/dev/null | wc -l | tr -d ' \\n')\"; ",
    "  echo \"DE:$(docker ps -aq --filter status=exited 2>/dev/null | wc -l | tr -d ' \\n')\"; ",
    "else echo \"DR:-1\"; echo \"DE:-1\"; fi"
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

fn opt_u64(s: &str) -> Option<u64> {
    let t = s.trim();
    if t.is_empty() {
        return None;
    }
    t.parse().ok()
}

fn opt_u32(s: &str) -> Option<u32> {
    opt_u64(s).and_then(|n| u32::try_from(n).ok())
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
    let mem_used = if mem_total > 0 {
        mem_total.saturating_sub(mem_avail)
    } else {
        0
    };
    let disk_pct: u32 = get("D").replace('%', "").parse().unwrap_or(0);
    let uptime: f64 = get("U").parse().unwrap_or(0.0);
    let cpu = cpu_from_stat(&get("CA"), &get("CB"));

    let os = get("OS");
    let kernel = get("K");
    let proc_count = opt_u32(&get("P"));
    let net_iface = get("IF");
    let net_rx = opt_u64(&get("RX"));
    let net_tx = opt_u64(&get("TX"));
    let failed_services = opt_u32(&get("SF"));

    let dr = opt_i32(&get("DR"));
    let de = opt_i32(&get("DE"));
    let (docker_running, docker_stopped, docker_available) = match (dr, de) {
        (Some(-1), _) | (_, Some(-1)) => (None, None, false),
        (Some(r), Some(e)) => (Some(r.max(0) as u32), Some(e.max(0) as u32), true),
        _ => (None, None, false),
    };

    let mut out = json!({
        "ok": true,
        "cores": cores,
        "cpuPct": cpu,
        "load": load3,
        "memTotalKb": mem_total,
        "memUsedKb": mem_used,
        "diskPct": disk_pct,
        "uptimeSec": uptime.round() as u64,
    });
    if !os.is_empty() {
        out["os"] = json!(os);
    }
    if !kernel.is_empty() {
        out["kernel"] = json!(kernel);
    }
    if let Some(n) = proc_count {
        out["procCount"] = json!(n);
    }
    if !net_iface.is_empty() {
        out["netIface"] = json!(net_iface);
    }
    if let Some(n) = net_rx {
        out["netRxBytes"] = json!(n);
    }
    if let Some(n) = net_tx {
        out["netTxBytes"] = json!(n);
    }
    if let Some(n) = failed_services {
        out["failedServices"] = json!(n);
    }
    out["dockerAvailable"] = json!(docker_available);
    if let Some(n) = docker_running {
        out["dockerRunning"] = json!(n);
    }
    if let Some(n) = docker_stopped {
        out["dockerStopped"] = json!(n);
    }
    out
}

fn opt_i32(s: &str) -> Option<i32> {
    let t = s.trim();
    if t.is_empty() {
        return None;
    }
    t.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_extended_overview_fields() {
        let sample = concat!(
            "N:4\nL:0.42 0.38 0.35\nMT:8000000\nMA:4000000\nD:52%\nU:86400.5\n",
            "CA:cpu 100 20 30 40 50 0 0 0\nCB:cpu 200 40 60 80 100 0 0 0\n",
            "OS:Debian GNU/Linux 12 (bookworm)\nK:6.1.0-18-amd64\nP:142\n",
            "IF:eth0\nRX:1234567890\nTX:987654321\nSF:2\nDR:3\nDE:1\n"
        );
        let v = parse(sample);
        assert_eq!(v["ok"], true);
        assert_eq!(v["os"], "Debian GNU/Linux 12 (bookworm)");
        assert_eq!(v["kernel"], "6.1.0-18-amd64");
        assert_eq!(v["procCount"], 142);
        assert_eq!(v["netIface"], "eth0");
        assert_eq!(v["netRxBytes"], 1234567890_u64);
        assert_eq!(v["failedServices"], 2);
        assert_eq!(v["dockerRunning"], 3);
        assert_eq!(v["dockerStopped"], 1);
        assert!(v["dockerAvailable"].as_bool().unwrap());
    }

    #[test]
    fn docker_absent_is_flagged() {
        let sample = "N:1\nL:0.1 0.1 0.1\nMT:1000\nMA:500\nD:10%\nU:100\nCA:a\nCB:b\nDR:-1\nDE:-1\n";
        let v = parse(sample);
        assert!(!v["dockerAvailable"].as_bool().unwrap());
        assert!(v.get("dockerRunning").is_none());
    }
}
