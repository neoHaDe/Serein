//! Запасной путь файлового менеджера: сервер без подсистемы SFTP.
//!
//! Ради этого пути и написан `scp.rs`: старые прошивки и урезанные образы часто идут без
//! `Subsystem sftp`, и файловый менеджер обязан работать через `scp` и `ls`, а не
//! показывать пустой каталог. Проверить это можно только на таком сервере — в стенде для
//! него отдельный контейнер, у которого подсистема вырезана из конфигурации.
//!
//! Запуск: `cargo test --test scp_stand -- --ignored --test-threads=1`

mod common;
use common::{rt, Stand};

use serein_lib::remote_fs::{self, SessionFs};
use serein_lib::ssh;
use std::sync::{Arc, Mutex};

/// Подключение к серверу без SFTP плюс свежее состояние выбора бэкенда.
async fn connect(s: &Stand) -> (ssh::SharedHandle, Arc<Mutex<SessionFs>>) {
    let h = ssh::connect_client(vec![s.by_key(s.nosftp_port)])
        .await
        .expect("подключение к серверу без SFTP");
    (h, Arc::new(Mutex::new(SessionFs::new())))
}

fn scratch(name: &str) -> String {
    format!("/tmp/serein-scp-{name}")
}

#[test]
#[ignore = "нужен стенд: scripts/ssh-stand/up.sh"]
fn falls_back_to_scp_when_sftp_is_missing() {
    // Главная проверка: приложение само понимает, что подсистемы нет, и уходит на SCP.
    // Если бы оно этого не делало, пользователь видел бы пустой каталог вместо файлов.
    let s = Stand::from_env();
    rt().block_on(async {
        let (h, fs) = connect(&s).await;
        let listed = remote_fs::list(&fs, &h, "/tmp").await.expect("листинг");
        assert_eq!(
            listed.get("backend").and_then(|v| v.as_str()),
            Some("scp"),
            "на сервере без SFTP листинг обязан идти через SCP"
        );
    });
}

#[test]
#[ignore = "нужен стенд: scripts/ssh-stand/up.sh"]
fn file_survives_a_round_trip_through_scp() {
    // Запись и чтение через SCP, с именем в юникоде и пробелами: путь уходит в командную
    // строку, поэтому именно такие имена ломают экранирование, если оно неверное.
    let s = Stand::from_env();
    let dir = scratch("круговой тест");
    rt().block_on(async {
        let (h, fs) = connect(&s).await;
        let _ = remote_fs::remove(&fs, &h, &dir, true).await;
        remote_fs::mkdir(&fs, &h, &dir).await.expect("каталог");

        let file = format!("{dir}/файл с пробелами.txt");
        let text = "содержимое через SCP ✓";
        remote_fs::write_file(&fs, &h, &file, text, 0o644, 0, "lf")
            .await
            .expect("запись");

        let read = remote_fs::read_file(&fs, &h, &file).await.expect("чтение");
        assert_eq!(read.get("content").and_then(|v| v.as_str()), Some(text));

        remote_fs::remove(&fs, &h, &dir, true).await.expect("уборка");
    });
}

#[test]
#[ignore = "нужен стенд: scripts/ssh-stand/up.sh"]
fn quoting_survives_a_name_that_looks_like_a_shell_trick() {
    // Имя файла попадает в командную строку. Если экранирование дырявое, кавычка и `$(…)`
    // превратятся в выполнение команды на сервере — здесь это ловится сразу.
    let s = Stand::from_env();
    let dir = scratch("кавычки");
    rt().block_on(async {
        let (h, fs) = connect(&s).await;
        let _ = remote_fs::remove(&fs, &h, &dir, true).await;
        remote_fs::mkdir(&fs, &h, &dir).await.expect("каталог");

        // Слэша в имени быть не может — это разделитель пути, и SCP передаёт имя отдельным
        // полем. Всё остальное, чем ломают командную строку, здесь есть.
        let evil = "it's $(id) && echo pwned; `whoami`.txt";
        let tricky = format!("{dir}/{evil}");
        remote_fs::write_file(&fs, &h, &tricky, "безобидно", 0o644, 0, "lf")
            .await
            .expect("запись файла со странным именем");

        // Имя должно лежать на сервере ровно таким, каким его задали: если подстановка
        // выполнилась, в каталоге окажется что-то другое — или не окажется ничего.
        let listed = remote_fs::list(&fs, &h, &dir).await.expect("листинг");
        let names: Vec<String> = listed
            .get("entries")
            .and_then(|v| v.as_array())
            .expect("entries")
            .iter()
            .filter_map(|e| e.get("name").and_then(|v| v.as_str()).map(str::to_string))
            .collect();
        assert_eq!(names, vec![evil.to_string()], "имя файла на сервере разошлось с заданным");

        let read = remote_fs::read_file(&fs, &h, &tricky).await.expect("чтение");
        assert_eq!(read.get("content").and_then(|v| v.as_str()), Some("безобидно"));

        remote_fs::remove(&fs, &h, &dir, true).await.expect("уборка");
    });
}

#[test]
#[ignore = "нужен стенд: scripts/ssh-stand/up.sh"]
fn sftp_server_still_uses_sftp() {
    // Обратная проверка: там, где подсистема есть, на SCP уходить незачем.
    let s = Stand::from_env();
    rt().block_on(async {
        let h = ssh::connect_client(vec![s.by_key(s.debian_port)])
            .await
            .expect("подключение к обычному серверу");
        let fs = Arc::new(Mutex::new(SessionFs::new()));
        let listed = remote_fs::list(&fs, &h, "/tmp").await.expect("листинг");
        assert_eq!(listed.get("backend").and_then(|v| v.as_str()), Some("sftp"));
    });
}
