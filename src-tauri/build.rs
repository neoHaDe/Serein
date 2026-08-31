fn main() {
    // Версия живёт в двух местах и обязана совпадать.
    //
    // `tauri.conf.json` теперь ссылается на `package.json`, так что осталась пара
    // package.json ↔ Cargo.toml. Cargo сослаться на чужой файл не умеет, поэтому сверяем
    // здесь: расхождение — это релиз, в котором установщик, «о программе» и обновление
    // называют разные версии, и ловится оно обычно уже после публикации.
    check_version_matches_package_json();

    // Манифест ComCtl32 v6 встраиваем сами, для ВСЕХ линкуемых артефактов, а не только
    // для приложения.
    //
    // Зачем: wry импортирует SetWindowSubclass, RemoveWindowSubclass, DefSubclassProc и
    // TaskDialogIndirect — их экспортирует только шестая версия comctl32. Без манифеста
    // загрузчик подставляет 5.82 из System32, этих функций там нет, и процесс умирает
    // ещё до main с STATUS_ENTRYPOINT_NOT_FOUND (0xc0000139).
    //
    // Приложению манифест клал сам Tauri, а тестовому бинарю — некому. Из-за этого любой
    // тест, упоминающий `tauri::AppHandle`, ронял ВЕСЬ тестовый бинарь крейта — а
    // `AppHandle` лежит внутри `ClientHandler`, то есть внутри SSH-слоя. Отсюда и нулевое
    // покрытие `ssh.rs`, `tunnels.rs`, `sftp.rs`, `vault.rs`, `backup.rs`: они не
    // «остались без тестов», их было невозможно собрать.
    //
    // Почему именно так, а не флагом только для тестов: `rustc-link-arg-tests` этот cargo
    // не принимает (в отличие от `-bins`), а общий флаг поверх манифеста Tauri даёт
    // конфликт — LNK1123 при сборке приложения. Поэтому источник манифеста делаем один:
    // Tauri свой не кладёт, наш получают и приложение, и тесты. Содержимое совпадает с
    // тем, что Tauri встраивал по умолчанию, — см. `serein.manifest`.
    #[cfg(windows)]
    {
        let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("serein.manifest");
        println!("cargo:rerun-if-changed=serein.manifest");
        println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
        println!("cargo:rustc-link-arg=/MANIFESTINPUT:{}", manifest.display());

        let attrs = tauri_build::Attributes::new()
            .windows_attributes(tauri_build::WindowsAttributes::new_without_app_manifest());
        tauri_build::try_build(attrs).expect("не удалось собрать tauri-build");
        return;
    }
    #[cfg(not(windows))]
    tauri_build::build()
}

/// Сверяет версию в `Cargo.toml` с версией в `package.json`.
fn check_version_matches_package_json() {
    let pkg = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../package.json");
    println!("cargo:rerun-if-changed=../package.json");
    let Ok(text) = std::fs::read_to_string(&pkg) else {
        // Собирают только Rust-часть, без фронтенда — не повод ломать сборку.
        return;
    };
    // Мелкий разбор вместо зависимости на serde_json в build-скрипте: нужна одна строка.
    let found = text
        .split("\"version\"")
        .nth(1)
        .and_then(|rest| rest.split('"').nth(1))
        .unwrap_or_default()
        .to_string();
    let ours = env!("CARGO_PKG_VERSION");
    if !found.is_empty() && found != ours {
        panic!(
            "версии разошлись: Cargo.toml = {ours}, package.json = {found}. Меняйте обе разом: node scripts/set-version.mjs <версия>"
        );
    }
}
