fn main() {
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
