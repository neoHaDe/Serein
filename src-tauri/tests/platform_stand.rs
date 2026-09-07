//! Определитель платформы — против настоящих систем стенда.
//!
//! Модульные тесты проверяют разбор на заранее записанных ответах, а этот — что зонд вообще
//! выполняется в чужой оболочке и возвращает то, по чему можно судить. Ровно здесь ломаются
//! догадки: команда, которая красиво работает в bash, на BusyBox может промолчать.
//!
//! Запуск: `cargo test --test platform_stand -- --ignored --test-threads=1`

mod common;
use common::{rt, Stand};

use serein_lib::platform::{self, Kind};
use serein_lib::ssh;

async fn probe(s: &Stand, port: u16) -> (Kind, String) {
    let h = ssh::connect_client(vec![s.by_key(port)])
        .await
        .expect("подключение к серверу");
    let (_, out, _) = ssh::exec(&h, platform::PROBE_CMD, None)
        .await
        .expect("зонд платформы");
    platform::detect(&out)
}

#[test]
#[ignore = "нужен стенд: scripts/ssh-stand/up.sh"]
fn debian_определяется_как_linux() {
    let s = Stand::from_env();
    rt().block_on(async {
        let (kind, version) = probe(&s, s.debian_port).await;
        assert_eq!(kind, Kind::Linux, "версия: {version}");
        assert!(version.to_lowercase().contains("linux"), "версия пустая: {version}");
    });
}

#[test]
#[ignore = "нужен стенд: scripts/ssh-stand/up.sh"]
fn alpine_определяется_как_busybox() {
    // Alpine — это и есть тот случай, ради которого различение заведено: `uname` говорит
    // «Linux», а утилиты там свои, с урезанным набором ключей.
    let s = Stand::from_env();
    rt().block_on(async {
        let (kind, version) = probe(&s, s.alpine_port).await;
        assert_eq!(kind, Kind::BusyBox, "версия: {version}");
    });
}

#[test]
#[ignore = "нужен стенд: scripts/ssh-stand/up.sh"]
fn зонд_не_считает_posix_оболочку_виндой() {
    // `echo %OS%` в POSIX печатает саму строку «%OS%». Принять её за Windows значит
    // отправить на Linux-сервер команды PowerShell — и получить пустые панели.
    let s = Stand::from_env();
    rt().block_on(async {
        for port in [s.debian_port, s.alpine_port, s.nosftp_port] {
            let (kind, _) = probe(&s, port).await;
            assert_ne!(kind, Kind::Windows, "порт {port} принят за Windows");
        }
    });
}

async fn run(s: &Stand, port: u16, cmd: &str) -> String {
    let h = ssh::connect_client(vec![s.by_key(port)])
        .await
        .expect("подключение к серверу");
    let (_, out, _) = ssh::exec(&h, cmd, None).await.expect("команда");
    out
}

#[test]
#[ignore = "нужен стенд: scripts/ssh-stand/up.sh"]
fn список_процессов_на_busybox_не_пустой() {
    // Ровно та ошибка, ради которой заведена отдельная ветка команд: `ps -eo ... --sort`
    // BusyBox не понимает и отвечает справкой в поток ошибок. Разбор такой справки давал
    // ноль строк, и панель показывала пустую таблицу без единого сообщения — то есть
    // выглядела как исправная. Проверять это можно только на живой системе.
    let s = Stand::from_env();
    rt().block_on(async {
        let out = run(&s, s.alpine_port, platform::cmd::PS_BUSYBOX).await;
        let v = platform::busybox::parse_ps(&out);
        let rows = v["rows"].as_array().expect("строки процессов");
        assert!(!rows.is_empty(), "BusyBox отдал пустой список процессов:\n{out}");
        assert!(
            rows.iter().any(|r| r["cmd"].as_str().unwrap_or("").contains("sshd")),
            "среди процессов нет sshd, а мы к нему подключены:\n{out}"
        );
        // Загрузку процессора BusyBox не сообщает — на этом месте обязан быть null.
        assert!(rows[0]["cpu"].is_null(), "взялась загрузка, которой неоткуда взяться");
    });
}

#[test]
#[ignore = "нужен стенд: scripts/ssh-stand/up.sh"]
fn старая_команда_процессов_на_busybox_действительно_ломается() {
    // Обратная сторона теста выше: если однажды BusyBox научится ключам procps, ветка
    // станет лишней и об этом лучше узнать от упавшего теста, чем никогда.
    let s = Stand::from_env();
    rt().block_on(async {
        let out = run(&s, s.alpine_port, serein_lib::workspace::PS_CMD).await;
        let v = serein_lib::workspace::parse_ps(&out);
        assert!(
            v["rows"].as_array().unwrap().is_empty(),
            "команда procps внезапно заработала на BusyBox — ветку можно убирать"
        );
    });
}

#[test]
#[ignore = "нужен стенд: scripts/ssh-stand/up.sh"]
fn отсутствие_openrc_объясняется_а_не_показывается_пустой_таблицей() {
    let s = Stand::from_env();
    rt().block_on(async {
        let out = run(&s, s.alpine_port, platform::cmd::SERVICES_BUSYBOX).await;
        let v = platform::busybox::parse_services(&out);
        assert_eq!(v["ok"], false, "ответ стенда: {out:?}");
        assert!(v["error"].as_str().unwrap_or("").contains("OpenRC"));
    });
}

#[test]
#[ignore = "нужен стенд: scripts/ssh-stand/up.sh"]
fn журнал_находится_и_там_где_нет_journalctl() {
    // На Alpine нет ни journalctl, ни /var/log/syslog. Раньше панель логов показывала
    // на этом «Нет journalctl и syslog» и всё; теперь остаётся кольцевой буфер BusyBox.
    let s = Stand::from_env();
    rt().block_on(async {
        let out = run(&s, s.alpine_port, serein_lib::workspace::LOGS_CMD).await;
        assert!(!out.trim().is_empty(), "команда журнала не ответила вовсе");
        // Хоть что-то: либо строки журнала, либо честное объяснение, почему их нет.
        assert!(
            out.contains("Журнал недоступен") || out.lines().count() > 1,
            "непонятный ответ журнала:\n{out}"
        );
    });
}

#[test]
#[ignore = "нужен стенд: scripts/ssh-stand/up.sh"]
fn снимок_нагрузки_на_busybox_считает_процессы() {
    // `ps -e --no-headers` на BusyBox молчал, и счётчик процессов показывал ноль —
    // при живом сервере с работающими процессами.
    let s = Stand::from_env();
    rt().block_on(async {
        let out = run(&s, s.alpine_port, serein_lib::monitor::SAMPLE_CMD).await;
        let v = serein_lib::monitor::parse(&out);
        let n = v["procCount"].as_u64().unwrap_or(0);
        assert!(n > 0, "процессов насчитано {n}, ответ стенда:\n{out}");
        assert!(v["memTotalKb"].as_u64().unwrap_or(0) > 0, "не прочиталась память");
        // systemd на Alpine нет, и плитка упавших служб не должна выдумывать ноль.
        assert!(
            v.get("failedServices").is_none(),
            "без systemd не бывает и списка упавших служб"
        );
    });
}
