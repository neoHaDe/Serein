//! Края SFTP против настоящих серверов.
//!
//! Здесь намеренно нет «залил файл, скачал файл» в чистом виде — этот путь ломается
//! редко. Ломается всё остальное: имена в юникоде, битые симлинки, отказ прав, глубокие
//! деревья. Именно на них SFTP-панель и спотыкалась в жизни.
//!
//! Запуск: `cargo test --test sftp_stand -- --ignored` с переменными из `up.sh`.

mod common;
use common::{rt, Stand};

use serein_lib::sftp;
use serein_lib::ssh;

/// Подключиться к Debian ключом. Alpine для SFTP не берём: там sftp-подсистема тоже есть,
/// но проверяем края протокола, а не различия дистрибутивов.
async fn connect(s: &Stand) -> ssh::SharedHandle {
    ssh::connect_client(vec![s.by_key(s.debian_port)])
        .await
        .expect("подключение к стенду")
}

/// Уникальный каталог на прогон: тесты идут параллельно и не должны мешать друг другу.
fn scratch(name: &str) -> String {
    format!("/tmp/serein-sftp-{name}")
}

#[test]
#[ignore = "нужен стенд: scripts/ssh-stand/up.sh"]
fn unicode_and_spaces_in_names_survive_a_round_trip() {
    // Имена с кириллицей, пробелами и эмодзи: если где-то путь склеивается через шелл или
    // кодируется не в UTF-8, ломается именно здесь.
    let s = Stand::from_env();
    let dir = scratch("юникод тест 🔑");
    rt().block_on(async {
        let h = connect(&s).await;
        let _ = sftp::remove(&h, &dir, true).await;
        sftp::mkdir(&h, &dir).await.expect("создание каталога");

        let file = format!("{dir}/файл с пробелами.txt");
        sftp::write_file(&h, &file, "содержимое с юникодом ✓", 0o644, 0, "lf")
            .await
            .expect("запись файла");

        let read = sftp::read_file(&h, &file).await.expect("чтение файла");
        assert_eq!(
            read.get("content").and_then(|v| v.as_str()),
            Some("содержимое с юникодом ✓")
        );

        let listed = sftp::list(&h, &dir).await.expect("листинг");
        let names: Vec<String> = listed
            .get("entries")
            .and_then(|v| v.as_array())
            .expect("entries")
            .iter()
            .filter_map(|e| e.get("name").and_then(|v| v.as_str()).map(str::to_string))
            .collect();
        assert!(
            names.iter().any(|n| n == "файл с пробелами.txt"),
            "имя приехало искажённым: {names:?}"
        );

        sftp::remove(&h, &dir, true).await.expect("уборка");
    });
}

#[test]
#[ignore = "нужен стенд: scripts/ssh-stand/up.sh"]
fn broken_symlink_does_not_break_the_whole_listing() {
    // Битая ссылка — обычное дело на сервере. Раньше такие вещи роняли весь листинг, и
    // каталог выглядел пустым или недоступным вместо «одна запись странная».
    let s = Stand::from_env();
    let dir = scratch("симлинк");
    rt().block_on(async {
        let h = connect(&s).await;
        let _ = sftp::remove(&h, &dir, true).await;
        sftp::mkdir(&h, &dir).await.expect("каталог");
        sftp::write_file(&h, &format!("{dir}/живой.txt"), "ok", 0o644, 0, "lf")
            .await
            .expect("обычный файл");
        // Симлинк на несуществующее — через exec: в SFTP-обёртке создания ссылок нет.
        let (code, _, err) = ssh::exec(
            &h,
            &format!("ln -sfn /такого/пути/нет '{dir}/битая-ссылка'"),
            None,
        )
        .await
        .expect("создание ссылки");
        assert_eq!(code, 0, "не удалось создать битую ссылку: {err}");

        let listed = sftp::list(&h, &dir).await.expect("листинг с битой ссылкой");
        let entries = listed
            .get("entries")
            .and_then(|v| v.as_array())
            .expect("entries");
        assert_eq!(entries.len(), 2, "битая ссылка не должна скрывать соседей");

        sftp::remove(&h, &dir, true).await.expect("уборка");
    });
}

#[test]
#[ignore = "нужен стенд: scripts/ssh-stand/up.sh"]
fn permission_denied_is_explained_not_swallowed() {
    // Чужой каталог: пользователь должен увидеть «нет прав», а не пустой список, из
    // которого следует, будто каталог пуст.
    let s = Stand::from_env();
    rt().block_on(async {
        let h = connect(&s).await;
        match sftp::list(&h, "/root").await {
            Ok(v) => panic!("листинг чужого каталога не должен удаваться: {v}"),
            Err(e) => assert!(!e.is_empty(), "ошибка должна что-то объяснять"),
        }
        match sftp::write_file(&h, "/root/нельзя.txt", "x", 0o644, 0, "lf").await {
            Ok(_) => panic!("запись в чужой каталог не должна удаваться"),
            Err(e) => assert!(!e.is_empty(), "ошибка должна что-то объяснять"),
        }
    });
}

#[test]
#[ignore = "нужен стенд: scripts/ssh-stand/up.sh"]
fn parent_traversal_is_rejected_for_writes_and_normalised_for_reads() {
    // Проверка на `..` стоит на изменяющих операциях и НЕ стоит на листинге — и это
    // осознанно, а не забыли. Запретить `..` при чтении было бы театром: пользователь и
    // так волен открыть любой каталог через интерфейс, а `canonicalize` на сервере всё
    // равно приводит путь к нормальному виду. А вот удаление или переименование по пути
    // с `..` — почти всегда не то, что человек имел в виду, и туда проверка нужна.
    let s = Stand::from_env();
    rt().block_on(async {
        let h = connect(&s).await;

        let err = sftp::remove(&h, "/tmp/../etc/passwd", false)
            .await
            .expect_err("удаление по пути с «..» должно отклоняться");
        assert!(err.contains(".."), "{err}");

        let err = sftp::rename(&h, "/tmp/../etc/passwd", "/tmp/x")
            .await
            .expect_err("переименование по пути с «..» должно отклоняться");
        assert!(err.contains(".."), "{err}");

        let listed = sftp::list(&h, "/tmp/../etc").await.expect("листинг нормализует «..»");
        assert_eq!(
            listed.get("path").and_then(|v| v.as_str()),
            Some("/etc"),
            "путь должен приводиться к нормальному виду"
        );
    });
}

#[test]
#[ignore = "нужен стенд: scripts/ssh-stand/up.sh"]
fn deep_tree_is_walked_to_the_bottom() {
    // Глубокое дерево ловит рекурсию с ограничением на длину пути и на число уровней.
    let s = Stand::from_env();
    let dir = scratch("глубина");
    rt().block_on(async {
        let h = connect(&s).await;
        let _ = sftp::remove(&h, &dir, true).await;
        let deep = (1..=20).map(|i| format!("у{i}")).collect::<Vec<_>>().join("/");
        let full = format!("{dir}/{deep}");
        let (code, _, err) = ssh::exec(&h, &format!("mkdir -p '{full}' && echo дно > '{full}/дно.txt'"), None)
            .await
            .expect("создание дерева");
        assert_eq!(code, 0, "{err}");

        let listed = sftp::list(&h, &full).await.expect("листинг дна");
        let names: Vec<String> = listed
            .get("entries")
            .and_then(|v| v.as_array())
            .expect("entries")
            .iter()
            .filter_map(|e| e.get("name").and_then(|v| v.as_str()).map(str::to_string))
            .collect();
        assert_eq!(names, vec!["дно.txt"]);

        sftp::remove(&h, &dir, true).await.expect("уборка");
    });
}

#[test]
#[ignore = "нужен стенд: scripts/ssh-stand/up.sh"]
fn rename_and_chmod_take_effect() {
    let s = Stand::from_env();
    let dir = scratch("права");
    rt().block_on(async {
        let h = connect(&s).await;
        let _ = sftp::remove(&h, &dir, true).await;
        sftp::mkdir(&h, &dir).await.expect("каталог");

        let from = format!("{dir}/было.txt");
        let to = format!("{dir}/стало.txt");
        sftp::write_file(&h, &from, "текст", 0o644, 0, "lf").await.expect("файл");
        sftp::rename(&h, &from, &to).await.expect("переименование");
        sftp::chmod(&h, &to, 0o600).await.expect("права");

        let (_, out, _) = ssh::exec(&h, &format!("stat -c '%n %a' '{to}'"), None)
            .await
            .expect("проверка на сервере");
        assert!(out.contains("стало.txt"), "переименование не доехало: {out}");
        assert!(out.trim().ends_with("600"), "права не выставились: {out}");

        sftp::remove(&h, &dir, true).await.expect("уборка");
    });
}

#[test]
#[ignore = "нужен стенд: scripts/ssh-stand/up.sh"]
fn large_file_survives_upload_and_download() {
    // Файл заведомо больше одного SFTP-чанка: ловит ошибки нарезки, из-за которых
    // передача обрывается или файл приезжает укороченным.
    let s = Stand::from_env();
    let dir = scratch("большой");
    rt().block_on(async {
        let h = connect(&s).await;
        let _ = sftp::remove(&h, &dir, true).await;
        sftp::mkdir(&h, &dir).await.expect("каталог");

        let local = std::env::temp_dir().join("serein-big-upload.bin");
        let payload: Vec<u8> = (0..(512 * 1024u32)).map(|i| (i % 251) as u8).collect();
        std::fs::write(&local, &payload).expect("локальный файл");

        let remote = format!("{dir}/большой.bin");
        sftp::put_file(&h, local.to_str().unwrap(), &remote)
            .await
            .expect("загрузка");

        let back = std::env::temp_dir().join("serein-big-download.bin");
        sftp::download_file(&h, &remote, back.to_str().unwrap())
            .await
            .expect("скачивание");

        let got = std::fs::read(&back).expect("скачанный файл");
        assert_eq!(got.len(), payload.len(), "размер не совпал");
        assert_eq!(got, payload, "содержимое не совпало");

        let _ = std::fs::remove_file(&local);
        let _ = std::fs::remove_file(&back);
        sftp::remove(&h, &dir, true).await.expect("уборка");
    });
}
