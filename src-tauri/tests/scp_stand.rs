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
use serein_lib::scp;
use serein_lib::ssh;
use std::sync::atomic::{AtomicU32, Ordering};
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
        assert_eq!(
            names,
            vec![evil.to_string()],
            "имя файла на сервере разошлось с заданным"
        );

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

#[test]
#[ignore = "нужен стенд: scripts/ssh-stand/up.sh"]
fn время_правки_читается_и_без_подсистемы_sftp() {
    // Здесь это не метаданные протокола, а разбор вывода `stat` через оболочку - ровно то
    // место, где легко получить пустую строку и молча счесть, что файл не менялся.
    let s = Stand::from_env();
    let dir = scratch("время правки");
    rt().block_on(async {
        let (h, fs) = connect(&s).await;
        let _ = remote_fs::remove(&fs, &h, &dir, true).await;
        remote_fs::mkdir(&fs, &h, &dir).await.expect("каталог");
        let file = format!("{dir}/файл.txt");
        remote_fs::write_file(&fs, &h, &file, "раз", 0o644, 0, "lf")
            .await
            .expect("запись");

        let t = remote_fs::remote_mtime(&fs, &h, &file)
            .await
            .expect("запрос времени")
            .expect("сервер обязан сообщить время правки");
        assert!(t > 1_600_000_000_000, "время похоже на миллисекунды эпохи: {t}");

        remote_fs::remove(&fs, &h, &dir, true).await.expect("уборка");
    });
}

#[test]
#[ignore = "нужен стенд: scripts/ssh-stand/up.sh"]
fn обход_scp_не_заходит_в_ссылки_и_сообщает_о_них() {
    // Ссылка на родительский каталог раньше закручивала обход SCP, а пропущенное не
    // попадало никуда. Теперь в ссылку не заходим и говорим об этом в списке отказов.
    let s = Stand::from_env();
    let dir = scratch("обход");
    rt().block_on(async {
        let (h, _) = connect(&s).await;
        let cmd = format!(
            "rm -rf '{dir}' && mkdir -p '{dir}/sub' && echo x > '{dir}/sub/f.txt' && ln -s '{dir}' '{dir}/sub/loop'"
        );
        let (code, _, err) = ssh::exec(&h, &cmd, None).await.expect("дерево на сервере");
        assert_eq!(code, 0, "дерево не создалось: {err}");

        let (jobs, refused) = scp::walk_remote(&h, &dir, "/local/обход", "обход", None)
            .await
            .expect("обход");
        assert_eq!(jobs.len(), 1, "один настоящий файл: {jobs:?}");
        assert_eq!(jobs[0].2, "обход/sub/f.txt");
        assert!(
            refused
                .iter()
                .any(|(r, why)| r == "обход/sub/loop" && why.contains("ссылка")),
            "ссылка названа в отказах: {refused:?}"
        );
    });
}

/// Своя временная папка на этой машине.
fn local_scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("serein-scp-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("своя папка");
    dir
}

#[test]
#[ignore = "нужен стенд: scripts/ssh-stand/up.sh"]
fn большой_файл_по_scp_идёт_потоком_и_возвращается_целым() {
    // Раньше файл целиком лежал в памяти и по пути туда, и обратно. Здесь несколько
    // мегабайт неповторяющегося содержимого: сдвиг или потерянный кусок сразу виден.
    let s = Stand::from_env();
    let dir = scratch("поток");
    rt().block_on(async {
        let (h, fs) = connect(&s).await;
        let _ = remote_fs::remove(&fs, &h, &dir, true).await;
        remote_fs::mkdir(&fs, &h, &dir).await.expect("каталог");
        let local = local_scratch("поток");
        let src = local.join("big.bin");
        let data: Vec<u8> = (0..5 * 1024 * 1024 + 7u64)
            .map(|i| ((i * 2_654_435_761) >> 13) as u8)
            .collect();
        std::fs::write(&src, &data).unwrap();

        let remote = format!("{dir}/big.bin");
        let mut seen = 0;
        scp::put_file_ctl(
            &h,
            src.to_str().unwrap(),
            &remote,
            &|| true,
            &mut |done: u64, _: u64| seen = done,
        )
        .await
        .expect("заливка");
        assert_eq!(seen, data.len() as u64, "ход передачи дошёл до конца");

        let back = local.join("back.bin");
        scp::download_file(&h, &remote, back.to_str().unwrap())
            .await
            .expect("скачивание");
        assert!(
            std::fs::read(&back).unwrap() == data,
            "содержимое вернулось без искажений"
        );

        let listed = remote_fs::list(&fs, &h, &dir).await.expect("листинг");
        let names: Vec<&str> = listed["entries"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|e| e["name"].as_str())
            .collect();
        assert_eq!(names, vec!["big.bin"], "временных файлов на сервере не осталось");

        let _ = std::fs::remove_dir_all(&local);
        remote_fs::remove(&fs, &h, &dir, true).await.expect("уборка");
    });
}

#[test]
#[ignore = "нужен стенд: scripts/ssh-stand/up.sh"]
fn прерванная_заливка_по_scp_оставляет_прежний_файл_а_удачная_его_права() {
    // Замена идёт через временный файл: отмена посреди передачи не оставляет на сервере
    // половину нового файла под именем старого, а удачная заливка не сбрасывает права.
    let s = Stand::from_env();
    let dir = scratch("замена");
    rt().block_on(async {
        let (h, fs) = connect(&s).await;
        let _ = remote_fs::remove(&fs, &h, &dir, true).await;
        remote_fs::mkdir(&fs, &h, &dir).await.expect("каталог");
        let remote = format!("{dir}/run.sh");
        let (code, _, err) = ssh::exec(
            &h,
            &format!("printf 'старое' > '{remote}' && chmod 750 '{remote}'"),
            None,
        )
        .await
        .expect("исходный файл");
        assert_eq!(code, 0, "{err}");

        let local = local_scratch("замена");
        let src = local.join("run.sh");
        std::fs::write(&src, vec![b'x'; 1024 * 1024]).unwrap();
        let checks = AtomicU32::new(0);
        let live = || checks.fetch_add(1, Ordering::Relaxed) < 3;
        scp::put_file_ctl(&h, src.to_str().unwrap(), &remote, &live, &mut |_: u64, _: u64| {})
            .await
            .expect_err("заливка отменена посреди файла");

        let state = format!("cat '{remote}'; echo; stat -c %a '{remote}'; ls -A '{dir}'");
        let (_, out, _) = ssh::exec(&h, &state, None).await.expect("состояние");
        assert_eq!(out.trim(), "старое\n750\nrun.sh", "оригинал цел, временного файла нет");

        std::fs::write(&src, "новое").unwrap();
        scp::put_file(&h, src.to_str().unwrap(), &remote)
            .await
            .expect("заливка");
        let (_, out, _) = ssh::exec(&h, &state, None).await.expect("состояние");
        assert_eq!(out.trim(), "новое\n750\nrun.sh", "содержимое заменено, права прежние");

        let _ = std::fs::remove_dir_all(&local);
        remote_fs::remove(&fs, &h, &dir, true).await.expect("уборка");
    });
}
