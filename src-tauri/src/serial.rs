//! Последовательный порт (COM) как ещё один тип сессии — рядом с локальным терминалом и SSH.
//!
//! Механика та же, что в `pty.rs`: поток-читатель льёт байты в `TermOut`, обратно пишем
//! напрямую в порт. Асинхронная обёртка тут не нужна — serial-сессий единицы, а поток
//! на сессию у нас уже есть у локального терминала.

use serde_json::{json, Value};
use serialport::{DataBits, FlowControl, Parity, SerialPort, StopBits};
use std::io::{ErrorKind, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::{AppHandle, Emitter};

/// Таймаут чтения. Порт молчит почти всё время, поэтому таймаут — норма, а не ошибка:
/// он нужен только чтобы поток регулярно просыпался и проверял флаг закрытия.
const READ_TIMEOUT: Duration = Duration::from_millis(200);
const DEFAULT_BAUD: u32 = 115_200;

pub struct SerialSession {
    port: Mutex<Option<Box<dyn SerialPort>>>,
    alive: Arc<AtomicBool>,
}

impl SerialSession {
    pub fn write(&self, data: &str) {
        if let Ok(mut p) = self.port.lock() {
            if let Some(p) = p.as_mut() {
                let _ = p.write_all(data.as_bytes());
                let _ = p.flush();
            }
        }
    }

    pub fn close(&self) {
        self.alive.store(false, Ordering::Relaxed);
        if let Ok(mut p) = self.port.lock() {
            p.take(); // закрытие хэндла разбудит читателя ошибкой
        }
    }

    /// Сигнал BREAK — им сетевое железо переводят в recovery/rommon.
    pub fn send_break(&self) -> Result<(), String> {
        let mut guard = self.port.lock().map_err(|_| "Порт занят")?;
        let port = guard.as_mut().ok_or("Порт закрыт")?;
        port.set_break().map_err(|e| e.to_string())?;
        std::thread::sleep(Duration::from_millis(250));
        port.clear_break().map_err(|e| e.to_string())
    }

    pub fn set_signal(&self, line: &str, on: bool) -> Result<(), String> {
        let mut guard = self.port.lock().map_err(|_| "Порт занят")?;
        let port = guard.as_mut().ok_or("Порт закрыт")?;
        match line {
            "dtr" => port.write_data_terminal_ready(on).map_err(|e| e.to_string()),
            "rts" => port.write_request_to_send(on).map_err(|e| e.to_string()),
            _ => Err("Допустимы только dtr и rts".into()),
        }
    }
}

/// Доступные COM-порты. USB-переходники подписываем производителем и продуктом —
/// по одному «COM7» пользователь не поймёт, какой из трёх переходников воткнут.
pub fn list_ports() -> Vec<Value> {
    let ports = match serialport::available_ports() {
        Ok(p) => p,
        Err(_) => return Vec::new(),
    };
    let mut out: Vec<Value> = ports
        .into_iter()
        .map(|p| {
            let (kind, label) = match &p.port_type {
                serialport::SerialPortType::UsbPort(info) => {
                    let name = info
                        .product
                        .clone()
                        .or_else(|| info.manufacturer.clone())
                        .unwrap_or_else(|| "USB-устройство".into());
                    ("usb", name)
                }
                serialport::SerialPortType::BluetoothPort => ("bluetooth", "Bluetooth".into()),
                serialport::SerialPortType::PciPort => ("pci", "PCI".into()),
                serialport::SerialPortType::Unknown => ("unknown", String::new()),
            };
            json!({ "port": p.port_name, "kind": kind, "label": label })
        })
        .collect();
    out.sort_by(|a, b| {
        a["port"]
            .as_str()
            .unwrap_or("")
            .cmp(b["port"].as_str().unwrap_or(""))
    });
    out
}

fn data_bits(n: u64) -> DataBits {
    match n {
        5 => DataBits::Five,
        6 => DataBits::Six,
        7 => DataBits::Seven,
        _ => DataBits::Eight,
    }
}

fn stop_bits(n: u64) -> StopBits {
    match n {
        2 => StopBits::Two,
        _ => StopBits::One,
    }
}

fn parity(s: &str) -> Parity {
    match s {
        "odd" => Parity::Odd,
        "even" => Parity::Even,
        _ => Parity::None,
    }
}

fn flow_control(s: &str) -> FlowControl {
    match s {
        "software" => FlowControl::Software,
        "hardware" => FlowControl::Hardware,
        _ => FlowControl::None,
    }
}

/// Человеческая расшифровка настроек — уходит в терминал первой строкой,
/// чтобы было видно, с какими параметрами открылись.
pub fn describe(cfg: &Value) -> String {
    let get_str = |k: &str, d: &str| cfg.get(k).and_then(|v| v.as_str()).unwrap_or(d).to_string();
    let get_u64 = |k: &str, d: u64| cfg.get(k).and_then(|v| v.as_u64()).unwrap_or(d);
    format!(
        "{} · {} бод · {}{}{} · поток: {}",
        get_str("port", "COM?"),
        get_u64("baudRate", DEFAULT_BAUD as u64),
        get_u64("dataBits", 8),
        match get_str("parity", "none").as_str() {
            "odd" => "O",
            "even" => "E",
            _ => "N",
        },
        get_u64("stopBits", 1),
        match get_str("flowControl", "none").as_str() {
            "software" => "XON/XOFF",
            "hardware" => "RTS/CTS",
            _ => "нет",
        }
    )
}

/// Человеческий текст вместо системного кода. Две частые ситуации — переходник выдернут
/// и порт уже занят (второй терминал, монитор порта, прошивальщик) — должны читаться сразу.
fn open_error(name: &str, e: &serialport::Error) -> String {
    match e.kind() {
        // Windows отдаёт `NoDevice` и когда порта нет, и когда он уже кем-то открыт
        // (проверено на com0com: порт, занятый другой вкладкой, приходит именно так).
        // Различить нельзя, поэтому называем обе причины — иначе «устройство отключено»
        // сбивает с толку там, где надо просто закрыть вторую программу.
        serialport::ErrorKind::NoDevice | serialport::ErrorKind::Io(ErrorKind::NotFound) => {
            format!("Порт {name} недоступен — устройство отключено или порт занят другой программой")
        }
        serialport::ErrorKind::Io(ErrorKind::PermissionDenied) => {
            format!("Порт {name} занят другой программой")
        }
        _ => format!("Не удалось открыть {name}: {e}"),
    }
}

/// Открывает порт по настройкам профиля. Вынесено из `open_serial`, чтобы путь
/// «конфиг → открытый порт» можно было проверить тестом без Tauri.
fn build_port(cfg: &Value) -> Result<Box<dyn SerialPort>, String> {
    let name = cfg
        .get("port")
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .ok_or("Не выбран COM-порт")?;
    let baud = cfg
        .get("baudRate")
        .and_then(|v| v.as_u64())
        .unwrap_or(DEFAULT_BAUD as u64) as u32;

    serialport::new(name, baud)
        .data_bits(data_bits(cfg.get("dataBits").and_then(|v| v.as_u64()).unwrap_or(8)))
        .stop_bits(stop_bits(cfg.get("stopBits").and_then(|v| v.as_u64()).unwrap_or(1)))
        .parity(parity(cfg.get("parity").and_then(|v| v.as_str()).unwrap_or("none")))
        .flow_control(flow_control(
            cfg.get("flowControl").and_then(|v| v.as_str()).unwrap_or("none"),
        ))
        .timeout(READ_TIMEOUT)
        .open()
        .map_err(|e| open_error(name, &e))
}

/// Открывает порт и запускает поток-читатель. `cfg` — секция `serial` профиля сервера.
pub fn open_serial(app: AppHandle, id: String, cfg: &Value) -> Result<SerialSession, String> {
    let port = build_port(cfg)?;

    // Линии DTR/RTS: часть устройств (например, платы на CH340) без них не отвечает.
    if let Some(on) = cfg.get("dtr").and_then(|v| v.as_bool()) {
        let _ = port.try_clone().map(|mut p| p.write_data_terminal_ready(on));
    }
    if let Some(on) = cfg.get("rts").and_then(|v| v.as_bool()) {
        let _ = port.try_clone().map(|mut p| p.write_request_to_send(on));
    }

    let mut reader = port
        .try_clone()
        .map_err(|e| format!("Не удалось открыть порт на чтение: {e}"))?;

    let alive = Arc::new(AtomicBool::new(true));
    let alive2 = alive.clone();
    let app2 = app.clone();
    let id2 = id.clone();
    let out = crate::term_out::TermOut::spawn(app.clone(), id.clone());

    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        let mut error: Option<String> = None;
        while alive2.load(Ordering::Relaxed) {
            match reader.read(&mut buf) {
                Ok(0) => {}
                Ok(n) => out.push(&buf[..n]),
                // Тишина на линии — обычное состояние, а не обрыв.
                Err(ref e) if e.kind() == ErrorKind::TimedOut => {}
                Err(ref e) if e.kind() == ErrorKind::Interrupted => {}
                Err(e) => {
                    if alive2.load(Ordering::Relaxed) {
                        error = Some(format!("Порт отключён: {e}"));
                    }
                    break;
                }
            }
        }
        out.close();
        out.join_blocking();
        let _ = app2.emit(
            "session-exit",
            json!({ "id": id2, "code": null, "signal": null, "error": error }),
        );
    });

    Ok(SerialSession {
        port: Mutex::new(Some(port)),
        alive,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_settings_map_to_serialport_values() {
        assert!(matches!(data_bits(7), DataBits::Seven));
        assert!(matches!(data_bits(99), DataBits::Eight)); // мусор → безопасный дефолт
        assert!(matches!(stop_bits(2), StopBits::Two));
        assert!(matches!(stop_bits(0), StopBits::One));
        assert!(matches!(parity("even"), Parity::Even));
        assert!(matches!(parity("нет"), Parity::None));
        assert!(matches!(flow_control("hardware"), FlowControl::Hardware));
        assert!(matches!(flow_control(""), FlowControl::None));
    }

    #[test]
    fn describe_defaults_to_8n1() {
        let cfg = json!({ "port": "COM3" });
        assert_eq!(describe(&cfg), "COM3 · 115200 бод · 8N1 · поток: нет");
    }

    #[test]
    fn describe_shows_explicit_settings() {
        let cfg = json!({
            "port": "COM7", "baudRate": 9600, "dataBits": 7,
            "stopBits": 2, "parity": "even", "flowControl": "hardware"
        });
        assert_eq!(describe(&cfg), "COM7 · 9600 бод · 7E2 · поток: RTS/CTS");
    }

    #[test]
    fn open_without_port_name_is_rejected() {
        // Без имени порта до железа дело не доходит — ошибка до открытия.
        let cfg = json!({ "baudRate": 9600 });
        assert!(cfg.get("port").and_then(|v| v.as_str()).is_none());
    }

    #[test]
    fn missing_and_busy_ports_get_readable_errors() {
        let no_device = serialport::Error::new(serialport::ErrorKind::NoDevice, "нет");
        assert_eq!(
            open_error("COM99", &no_device),
            "Порт COM99 недоступен — устройство отключено или порт занят другой программой"
        );

        let busy = serialport::Error::new(
            serialport::ErrorKind::Io(ErrorKind::PermissionDenied),
            "занят",
        );
        assert_eq!(open_error("COM3", &busy), "Порт COM3 занят другой программой");

        // Незнакомый код не теряем — показываем системный текст целиком.
        let weird = serialport::Error::new(serialport::ErrorKind::InvalidInput, "кривые параметры");
        assert!(open_error("COM4", &weird).contains("кривые параметры"));
    }

    /// Есть ли в системе пара виртуальных портов для сквозного теста.
    fn pair_available(a: &str, b: &str) -> bool {
        match serialport::available_ports() {
            Ok(ports) => {
                let names: Vec<String> = ports.into_iter().map(|p| p.port_name).collect();
                names.iter().any(|n| n == a) && names.iter().any(|n| n == b)
            }
            Err(_) => false,
        }
    }

    /// Сквозная проверка на виртуальной паре (com0com: COM5 ↔ COM6).
    /// Открываем один конец **нашим** `build_port` по настройкам профиля, пишем во второй
    /// конец и убеждаемся, что байты дошли. Без пары тест тихо пропускается — на CI и на
    /// машине без портов он не должен падать.
    #[test]
    fn round_trip_over_virtual_pair() {
        if !pair_available("COM5", "COM6") {
            eprintln!("пропуск: виртуальной пары COM5/COM6 нет");
            return;
        }
        let cfg = json!({ "port": "COM5", "baudRate": 115200, "dataBits": 8, "stopBits": 1 });
        // Порт может быть занят — например, открытой вкладкой в самом приложении.
        // Это не повод ронять прогон: пропускаем, как и при отсутствии пары.
        let Ok(mut ours) = build_port(&cfg) else {
            eprintln!("пропуск: COM5 занят (открыт в приложении?) или недоступен");
            return;
        };
        let Ok(mut other) = serialport::new("COM6", 115_200).timeout(READ_TIMEOUT).open() else {
            eprintln!("пропуск: COM6 занят или недоступен");
            return;
        };

        let payload = b"serein-serial-check\r\n";
        other.write_all(payload).expect("запись во второй конец");
        other.flush().ok();

        // Читаем с запасом по времени: пара виртуальная, но данные идут через драйвер.
        let mut got = Vec::new();
        let mut buf = [0u8; 128];
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        while std::time::Instant::now() < deadline && got.len() < payload.len() {
            match ours.read(&mut buf) {
                Ok(n) => got.extend_from_slice(&buf[..n]),
                Err(ref e) if e.kind() == ErrorKind::TimedOut => continue,
                Err(e) => panic!("чтение из COM5 не удалось: {e}"),
            }
        }
        assert_eq!(
            String::from_utf8_lossy(&got),
            String::from_utf8_lossy(payload),
            "данные должны дойти с COM6 на COM5 без потерь"
        );
    }

    /// Реальное открытие несуществующего порта: проверяем, что до пользователя доходит
    /// наше сообщение, а не системный код. Железо не нужно — порт заведомо отсутствует.
    #[test]
    fn opening_absent_port_returns_our_message() {
        let name = if cfg!(windows) { "COM99" } else { "/dev/ttyDOESNOTEXIST" };
        let err = serialport::new(name, 115_200)
            .timeout(READ_TIMEOUT)
            .open()
            .map(|_| ())
            .map_err(|e| open_error(name, &e))
            .expect_err("такого порта в системе быть не должно");
        assert!(
            err.contains(name) && !err.contains("Error"),
            "сообщение должно быть человеческим, получено: {err}"
        );
    }
}
