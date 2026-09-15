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
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

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

/// Имена в каталоге, включая скрытые: временные файлы передачи начинаются с точки.
async fn names_in(h: &ssh::SharedHandle, dir: &str) -> Vec<String> {
    let (code, out, err) = ssh::exec(h, &format!("ls -A '{dir}'"), None).await.expect("листинг");
    assert_eq!(code, 0, "листинг: {err}");
    out.lines().map(str::to_string).collect()
}

async fn remote_text(h: &ssh::SharedHandle, path: &str) -> String {
    let (code, out, err) = ssh::exec(h, &format!("cat '{path}'"), None).await.expect("чтение");
    assert_eq!(code, 0, "чтение: {err}");
    out
}

/// Своя папка на прогон, пустая.
fn local_scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("serein-стенд-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("своя папка");
    dir
}

fn local_names(dir: &std::path::Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .expect("своя папка")
        .filter_map(|e| e.ok().map(|e| e.file_name().to_string_lossy().to_string()))
        .collect();
    names.sort();
    names
}

/// Опустить флажок передачи чуть позже, посреди неё.
fn stop_soon(alive: &Arc<AtomicBool>) {
    let flip = alive.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(40)).await;
        flip.store(false, Ordering::Relaxed);
    });
}

#[test]
#[ignore = "нужен стенд: scripts/ssh-stand/up.sh"]
fn прерванная_заливка_оставляет_прежний_файл_целым() {
    // Раньше заливка писала прямо в целевой файл: отменённая или оборванная замена конфига
    // оставляла на его месте обрубок. Теперь файл пишется рядом и встаёт на место в конце.
    let s = Stand::from_env();
    let dir = scratch("прерванная заливка");
    rt().block_on(async {
        let h = connect(&s).await;
        let _ = sftp::remove(&h, &dir, true).await;
        sftp::mkdir(&h, &dir).await.expect("каталог");
        let file = format!("{dir}/данные.bin");
        sftp::write_file(&h, &file, "оригинал", 0o640, 0, "lf").await.expect("оригинал");

        let local_dir = local_scratch("прерванная-заливка");
        let local = local_dir.join("данные.bin");
        let payload: Vec<u8> = (0..(48 * 1024 * 1024u32)).map(|i| (i % 253) as u8).collect();
        std::fs::write(&local, &payload).expect("свой файл");
        let local = local.to_string_lossy().to_string();

        // Отмена до первого байта: путь уборки проходится всегда, как бы быстро ни шла сеть.
        let stopped = AtomicBool::new(false);
        let r = sftp::put_file_while(&h, &local, &file, Some(&stopped)).await;
        assert!(r.is_err(), "отменённая заливка не может закончиться успехом");
        assert_eq!(remote_text(&h, &file).await, "оригинал");
        assert_eq!(names_in(&h, &dir).await, vec!["данные.bin"], "временный файл обязан убираться");

        // Отмена посреди передачи. Успеть может любая сторона, поэтому проверяем то, что
        // верно в обоих случаях: файл либо прежний, либо новый целиком - и ничего лишнего.
        let alive = Arc::new(AtomicBool::new(true));
        stop_soon(&alive);
        let r = sftp::put_file_while(&h, &local, &file, Some(&alive)).await;
        match r {
            Ok(()) => {
                let (_, size, _) = ssh::exec(&h, &format!("stat -c %s '{file}'"), None).await.expect("размер");
                assert_eq!(size.trim(), payload.len().to_string(), "успешная заливка - файл целиком");
            }
            Err(_) => assert_eq!(remote_text(&h, &file).await, "оригинал", "после отмены - прежний файл"),
        }
        assert_eq!(names_in(&h, &dir).await, vec!["данные.bin"]);

        let _ = std::fs::remove_dir_all(&local_dir);
        sftp::remove(&h, &dir, true).await.expect("уборка");
    });
}

#[test]
#[ignore = "нужен стенд: scripts/ssh-stand/up.sh"]
fn заливка_поверх_файла_сохраняет_его_права() {
    // Замена через временный файл не должна открывать закрытый файл: новый получает права
    // прежнего, а не обычные права только что созданного.
    let s = Stand::from_env();
    let dir = scratch("права заливки");
    rt().block_on(async {
        let h = connect(&s).await;
        let _ = sftp::remove(&h, &dir, true).await;
        sftp::mkdir(&h, &dir).await.expect("каталог");
        let file = format!("{dir}/секрет.conf");
        sftp::write_file(&h, &file, "старое", 0o600, 0, "lf").await.expect("прежний файл");

        let local_dir = local_scratch("права-заливки");
        let local = local_dir.join("секрет.conf");
        std::fs::write(&local, "новое").expect("свой файл");
        sftp::put_file(&h, &local.to_string_lossy(), &file).await.expect("заливка");

        let read = sftp::read_file(&h, &file).await.expect("чтение");
        assert_eq!(read.get("content").and_then(|v| v.as_str()), Some("новое"));
        assert_eq!(
            read.get("mode").and_then(|v| v.as_u64()).map(|m| m & 0o777),
            Some(0o600),
            "права обязаны остаться прежними"
        );
        assert_eq!(names_in(&h, &dir).await, vec!["секрет.conf"]);

        let _ = std::fs::remove_dir_all(&local_dir);
        sftp::remove(&h, &dir, true).await.expect("уборка");
    });
}

#[test]
#[ignore = "нужен стенд: scripts/ssh-stand/up.sh"]
fn заливка_в_ссылку_меняет_цель_и_не_ломает_ссылку() {
    // `sites-enabled/site` -> `sites-available/site`: замена переименованием превратила бы
    // ссылку в обычный файл, и цель молча разошлась бы с ней.
    let s = Stand::from_env();
    let dir = scratch("заливка в ссылку");
    rt().block_on(async {
        let h = connect(&s).await;
        let _ = sftp::remove(&h, &dir, true).await;
        sftp::mkdir(&h, &dir).await.expect("каталог");
        sftp::write_file(&h, &format!("{dir}/цель.conf"), "было", 0o644, 0, "lf").await.expect("цель");
        let (code, _, err) = ssh::exec(&h, &format!("cd '{dir}' && ln -s цель.conf ссылка.conf"), None)
            .await
            .expect("ссылка");
        assert_eq!(code, 0, "ссылка не создалась: {err}");

        let local_dir = local_scratch("заливка-в-ссылку");
        let local = local_dir.join("ссылка.conf");
        std::fs::write(&local, "стало").expect("свой файл");
        sftp::put_file(&h, &local.to_string_lossy(), &format!("{dir}/ссылка.conf")).await.expect("заливка");

        let (code, out, _) = ssh::exec(&h, &format!("cd '{dir}' && test -L ссылка.conf && cat цель.conf"), None)
            .await
            .expect("проверка");
        assert_eq!(code, 0, "ссылка обязана остаться ссылкой");
        assert_eq!(out, "стало", "новое содержимое - в цели");

        let _ = std::fs::remove_dir_all(&local_dir);
        sftp::remove(&h, &dir, true).await.expect("уборка");
    });
}

#[test]
#[ignore = "нужен стенд: scripts/ssh-stand/up.sh"]
fn совпадения_имён_ищутся_по_имени_файла() {
    // Панель спрашивает про замену до заливки. Имена приходят своими путями - в том числе
    // виндовыми, - а сравнивать надо только последнюю часть, и папка тоже считается.
    let s = Stand::from_env();
    let dir = scratch("совпадения имён");
    rt().block_on(async {
        let h = connect(&s).await;
        let _ = sftp::remove(&h, &dir, true).await;
        sftp::mkdir(&h, &dir).await.expect("каталог");
        sftp::write_file(&h, &format!("{dir}/отчёт.txt"), "есть", 0o644, 0, "lf").await.expect("файл");
        sftp::mkdir(&h, &format!("{dir}/папка")).await.expect("папка");

        let names: Vec<String> = [r"C:\Users\me\отчёт.txt", "/home/me/папка", "новый.txt", "..", ""]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let found = sftp::name_conflicts(&h, &dir, &names).await.expect("проверка имён");
        assert_eq!(found, vec!["отчёт.txt", "папка"]);

        sftp::remove(&h, &dir, true).await.expect("уборка");
    });
}

#[test]
#[ignore = "нужен стенд: scripts/ssh-stand/up.sh"]
fn прерванное_скачивание_не_портит_свой_файл() {
    // Скачивание идёт во временный файл рядом; отмена обязана оставить прежний свой файл
    // и не оставить обрывок.
    let s = Stand::from_env();
    let dir = scratch("прерванное скачивание");
    rt().block_on(async {
        let h = connect(&s).await;
        let _ = sftp::remove(&h, &dir, true).await;
        sftp::mkdir(&h, &dir).await.expect("каталог");
        let file = format!("{dir}/большой.bin");
        let (code, _, err) = ssh::exec(&h, &format!("head -c 33554432 /dev/zero > '{file}'"), None)
            .await
            .expect("большой файл");
        assert_eq!(code, 0, "файл не создался: {err}");

        let local_dir = local_scratch("прерванное-скачивание");
        let local = local_dir.join("большой.bin");
        std::fs::write(&local, "мой").expect("свой файл");
        let local_s = local.to_string_lossy().to_string();

        let stopped = AtomicBool::new(false);
        let r = sftp::download_file_while(&h, &file, &local_s, Some(&stopped)).await;
        assert!(r.is_err(), "отменённое скачивание не может закончиться успехом");
        assert_eq!(std::fs::read_to_string(&local).expect("свой файл"), "мой");
        assert_eq!(local_names(&local_dir), vec!["большой.bin"]);

        let alive = Arc::new(AtomicBool::new(true));
        stop_soon(&alive);
        match sftp::download_file_while(&h, &file, &local_s, Some(&alive)).await {
            Ok(()) => assert_eq!(std::fs::metadata(&local).expect("файл").len(), 33_554_432),
            Err(_) => assert_eq!(std::fs::read_to_string(&local).expect("свой файл"), "мой"),
        }
        assert_eq!(local_names(&local_dir), vec!["большой.bin"], "обрывок не оставляем");

        let _ = std::fs::remove_dir_all(&local_dir);
        sftp::remove(&h, &dir, true).await.expect("уборка");
    });
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

#[test]
#[ignore = "нужен стенд: scripts/ssh-stand/up.sh"]
fn имена_с_сервера_не_выводят_запись_за_папку_скачивания() {
    // На Linux `..\..\снаружи.txt` - законное имя файла: обратная косая там обычный
    // символ. Windows читает её как разделитель каталогов, и такое имя, склеенное с папкой
    // скачивания, указывает уже за её пределы. Проверяем на настоящем сервере: имя
    // создаётся, попадает в листинг, но в задания на скачивание не проходит.
    let s = Stand::from_env();
    let dir = scratch("ловушка имён");
    rt().block_on(async {
        let h = connect(&s).await;
        let _ = sftp::remove(&h, &dir, true).await;
        sftp::mkdir(&h, &dir).await.expect("каталог");

        let trap = format!("{dir}/..\\..\\снаружи.txt");
        sftp::write_file(&h, &trap, "не должно попасть наружу", 0o644, 0, "lf")
            .await
            .expect("сервер обязан позволить такое имя - в этом и дело");
        let ok = format!("{dir}/отчёт.txt");
        sftp::write_file(&h, &ok, "обычный файл", 0o644, 0, "lf")
            .await
            .expect("обычный файл");

        let local_dir = std::env::temp_dir().join("serein-имена-стенд");
        let plan = sftp::plan_download(&h, &dir, &local_dir.to_string_lossy())
            .await
            .expect("план скачивания");

        let names: Vec<&str> = plan.jobs.iter().map(|(lp, ..)| lp.as_str()).collect();
        assert!(
            names.iter().any(|lp| lp.ends_with("отчёт.txt")),
            "безопасные имена должны скачиваться: {names:?}"
        );
        // Отклонять это имя обязана сборка для Windows: там оно и есть путь. На юниксах
        // оно законно, и файл скачивается как есть - с обратными косыми внутри имени.
        // Проверка ниже важна для обеих систем: наружу не выходит ни одно задание.
        if cfg!(windows) {
            assert!(
                !plan.refused.is_empty(),
                "имя-ловушка обязано быть отклонено, а не скачано молча"
            );
        }
        for (lp, ..) in &plan.jobs {
            assert!(
                serein_lib::localname::under_root(
                    std::path::Path::new(&local_dir),
                    std::path::Path::new(lp)
                ),
                "задание «{lp}» выходит за папку скачивания"
            );
        }

        sftp::remove(&h, &dir, true).await.expect("уборка");
    });
}

#[test]
#[ignore = "нужен стенд: scripts/ssh-stand/up.sh"]
fn сохранение_сохраняет_права_и_не_теряет_оригинал() {
    // Раньше сохранение писало во временный файл с постоянным именем, при любой неудаче
    // переименования удаляло оригинал и пробовало снова - а права терялись всегда:
    // на месте закрытого файла оказывался новый с обычными правами.
    let s = Stand::from_env();
    let dir = scratch("права сохранения");
    rt().block_on(async {
        let h = connect(&s).await;
        let _ = sftp::remove(&h, &dir, true).await;
        sftp::mkdir(&h, &dir).await.expect("каталог");

        let file = format!("{dir}/секрет.conf");
        sftp::write_file(&h, &file, "было", 0o600, 0, "lf").await.expect("первая запись");
        let before = sftp::list(&h, &dir).await.expect("листинг");
        assert!(
            format!("{before:?}").contains("секрет.conf"),
            "файл должен появиться: {before:?}"
        );

        // Второй раз - как это делает панель после правки: права не передаются, их
        // полагается сохранить от прежнего файла.
        let saved = sftp::write_file(&h, &file, "стало", 0, 0, "lf").await.expect("вторая запись");
        assert_eq!(saved.get("ok").and_then(|v| v.as_bool()), Some(true));

        let read = sftp::read_file(&h, &file).await.expect("чтение");
        assert_eq!(read.get("content").and_then(|v| v.as_str()), Some("стало"));
        assert_eq!(
            read.get("mode").and_then(|v| v.as_u64()).map(|m| m & 0o777),
            Some(0o600),
            "права обязаны остаться прежними"
        );

        // Ни одного временного или отложенного файла после успешного сохранения.
        let listed = sftp::list(&h, &dir).await.expect("листинг");
        let names: Vec<String> = listed["entries"]
            .as_array()
            .expect("entries")
            .iter()
            .filter_map(|e| e["name"].as_str().map(str::to_string))
            .collect();
        assert_eq!(names.len(), 1, "после сохранения остаётся один файл: {names:?}");

        sftp::remove(&h, &dir, true).await.expect("уборка");
    });
}

#[test]
#[ignore = "нужен стенд: scripts/ssh-stand/up.sh"]
fn время_правки_файла_читается_с_сервера() {
    // На этом времени держится защита от затирания чужой правки во внешнем редакторе:
    // не узнали время - не имеем права считать, что файл никто не менял.
    let s = Stand::from_env();
    let dir = scratch("время правки");
    rt().block_on(async {
        let h = connect(&s).await;
        let _ = sftp::remove(&h, &dir, true).await;
        sftp::mkdir(&h, &dir).await.expect("каталог");
        let file = format!("{dir}/файл.txt");
        sftp::write_file(&h, &file, "раз", 0o644, 0, "lf").await.expect("запись");

        let было = sftp::remote_mtime(&h, &file).await.expect("запрос времени");
        let было = было.expect("сервер обязан сообщить время правки файла");
        // Секунда - шаг времени в SFTP: чтобы вторая правка отличалась, надо переждать его.
        tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
        sftp::write_file(&h, &file, "два", 0o644, 0, "lf").await.expect("вторая запись");
        let стало = sftp::remote_mtime(&h, &file)
            .await
            .expect("запрос времени")
            .expect("время после второй правки");
        assert!(стало > было, "время правки обязано вырасти: было {было}, стало {стало}");

        sftp::remove(&h, &dir, true).await.expect("уборка");
    });
}

#[test]
#[ignore = "нужен стенд: scripts/ssh-stand/up.sh"]
fn ссылки_не_закручивают_обход_дерева() {
    // Петля из ссылок - не выдумка: `/proc` и `/sys` полны ссылок на самих себя, и обход
    // по ним не заканчивается никогда. При этом ссылка на файл (`latest.log`) - обычное
    // дело, и терять её было бы неожиданно. Проверяем оба случая сразу.
    let s = Stand::from_env();
    let dir = scratch("ссылки");
    rt().block_on(async {
        let h = connect(&s).await;
        let _ = sftp::remove(&h, &dir, true).await;
        sftp::mkdir(&h, &dir).await.expect("каталог");
        sftp::write_file(&h, &format!("{dir}/файл.txt"), "содержимое", 0o644, 0, "lf")
            .await
            .expect("файл");
        // Ссылка на сам каталог - петля; ссылка на файл - обычная полезная ссылка.
        let (code, _, err) = ssh::exec(
            &h,
            &format!("ln -s {dir} {dir}/петля && ln -s {dir}/файл.txt {dir}/на-файл"),
            None,
        )
        .await
        .expect("создание ссылок");
        assert_eq!(code, 0, "ссылки не создались: {err}");

        let местная = std::env::temp_dir().join("serein-ссылки-стенд");
        let план = sftp::plan_download(&h, &dir, &местная.to_string_lossy())
            .await
            .expect("план скачивания не должен зависнуть");

        let пути: Vec<&str> = план.jobs.iter().map(|(lp, ..)| lp.as_str()).collect();
        assert!(пути.iter().any(|p| p.ends_with("файл.txt")), "обычный файл: {пути:?}");
        assert!(пути.iter().any(|p| p.ends_with("на-файл")), "ссылка на файл забирается: {пути:?}");
        assert!(
            !пути.iter().any(|p| p.contains("петля")),
            "в ссылку на каталог заходить нельзя: {пути:?}"
        );
        assert!(
            план.refused.iter().any(|(rel, _)| rel.ends_with("петля")),
            "про пропущенную ссылку надо сказать: {:?}",
            план.refused
        );

        sftp::remove(&h, &dir, true).await.expect("уборка");
    });
}
