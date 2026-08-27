//! Phase 0.2: те же russh/SFTP-модули, что у приложения. Без WebView/xterm.
//! Запуск: cargo run --release --manifest-path src-tauri/Cargo.toml --bin phase0_bench -- home_server

use russh_sftp::client::SftpSession;
use serde_json::{json, Value};
use serein_lib::sftp;
use serein_lib::ssh;
use serein_lib::store;
use std::collections::HashSet;
use std::io::Write;
use std::time::{Duration, Instant};

fn jump_chain(server_id: &str) -> Result<Vec<Value>, String> {
    let mut chain = Vec::new();
    let mut seen = HashSet::new();
    let mut id = Some(server_id.to_string());
    while let Some(sid) = id {
        if !seen.insert(sid.clone()) {
            return Err("цикл jump".into());
        }
        let s = store::server_with_secrets(&sid).ok_or("сервер не найден / секреты не открылись")?;
        let next = s
            .get("proxyJump")
            .and_then(|v| v.as_str())
            .filter(|x| !x.is_empty())
            .map(|x| x.to_string());
        chain.push(s);
        id = next;
    }
    Ok(chain)
}

fn pick_server() -> Result<String, String> {
    let args: Vec<String> = std::env::args().skip(1).filter(|a| !a.starts_with('-')).collect();
    let list = store::servers_list();
    if let Some(a) = args.first() {
        if list.iter().any(|s| s.get("id").and_then(|v| v.as_str()) == Some(a.as_str())) {
            return Ok(a.clone());
        }
        if let Some(s) = list.iter().find(|s| s.get("name").and_then(|v| v.as_str()) == Some(a.as_str())) {
            return Ok(s["id"].as_str().unwrap().to_string());
        }
        return Err(format!("нет сервера {a}"));
    }
    list.iter()
        .find(|s| s.get("name").and_then(|v| v.as_str()) == Some("home_server"))
        .or_else(|| list.first())
        .and_then(|s| s.get("id").and_then(|v| v.as_str()).map(|x| x.to_string()))
        .ok_or_else(|| "в профиле нет серверов".into())
}

fn mb_s(bytes: u64, dt: Duration) -> f64 {
    let s = dt.as_secs_f64().max(1e-6);
    (bytes as f64 / s) / (1024.0 * 1024.0)
}

fn rec(name: &str, ok: bool, extra: Value) {
    let mut o = extra.as_object().cloned().unwrap_or_default();
    o.insert("name".into(), json!(name));
    o.insert("ok".into(), json!(ok));
    println!("{}", Value::Object(o));
    let _ = std::io::stdout().flush();
}

async fn copy_up(sftp: &SftpSession, local: &str, remote: &str) -> Result<u64, String> {
    sftp::copy_file_up(sftp, local, remote).await
}

async fn copy_down(
    h: &ssh::SharedHandle,
    sftp: &SftpSession,
    remote: &str,
    local: &str,
) -> Result<u64, String> {
    sftp::copy_file_down(h, sftp, remote, local).await
}

async fn ensure_dir(sftp: &SftpSession, dir: &str) -> Result<(), String> {
    let mut cur = String::new();
    for part in dir.split('/').filter(|p| !p.is_empty()) {
        cur = if cur.is_empty() && dir.starts_with('/') {
            format!("/{part}")
        } else if cur.is_empty() {
            part.to_string()
        } else {
            format!("{cur}/{part}")
        };
        let _ = sftp.create_dir(&cur).await;
    }
    Ok(())
}

fn make_sized_file(path: &std::path::Path, bytes: u64) -> Result<(), String> {
    if let Some(p) = path.parent() {
        std::fs::create_dir_all(p).map_err(|e| e.to_string())?;
    }
    let f = std::fs::File::create(path).map_err(|e| e.to_string())?;
    f.set_len(bytes).map_err(|e| e.to_string())
}

async fn bench_n_sessions(chain: &[Value], n: usize) -> Value {
    let t0 = Instant::now();
    let mut handles = Vec::new();
    let mut errors = Vec::new();
    for i in 0..n {
        match ssh::connect_client(chain.to_vec()).await {
            Ok(h) => handles.push(h),
            Err(e) => errors.push(format!("{i}: {e}")),
        }
    }
    let connect_ms = t0.elapsed().as_millis();
    let mut pings = Vec::new();
    for h in &handles {
        if let Some(ms) = ssh::ping(h).await {
            pings.push(ms);
        }
    }
    let avg = if pings.is_empty() {
        0
    } else {
        pings.iter().sum::<u32>() / pings.len() as u32
    };
    drop(handles);
    json!({
        "n": n,
        "connected": n - errors.len(),
        "connect_ms": connect_ms,
        "ping_avg_ms": avg,
        "errors": errors,
    })
}

async fn reopen(h: &ssh::SharedHandle, sftp: SftpSession) -> Result<SftpSession, String> {
    drop(sftp);
    sftp::open(h).await
}

async fn run_file_batches(
    h: &ssh::SharedHandle,
    mut sftp: SftpSession,
    local_root: &std::path::Path,
    remote_root: &str,
) {
    for (n, label) in [(1000usize, "1k"), (10000usize, "10k")] {
        let dir = local_root.join(label);
        let _ = std::fs::create_dir_all(&dir);
        for i in 0..n {
            let _ = std::fs::write(dir.join(format!("{i}.txt")), b"x");
        }
        let rdir = format!("{remote_root}/{label}");
        if let Err(e) = ensure_dir(&sftp, &rdir).await {
            rec(&format!("sftp_files_{label}"), false, json!({ "error": e }));
            continue;
        }
        let t = Instant::now();
        let mut err: Option<String> = None;
        for i in 0..n {
            let lp = dir.join(format!("{i}.txt"));
            let rp = format!("{rdir}/{i}.txt");
            if let Err(e) = copy_up(&sftp, &lp.to_string_lossy(), &rp).await {
                err = Some(e);
                break;
            }
        }
        let dt = t.elapsed();
        match err {
            None => rec(
                &format!("sftp_files_{label}"),
                true,
                json!({ "files": n, "ms": dt.as_millis(), "files_s": n as f64 / dt.as_secs_f64().max(1e-6) }),
            ),
            Some(e) => rec(&format!("sftp_files_{label}"), false, json!({ "error": e, "ms": dt.as_millis() })),
        }
        let _ = std::fs::remove_dir_all(&dir);
        match reopen(h, sftp).await {
            Ok(s) => sftp = s,
            Err(e) => {
                rec("sftp_reopen", false, json!({ "error": e, "after": label }));
                return;
            }
        }
    }
}

#[tokio::main]
async fn main() {
    let sid = match pick_server() {
        Ok(s) => s,
        Err(e) => {
            rec("init", false, json!({ "error": e }));
            std::process::exit(1);
        }
    };
    let name = store::servers_list()
        .iter()
        .find(|s| s["id"] == sid)
        .and_then(|s| s["name"].as_str())
        .unwrap_or("?")
        .to_string();
    rec("init", true, json!({ "server": name, "note": "russh path, no WebView" }));

    let chain = match jump_chain(&sid) {
        Ok(c) => c,
        Err(e) => {
            rec("connect", false, json!({ "error": e }));
            std::process::exit(1);
        }
    };

    let files_only = std::env::args().any(|a| a == "--files");
    if files_only {
        let h = match ssh::connect_client(chain.clone()).await {
            Ok(h) => h,
            Err(e) => {
                rec("ssh_shell", false, json!({ "error": e }));
                std::process::exit(1);
            }
        };
        let local_root = std::env::temp_dir().join("serein-phase0-bench");
        let _ = std::fs::create_dir_all(&local_root);
        let remote_root = "/tmp/serein-phase0-bench";
        let _ = ssh::exec(&h, &format!("mkdir -p {remote_root}")).await;
        let sftp = match sftp::open(&h).await {
            Ok(s) => s,
            Err(e) => {
                rec("sftp_open", false, json!({ "error": e }));
                return;
            }
        };
        run_file_batches(&h, sftp, &local_root, remote_root).await;
        let _ = ssh::exec(&h, &format!("rm -rf {remote_root}")).await;
        let _ = std::fs::remove_dir_all(&local_root);
        rec("done", true, json!({ "mode": "files" }));
        return;
    }

    let t = Instant::now();
    let mut h = match ssh::connect_client(chain.clone()).await {
        Ok(h) => h,
        Err(e) => {
            rec("ssh_shell", false, json!({ "error": e }));
            std::process::exit(1);
        }
    };
    let connect_ms = t.elapsed().as_millis();
    let ping = ssh::ping(&h).await;
    rec(
        "ssh_shell",
        true,
        json!({ "connect_ms": connect_ms, "ping_ms": ping, "note": "exec channel, not PTY" }),
    );

    for n in [5usize, 10, 20] {
        let extra = bench_n_sessions(&chain, n).await;
        let ok = extra["connected"].as_u64().unwrap_or(0) == n as u64;
        rec(&format!("ssh_x{n}"), ok, extra);
    }

    let stream = Duration::from_secs(8);
    match ssh::exec_for(&h, "timeout 8 yes", stream).await {
        Ok((bytes, code)) => rec(
            "term_flood",
            true,
            json!({ "bytes": bytes, "mb_s": mb_s(bytes, stream), "exit": code, "secs": 8 }),
        ),
        Err(e) => rec("term_flood", false, json!({ "error": e })),
    }

    match ssh::exec(&h, "docker ps -q 2>/dev/null | head -1").await {
        Ok((0, id, _)) if !id.trim().is_empty() => {
            let cid = id.trim().to_string();
            let cmd = format!("timeout 8 docker logs -f {cid}");
            match ssh::exec_for(&h, &cmd, stream).await {
                Ok((bytes, code)) => rec(
                    "docker_logs_f",
                    true,
                    json!({ "bytes": bytes, "mb_s": mb_s(bytes, stream), "exit": code, "container": &cid[..8.min(cid.len())] }),
                ),
                Err(e) => rec("docker_logs_f", false, json!({ "error": e })),
            }
        }
        Ok((_, out, err)) => rec(
            "docker_logs_f",
            false,
            json!({ "error": "нет контейнера", "stdout_len": out.len(), "stderr_len": err.len() }),
        ),
        Err(e) => rec("docker_logs_f", false, json!({ "error": e })),
    }

    match ssh::exec_for(&h, "timeout 8 journalctl -f --no-pager", stream).await {
        Ok((bytes, code)) => rec(
            "journalctl_f",
            true,
            json!({ "bytes": bytes, "mb_s": mb_s(bytes, stream), "exit": code, "secs": 8 }),
        ),
        Err(e) => rec("journalctl_f", false, json!({ "error": e })),
    }

    let local_root = std::env::temp_dir().join("serein-phase0-bench");
    let _ = std::fs::remove_dir_all(&local_root);
    let _ = std::fs::create_dir_all(&local_root);
    let remote_root = "/tmp/serein-phase0-bench";
    let _ = ssh::exec(&h, &format!("rm -rf {remote_root} && mkdir -p {remote_root}")).await;

    let mut sftp = match sftp::open(&h).await {
        Ok(s) => s,
        Err(e) => {
            rec("sftp_open", false, json!({ "error": e }));
            return;
        }
    };
    rec("sftp_open", true, json!({ "request_timeout_s": 300 }));

    for mb in [10u64, 100, 1024] {
        let fname = format!("blob-{mb}m.bin");
        let lp = local_root.join(&fname);
        let rp = format!("{remote_root}/{fname}");
        if let Err(e) = make_sized_file(&lp, mb * 1024 * 1024) {
            rec(&format!("sftp_up_{mb}m"), false, json!({ "error": e }));
            continue;
        }
        let t = Instant::now();
        match copy_up(&sftp, &lp.to_string_lossy(), &rp).await {
            Ok(n) => {
                let dt = t.elapsed();
                rec(
                    &format!("sftp_up_{mb}m"),
                    true,
                    json!({ "bytes": n, "ms": dt.as_millis(), "mb_s": mb_s(n, dt) }),
                );
            }
            Err(e) => rec(&format!("sftp_up_{mb}m"), false, json!({ "error": e })),
        }
        sftp = match reopen(&h, sftp).await {
            Ok(s) => s,
            Err(e) => {
                rec("sftp_reopen", false, json!({ "error": e, "after": format!("up_{mb}m") }));
                return;
            }
        };
        let down = local_root.join(format!("down-{fname}"));
        rec("sftp_down_start", true, json!({ "mb": mb }));
        let t = Instant::now();
        let down_limit = if mb >= 1024 { Duration::from_secs(600) } else { Duration::from_secs(180) };
        let down_res = tokio::time::timeout(down_limit, copy_down(&h, &sftp, &rp, &down.to_string_lossy())).await;
        match down_res {
            Ok(Ok(n)) => {
                let dt = t.elapsed();
                rec(
                    &format!("sftp_down_{mb}m"),
                    true,
                    json!({ "bytes": n, "ms": dt.as_millis(), "mb_s": mb_s(n, dt) }),
                );
            }
            Ok(Err(e)) => rec(&format!("sftp_down_{mb}m"), false, json!({ "error": e })),
            Err(_) => rec(
                &format!("sftp_down_{mb}m"),
                false,
                json!({ "error": format!("timeout {}s", down_limit.as_secs()) }),
            ),
        }
        let _ = std::fs::remove_file(&lp);
        let _ = std::fs::remove_file(&down);
        sftp = match reopen(&h, sftp).await {
            Ok(s) => s,
            Err(e) => {
                rec("sftp_reopen", false, json!({ "error": e, "after": format!("down_{mb}m") }));
                return;
            }
        };
    }

    drop(sftp);
    h = match ssh::connect_client(chain.clone()).await {
        Ok(x) => x,
        Err(e) => {
            rec("ssh_reconnect", false, json!({ "error": e, "after": "before_files" }));
            return;
        }
    };
    let sftp = match sftp::open(&h).await {
        Ok(s) => s,
        Err(e) => {
            rec("sftp_open", false, json!({ "error": e, "after": "before_files" }));
            return;
        }
    };
    rec("ssh_reconnect", true, json!({ "after": "before_files" }));
    run_file_batches(&h, sftp, &local_root, remote_root).await;

    let _ = ssh::exec(&h, &format!("rm -rf {remote_root}")).await;
    let _ = std::fs::remove_dir_all(&local_root);
    rec("done", true, json!({}));
}
