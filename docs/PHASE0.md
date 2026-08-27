# Phase 0.1 — репродуцируемость

Живой протокол аудита. Не путать с `docs/RELEASE_NOTES_*.md`.
Цифры ниже — машина владельца, 2026-08-27 (Windows 11, build 26200).

## Поддерживаем vs чем собирали

| | Заявляем пользователю | Чем собран текущий tree |
| --- | --- | --- |
| ОС | Windows 10 x64 22H2+ или Windows 11 x64 | 10.0.26200 |
| WebView2 | Evergreen, отдельно не ставим | системный |
| Архитектура | x64. ARM / x86 нет | `x86_64-pc-windows-msvc` |
| Установщик | NSIS, без подписи | `targets: ["nsis"]` |
| Node (сборка) | 18.18+ | v24.16.0, npm 11.13.0 |
| Rust (сборка) | stable, `x86_64-pc-windows-msvc` | rustc / cargo **1.96.0** (2026-05-25) |
| Tauri CLI | 2.x | `@tauri-apps/cli` **2.11.2** (lock) |
| Tauri API | 2.x | `@tauri-apps/api` **2.11.0** (lock) |
| crate `tauri` | 2 | `Cargo.toml`: `tauri = "2"` |

Пользователю Node/Rust **не нужны**. Это только для `tauri dev` / `tauri build`.

Identifier установщика: `dev.serein.app`. Публичный `v0.1.0` ещё ставился как TermiNAL (`TermiNAL_0.1.0_x64-setup.exe`). Windows считает это **другим приложением**. In-place upgrade 0.1.0 → Serein не будет.

Данные: `%APPDATA%\serein\`. Если есть только `%APPDATA%\term-tauri\`, при первом запуске папка один раз переименовывается (`store.rs`).

## Автоматический smoke

Из корня репо (cargo в PATH, иначе `$env:USERPROFILE\.cargo\bin`):

```powershell
npm run smoke
```

Это `tsc --noEmit` и `cargo check` по `src-tauri`. SSH и UI не трогает.

То же на GitHub: `.github/workflows/ci.yml` (`windows-latest`) — плюс `cargo test --lib` (крипта). `tauri build` и живой SSH в Actions нет.

## Ручной smoke (установленный exe, без toolchain)

1. Запуск из меню Пуск → окно **Serein**, не падает за 10 секунд.
2. Локальный терминал: prompt есть, `echo ok` печатает `ok`.
3. После первого запуска есть `%APPDATA%\serein\`.
4. Сайдбар открывается, «добавить сервер» не пустой экран.
5. Закрыть крестиком, открыть снова — без диалога «уже запущено» и без второго процесса-зомби.

Живой SSH в этот минимум не входит (это уже 0.3).

## Чистая установка / без dev / upgrade / uninstall

Прогон 2026-08-27 на этой машине.

`npm run tauri -- build` собрал **release `serein.exe`** (`src-tauri/target/release/serein.exe`, ~16.6 МБ). Штатный NSIS-бандл Tauri **не** собрался: скачивание `nsis-3.11.zip` с GitHub оборвалось (`os error 10053` / connection reset). Поставили NSIS 3.12 через winget (SourceForge) и упаковали тот же exe скриптом `scripts/phase0-install.nsi` → `scripts/phase0-Serein_0.1.0_x64-setup.exe` (~4.2 МБ сжатый). Это per-user установка в `%LOCALAPPDATA%\Serein`, не шаблон Tauri (нет updater-артефакта). Клики в UI не гонялись.

Профиль перед опытами скопирован в `%TEMP%\serein-phase0-term-tauri-backup`.

### A. Запуск без dev environment — ok

Release и установленный `Serein.exe` стартовали с PATH без node/cargo/rustup. Процесс живой, к `:1420` не цеплялся. После первого запуска `%APPDATA%\serein\` на месте.

### B. Чистая установка — ok (наш NSIS)

Тихая установка `/S`. Есть `%LOCALAPPDATA%\Serein\Serein.exe`, ярлык в меню Пуск, ключ uninstall в HKCU.

### C. Upgrade с предыдущей версии — не in-place, миграция данных ok

Стоял TermiNAL 0.1.0 (`%LOCALAPPDATA%\TermiNAL\term-tauri.exe`). Identifier другой, новый установщик его не заменяет. Пока TermiNAL ещё был в реестре, запустили новый `serein.exe`: папка `%APPDATA%\term-tauri` один раз переименовалась в `serein`. Потом TermiNAL снесён (`uninstall.exe /S`), каталог `Local\TermiNAL` исчез.

Настоящий in-place (тот же `dev.serein.app`) — когда будет следующий Serein с тем же identifier.

### D. Uninstall / reinstall — ok

После `/S` uninstall: нет exe, нет каталога `Local\Serein`, нет ярлыка, нет ключа в реестре. `%APPDATA%\serein` **остался** (servers/settings/secrets). Повторная установка: exe снова на месте, те же json в AppData. Сейчас Serein снова установлен, процесс запущен.

## Журнал прогонов

| Дата | Что | Результат |
| --- | --- | --- |
| 2026-08-27 | `npm run smoke` | ok. Починен `vault.rs` (`crate::store`) |
| 2026-08-27 | `tauri build` | rust release ok; NSIS Tauri fail (GitHub nsis-3.11.zip) |
| 2026-08-27 | A: exe без node/cargo в PATH | ok, профиль `term-tauri` → `serein` |
| 2026-08-27 | C: TermiNAL 0.1.0 + новый exe | разные приложения; данные мигрировали; TermiNAL снесён |
| 2026-08-27 | B+D: phase0 NSIS install/uninstall/reinstall | exe/ярлык снимаются, AppData живёт; Serein снова стоит |
| 2026-08-27 | Phase 0.2 bench `home_server` LAN | см. раздел ниже; сырьё `docs/phase0-bench.jsonl` + `phase0-bench-files.jsonl` |

## Phase 0.2 — performance (2026-08-27)

Хост: `home_server` (`192.168.0.156`), ключ, LAN. Гарнесс: `cargo run --release --bin phase0_bench` — те же `ssh`/`sftp` модули, **без** WebView и xterm. PTY-shell не открывали: exec-канал. Кадры UI не снимали.

Serein.exe в это время уже был открыт: **~92 МБ** Working Set. Холодный старт в этом прогоне не мерили. Процесс бенча: ~15–19 МБ.

По дороге: в `russh-sftp` дефолт **10 с на запрос**. В `sftp::open` теперь **300 с** — иначе 100+ МБ на медленной стороне отваливаются.

| Сценарий | Результат |
| --- | --- |
| SSH connect + `true` | 79–93 мс connect, ping 53–66 мс (первый); дальше avg 2–11 мс |
| 5 / 10 / 20 сессий | все ок; connect 0.4 / 0.8 / 1.5 с; ping avg 2–11 мс |
| `timeout 8 yes` | ~890–937 МБ за 8 с, **~111 МБ/с** |
| `docker logs -f` 8 с | контейнер `bc22683d`, ~24 КБ (мало логов) |
| `journalctl -f` 8 с | 1–5 КБ |
| SFTP up 10 / 100 / 1024 МБ | **80–111 МБ/с** (0.12 с / 0.9 с / 9.2 с) |
| SFTP down 10 / 100 МБ | **~16.7–17.1 МБ/с** (0.6 с / 5.9 с) |
| SFTP down 1 ГБ | **завис** после reconnect SSH: CPU почти не рос, RSS ~15 МБ; процесс убит. Два прогона. |
| 1000 файлов по 1 байту | 831 мс, ~1200 файлов/с |
| 10 000 файлов по 1 байту | 7.7 с, ~1300 файлов/с |

Асимметрия up vs down на LAN — базовая цифра, не «гигабит симметричный». 1 ГБ download в этом стеке был дырой (завис). В дереве после 2026-08-27: pipeline READ, чанк 32 КиБ, окно SSH 16 МиБ. Повторный прогон 1 ГБ down — когда скажете.

Повтор: `phase0_bench.exe home_server` и `... --files`.

