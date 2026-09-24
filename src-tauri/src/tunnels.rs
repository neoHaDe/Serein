//! Туннели: local (-L), dynamic SOCKS5 (-D) и remote (-R) поверх russh.
//! -L/-D - direct-tcpip; -R - tcpip_forward + маршрутизация forwarded-каналов через ClientHandler.

use crate::ssh::{wait_cancel, CancelRx, ClientHandler, RemoteForwards};
use russh::client;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::oneshot;

struct TunnelEntry {
    stop: Option<oneshot::Sender<()>>,
    active: bool,
    error: Option<String>,
}

/// Вид туннеля из поля `type` конфига.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// `-L`: порт на этой машине, соединение идёт на `remoteHost` со стороны сервера.
    Local,
    /// `-R`: порт на сервере, соединение приходит на петлю этой машины.
    Remote,
    /// SOCKS5: адрес выбирает клиент в каждом запросе.
    Dynamic,
}

impl Kind {
    /// Нет поля - `local`, как было всегда. Незнакомое значение - отказ, а не «значит,
    /// локальный»: раньше тип «L» или «forward» открывал локальный проброс, а проверку
    /// политики, завязанную на `local`, обходил.
    pub fn of(cfg: &Value) -> Result<Kind, String> {
        match cfg.get("type") {
            None | Some(Value::Null) => Ok(Kind::Local),
            Some(v) => match v.as_str() {
                Some("local") => Ok(Kind::Local),
                Some("remote") => Ok(Kind::Remote),
                Some("dynamic") => Ok(Kind::Dynamic),
                _ => Err(format!("неизвестный тип туннеля: {v}")),
            },
        }
    }
}

/// Какой адрес туннеля проверяет политика до открытия. У `-L` адрес назначения задан в
/// конфиге. У `-R` цель - петля самой этой машины, проверять нечего. У SOCKS5 адрес приходит
/// в каждом запросе клиента, и проверка стоит на каждом соединении (`handle_socks5`).
pub fn policy_target(kind: Kind, cfg: &Value) -> Option<&str> {
    (kind == Kind::Local).then(|| cfg.get("remoteHost").and_then(|v| v.as_str()).unwrap_or(""))
}

#[derive(Default)]
pub struct TunnelManager {
    active: Mutex<HashMap<String, HashMap<String, TunnelEntry>>>,
}

fn emit(app: &AppHandle, session_id: &str, tunnel_id: &str, active: bool, error: Option<&str>) {
    let _ = app.emit(
        "tunnel-status",
        serde_json::json!({ "sessionId": session_id, "tunnelId": tunnel_id, "active": active, "error": error }),
    );
}

impl TunnelManager {
    pub fn list_status(&self, session_id: &str) -> Vec<Value> {
        crate::sync::lock(&self.active)
            .get(session_id)
            .map(|m| {
                m.iter()
                    .map(|(tid, t)| {
                        serde_json::json!({ "sessionId": session_id, "tunnelId": tid, "active": t.active, "error": t.error })
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn close(&self, session_id: &str, tunnel_id: &str, app: &AppHandle) {
        if let Some(map) = crate::sync::lock(&self.active).get_mut(session_id) {
            if let Some(mut t) = map.remove(tunnel_id) {
                if let Some(stop) = t.stop.take() {
                    let _ = stop.send(());
                }
            }
        }
        emit(app, session_id, tunnel_id, false, None);
    }

    pub fn close_session(&self, session_id: &str, app: &AppHandle) {
        if let Some(mut map) = crate::sync::lock(&self.active).remove(session_id) {
            for (tid, mut t) in map.drain() {
                if let Some(stop) = t.stop.take() {
                    let _ = stop.send(());
                }
                emit(app, session_id, &tid, false, None);
            }
        }
    }

    /// Открывает (или перезапускает) туннель по конфигу.
    pub async fn open(
        &self,
        app: AppHandle,
        handle: Arc<tokio::sync::Mutex<client::Handle<ClientHandler>>>,
        session_id: String,
        cfg: Value,
        remote_forwards: RemoteForwards,
        cancel: CancelRx,
    ) -> Result<(), String> {
        let tunnel_id = cfg.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let local_port = cfg.get("localPort").and_then(|v| v.as_u64()).unwrap_or(0) as u16;

        self.close(&session_id, &tunnel_id, &app);

        let (stop_tx, mut stop_rx) = oneshot::channel::<()>();
        {
            let mut guard = crate::sync::lock(&self.active);
            let m = guard.entry(session_id.clone()).or_default();
            m.insert(
                tunnel_id.clone(),
                TunnelEntry {
                    stop: Some(stop_tx),
                    active: false,
                    error: None,
                },
            );
        }

        // Тип и политика - здесь, в единственной точке открытия: сюда приходят и кнопка, и
        // туннели, которые поднимаются при подключении. Раньше проверка стояла только у кнопки,
        // и туннель из профиля открывался при входе мимо `allowedHosts`.
        let checked = Kind::of(&cfg).and_then(|kind| {
            if let Some(target) = policy_target(kind, &cfg) {
                crate::policy::check_target(target, "туннель")?;
            }
            Ok(kind)
        });
        let kind = match checked {
            Ok(kind) => kind,
            Err(msg) => {
                self.set_error(&session_id, &tunnel_id, &msg);
                emit(&app, &session_id, &tunnel_id, false, Some(&msg));
                return Err(msg);
            }
        };

        // ---- Remote (-R remotePort:127.0.0.1:localPort) ----
        if kind == Kind::Remote {
            let remote_port = cfg.get("remotePort").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
            crate::sync::lock(&remote_forwards).insert(remote_port, local_port);
            // tcpip_forward требует &mut - лочим Handle на время вызова.
            let fwd_res = tokio::select! {
                r = async {
                    let h = handle.lock().await;
                    h.tcpip_forward("127.0.0.1", remote_port).await
                } => Some(r),
                _ = &mut stop_rx => None,
                _ = wait_cancel(cancel.clone()) => None,
            };
            match fwd_res {
                None => {
                    crate::sync::lock(&remote_forwards).remove(&remote_port);
                    {
                        let h = handle.lock().await;
                        let _ = h.cancel_tcpip_forward("127.0.0.1", remote_port).await;
                    }
                    return Err("Отменено".into());
                }
                Some(Err(e)) => {
                    crate::sync::lock(&remote_forwards).remove(&remote_port);
                    let msg = format!("Remote-форвард не удался: {e}");
                    self.set_error(&session_id, &tunnel_id, &msg);
                    emit(&app, &session_id, &tunnel_id, false, Some(&msg));
                    return Err(msg);
                }
                Some(Ok(_)) => {
                    if !self.mark_active(&session_id, &tunnel_id) {
                        crate::sync::lock(&remote_forwards).remove(&remote_port);
                        {
                            let h = handle.lock().await;
                            let _ = h.cancel_tcpip_forward("127.0.0.1", remote_port).await;
                        }
                        return Err("Отменено".into());
                    }
                    emit(&app, &session_id, &tunnel_id, true, None);
                    let h2 = handle.clone();
                    let rf = remote_forwards.clone();
                    let cancel_r = cancel.clone();
                    tokio::spawn(async move {
                        tokio::select! {
                            _ = stop_rx => {}
                            _ = wait_cancel(cancel_r) => {}
                        }
                        {
                            let g = h2.lock().await;
                            let _ = g.cancel_tcpip_forward("127.0.0.1", remote_port).await;
                        }
                        crate::sync::lock(&rf).remove(&remote_port);
                    });
                    return Ok(());
                }
            }
        }

        let bind = tokio::select! {
            r = TcpListener::bind(("127.0.0.1", local_port)) => r,
            _ = &mut stop_rx => {
                return Err("Отменено".into());
            }
            _ = wait_cancel(cancel.clone()) => {
                return Err("Отменено".into());
            }
        };
        let listener = match bind {
            Ok(l) => l,
            Err(e) => {
                let msg = format!("Не удалось занять порт {local_port}: {e}");
                self.set_error(&session_id, &tunnel_id, &msg);
                emit(&app, &session_id, &tunnel_id, false, Some(&msg));
                return Err(msg);
            }
        };

        if !self.mark_active(&session_id, &tunnel_id) {
            return Err("Отменено".into());
        }
        emit(&app, &session_id, &tunnel_id, true, None);

        let remote_host = cfg.get("remoteHost").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let remote_port = cfg.get("remotePort").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
        let is_dynamic = kind == Kind::Dynamic;
        let cancel_loop = cancel.clone();

        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = &mut stop_rx => break,
                    _ = wait_cancel(cancel_loop.clone()) => break,
                    accepted = listener.accept() => {
                        let (sock, _) = match accepted { Ok(v) => v, Err(_) => break };
                        let h = handle.clone();
                        let rh = remote_host.clone();
                        let c = cancel.clone();
                        if is_dynamic {
                            tokio::spawn(async move { let _ = handle_socks5(sock, h, c).await; });
                        } else {
                            tokio::spawn(async move {
                                let ch = {
                                    let g = h.lock().await;
                                    g.channel_open_direct_tcpip(rh.as_str(), remote_port, "127.0.0.1", 0).await
                                };
                                if let Ok(ch) = ch {
                                    let mut stream = ch.into_stream();
                                    let mut sock = sock;
                                    tokio::select! {
                                        _ = tokio::io::copy_bidirectional(&mut sock, &mut stream) => {}
                                        _ = wait_cancel(c) => {}
                                    }
                                }
                            });
                        }
                    }
                }
            }
        });
        Ok(())
    }

    fn mark_active(&self, session_id: &str, tunnel_id: &str) -> bool {
        let mut guard = crate::sync::lock(&self.active);
        if let Some(t) = guard.get_mut(session_id).and_then(|m| m.get_mut(tunnel_id)) {
            t.active = true;
            true
        } else {
            false
        }
    }

    fn set_error(&self, session_id: &str, tunnel_id: &str, err: &str) {
        let mut guard = crate::sync::lock(&self.active);
        let m = guard.entry(session_id.to_string()).or_default();
        m.insert(
            tunnel_id.to_string(),
            TunnelEntry {
                stop: None,
                active: false,
                error: Some(err.to_string()),
            },
        );
    }
}

/// Минимальный SOCKS5: greeting → request (CONNECT) → direct-tcpip → bidi-pipe.
async fn handle_socks5(
    mut sock: tokio::net::TcpStream,
    handle: Arc<tokio::sync::Mutex<client::Handle<ClientHandler>>>,
    cancel: CancelRx,
) -> Result<(), String> {
    let mut head = [0u8; 2];
    sock.read_exact(&mut head).await.map_err(|e| e.to_string())?;
    if head[0] != 0x05 {
        return Err("Не SOCKS5".into());
    }
    let nmethods = head[1] as usize;
    let mut methods = vec![0u8; nmethods];
    sock.read_exact(&mut methods).await.map_err(|e| e.to_string())?;
    sock.write_all(&[0x05, 0x00]).await.map_err(|e| e.to_string())?; // no auth

    let mut req = [0u8; 4];
    sock.read_exact(&mut req).await.map_err(|e| e.to_string())?;
    let atyp = req[3];
    let host = match atyp {
        0x01 => {
            let mut a = [0u8; 4];
            sock.read_exact(&mut a).await.map_err(|e| e.to_string())?;
            format!("{}.{}.{}.{}", a[0], a[1], a[2], a[3])
        }
        0x03 => {
            let mut l = [0u8; 1];
            sock.read_exact(&mut l).await.map_err(|e| e.to_string())?;
            let mut d = vec![0u8; l[0] as usize];
            sock.read_exact(&mut d).await.map_err(|e| e.to_string())?;
            String::from_utf8_lossy(&d).to_string()
        }
        0x04 => {
            let mut a = [0u8; 16];
            sock.read_exact(&mut a).await.map_err(|e| e.to_string())?;
            let segs: Vec<String> = (0..8)
                .map(|i| format!("{:x}", u16::from_be_bytes([a[i * 2], a[i * 2 + 1]])))
                .collect();
            segs.join(":")
        }
        _ => {
            sock.write_all(&[0x05, 0x08, 0x00, 0x01, 0, 0, 0, 0, 0, 0]).await.ok();
            return Err("Неподдерживаемый тип адреса".into());
        }
    };
    let mut pbuf = [0u8; 2];
    sock.read_exact(&mut pbuf).await.map_err(|e| e.to_string())?;
    let port = u16::from_be_bytes(pbuf) as u32;

    // Политика администратора. У динамического туннеля адрес выбирает не конфиг, а сам
    // клиент в каждом запросе, поэтому проверка здесь, а не при открытии туннеля.
    if let Err(e) = crate::policy::check_target(&host, "туннель SOCKS5") {
        // 0x02 - «соединение запрещено правилами»: браузер покажет отказ сразу, а не
        // будет ждать таймаута.
        sock.write_all(&[0x05, 0x02, 0x00, 0x01, 0, 0, 0, 0, 0, 0]).await.ok();
        return Err(e);
    }

    let ch = {
        let g = handle.lock().await;
        g.channel_open_direct_tcpip(host.as_str(), port, "127.0.0.1", 0).await
    };
    match ch {
        Ok(ch) => {
            sock.write_all(&[0x05, 0x00, 0x00, 0x01, 0, 0, 0, 0, 0, 0]).await.ok();
            let mut stream = ch.into_stream();
            tokio::select! {
                _ = tokio::io::copy_bidirectional(&mut sock, &mut stream) => {}
                _ = wait_cancel(cancel) => {}
            }
            Ok(())
        }
        Err(_) => {
            sock.write_all(&[0x05, 0x05, 0x00, 0x01, 0, 0, 0, 0, 0, 0]).await.ok();
            Err("Канал не открылся".into())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn тип_туннеля_разбирается_строго() {
        assert_eq!(
            Kind::of(&json!({})).unwrap(),
            Kind::Local,
            "нет поля - local, как раньше"
        );
        assert_eq!(Kind::of(&json!({ "type": null })).unwrap(), Kind::Local);
        assert_eq!(Kind::of(&json!({ "type": "local" })).unwrap(), Kind::Local);
        assert_eq!(Kind::of(&json!({ "type": "remote" })).unwrap(), Kind::Remote);
        assert_eq!(Kind::of(&json!({ "type": "dynamic" })).unwrap(), Kind::Dynamic);
        // Раньше всё это открывалось локальным пробросом без проверки политики.
        for bad in [json!("L"), json!("forward"), json!("Local"), json!(""), json!(5)] {
            assert!(Kind::of(&json!({ "type": bad })).is_err(), "{bad}");
        }
    }

    #[test]
    fn политика_смотрит_на_адрес_только_у_локального_проброса() {
        let cfg = json!({ "remoteHost": "192.168.1.1" });
        assert_eq!(policy_target(Kind::Local, &cfg), Some("192.168.1.1"));
        assert_eq!(
            policy_target(Kind::Local, &json!({})),
            Some(""),
            "без адреса - пустой, и политика его отклонит"
        );
        assert_eq!(policy_target(Kind::Remote, &cfg), None);
        assert_eq!(policy_target(Kind::Dynamic, &cfg), None);
    }
}
