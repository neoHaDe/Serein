<div align="center">

<img src="src-tauri/icons/128x128.png" width="96" alt="Иконка Serein" />

# Serein

**Десктоп-клиент для серверов и сетевого железа — всё в одном окне.**

SSH, SFTP со встроенным редактором, консоль по COM-порту, telnet и сырой TCP.
Вкладки и сплит-панели, проброс портов, мониторинг ресурсов, панель Docker
и локальный терминал — в установщике на **≈ 6,7 МБ**.

Бесплатно, открытый код, Apache 2.0. Windows x64 и Linux x64, **v1.2.5**.

[![Tauri](https://img.shields.io/badge/Tauri-2-24C8DB?logo=tauri&logoColor=white)](https://tauri.app)
[![Rust](https://img.shields.io/badge/Rust-stable-000000?logo=rust&logoColor=white)](https://www.rust-lang.org)
[![React](https://img.shields.io/badge/React-18-61DAFB?logo=react&logoColor=black)](https://react.dev)
[![TypeScript](https://img.shields.io/badge/TypeScript-5-3178C6?logo=typescript&logoColor=white)](https://www.typescriptlang.org)
[![CI](https://github.com/neoHaDe/Serein/actions/workflows/ci.yml/badge.svg)](https://github.com/neoHaDe/Serein/actions/workflows/ci.yml)
[![Лицензия: Apache 2.0](https://img.shields.io/badge/License-Apache_2.0-blue.svg)](#-лицензия)

Windows · без своего Chromium (системный WebView2) ·
[English version](README.md)

### → [Скачать последний выпуск](../../releases/latest)

<img src="docs/screenshot.png" width="820" alt="Скриншот Serein" />

</div>

---

## ✨ Почему Serein

Интерфейс в системном WebView, SSH / SFTP / шифрование / PTY — в одном Rust-бинарнике.
Не тащим Chromium. Не пытаемся обогнать Tabby списком галочек: цель — **рабочее место
по серверу** (терминал, файлы, Docker, логи, ресурсы, туннели), а не «ещё один SSH-клиент».

| | **Serein (Tauri)** | Типичный Electron-клиент |
| --- | :---: | :---: |
| Размер установщика | **≈ 6,7 МБ** | ≈ 85 МБ |
| Память в простое | **≈ 33 МБ** | 150–250 МБ |
| SSH-движок | чистый Rust [`russh`](https://github.com/Eugeny/russh) | libssh2 / нативный |
| Рантайм | системный WebView2 | полный Chromium |

Цифры — живой `tauri dev` (RAM) и NSIS 1.2.5 (~6,7 МБ сжатый). На слабом ноутбуке те же 33 МБ не обещаем.

---

## 🚀 Возможности

### Server Workspace (v1.1.0)
- У SSH-вкладки — **рельса инструментов сервера**: Terminal, Docker, Logs, Processes, Services, Tunnels
- **Processes** — таблица `ps` + метрики CPU/RAM/диск сверху; **Docker** — компактный список, свойства по ПКМ
- **SFTP** — боковая панель из TabBar; список серверов при подключении сворачивается
- **↗ Открепить** вкладку или панель workspace в отдельное окно; **← вернуть** в главное окно (SSH-сессия не рвётся)

### Терминал и UX
- **Несколько SSH-вкладок** + **сплит-панели** (дерево с перетаскиванием границ, выбор сервера на панель)
- **Локальный терминал** с умным выбором shell (PowerShell → cmd, либо свой / WSL)
- Поиск (`Ctrl+F`), зум, **17 тем на весь интерфейс**, **компактный режим**
- **Своя рамка окна** (без хрома Windows): свернуть / развернуть / закрыть
- **Broadcast-ввод** (в пределах текущей вкладки), восстановление вкладок, перетаскивание вкладок
- **Ctrl+Shift+C / Ctrl+Shift+V** — копирование и вставка в терминале (не DevTools WebView2)
- Батчинг вывода SSH; при перегрузке — буфер 4 МиБ с пометкой

### Подключения
- Сайдбар с **группами и перетаскиванием**, меню по правой кнопке, поиск, **живой статус подключения**
- Аутентификация: **пароль · ключ · keyboard-interactive 2FA · SSH-агент** (выбор ключа из связки)
- **Консоль по COM-порту** — скорость, чётность, управление потоком, DTR/RTS, посылка BREAK
- **Telnet** — согласование опций, тип терминала, размер окна, BREAK/Interrupt/AYT, выбор перевода строки
- **TCP без обработки** — байты как есть, для консольных серверов
- **ProxyJump / бастион** — цепочки (рекурсивно, через `direct-tcpip`) и **ProxyCommand** (`%h %p %r`)
- **Сжатие трафика** (`zlib@openssh.com`) и режим **совместимости со старым железом**
- **TOFU-проверка known-hosts** на каждом хопе: диалог сверки, управление списком, импорт из OpenSSH
- **Переподключение при обрыве** — вручную или авто (до 5 попыток, бэкофф)
- Импорт из **`~/.ssh/config`** и сессий **PuTTY**

- Навигация по панелям с клавиатуры, настраиваемые хоткеи, **командная палитра** (`Ctrl+Shift+P`)

### Файлы (SFTP)
- Просмотр с **кликабельными хлебными крошками**, **инлайн-переименование**, drag & drop внутрь окна
- **Параллельные передачи** (пул 4, до 8 в настройках), **менеджер передач**: скорость, ETA, пауза, повтор
- **Колонки как в Explorer** (имя, тип, права, размер, дата), сортировка, ресайз, Ctrl/Shift-выделение
- **Drag-and-drop с Проводником** — с сервера на рабочий стол / в Загрузки; с ПК — upload в открытую папку
- chmod, скрытые файлы, symlink follow, превью картинок; действия в меню по ПКМ
- Двухпанельный режим (локально ↔ сервер); **SFTP можно открепить** в отдельное окно
- **Ctrl+колёсико** (и Ctrl+/−/0) меняет масштаб текста в SFTP и логах
- **Встроенный редактор** (CodeMirror 6) — атомарное сохранение прямо на сервер
- **Внешний редактор** — открывает файл в редакторе ОС и сам заливает при сохранении

### Туннели и эксплуатация
- Проброс портов: **локальный `-L`**, **обратный `-R`**, **динамический SOCKS5 `-D`** (открытие туннеля можно прервать)
- **Мониторинг ресурсов** — CPU / RAM / диск / load (опрос `/proc` + `df`)
- **Docker-workspace (v1.2.0)** — контейнеры с живыми `docker stats`, портами и health; файлы внутри контейнера; старт/стоп/рестарт/удаление, shell
- **Docker Compose (v1.2.0)** — проекты и сервисы, up/down/start/stop/restart, логи и shell по сервису, чтение compose YAML
- **Логи Docker** — цвет уровней, follow (`-f`) со стопом, широкая панель; **открепить** на второй монитор
- **Логи хоста (v1.2.0)** — `journalctl` с подсветкой, фильтром и выгрузкой отчёта об ошибках в `.txt`
- **Логирование сессии (v1.2.0)** — запись вывода терминала в `%APPDATA%\serein\logs`, без ANSI

### Окна
- Откреплённые вкладки, SFTP, логи и панели workspace — **отдельные окна ОС**, без рамки Windows
- **Магнит**: окна стыкуются вплотную и по направляющим (края / центр)
- Группу тащит главное окно; доп. окно отцепляется, если потянуть за него
- Aux сворачиваются **независимо** от main; свёрнутые aux вернуть — клик по Serein на панели задач или кнопка в статус-баре
- Клик по любому окну поднимает **все** окна Serein; по умолчанию **одна кнопка** на панели задач
  (в настройках можно вернуть отдельные кнопки)
- Запоминание расположения откреплённых окон после перезапуска

### Безопасность и хранение
- Секреты шифруются через **DPAPI** + опциональный **мастер-пароль**
  (scrypt → AES-256-GCM); в UI пароли и ключи не отдаются
- Зашифрованный **бэкап `.tbk`** серверов, настроек и сниппетов
- **Генерация SSH-ключей** (ed25519 / RSA) + `ssh-copy-id`

---

## Быстрый старт

1. Поставь установщик или скачай portable `Serein_1.2.5_x64-portable.exe` из [Releases](../../releases/latest).
2. Импортируй `~/.ssh/config` или добавь сервер вручную.
3. Подключись. Локальный терминал работает и без SSH.

Цель: установка → первое соединение меньше чем за две минуты.

---

## Что нужно

| Требование | Ответ |
| --- | --- |
| Система | Windows 10 x64 **22H2+** или Windows 11 x64; Linux x64 (`.deb` / AppImage). macOS пока нет |
| Веб-рантайм | WebView2 на Windows (уже есть в системе); `webkit2gtk-4.1` на Linux (ставится вместе с `.deb`) |
| Права | администратор для повседневной работы не нужен |
| Сборка из исходников | Node **18.18+** (проверено на 24.16), Rust **stable** (`x86_64-pc-windows-msvc` / `x86_64-unknown-linux-gnu`), Tauri CLI **2.11.x**. Для Linux нужен ещё `libudev-dev` — см. [LINUX_MIGRATION.md](docs/LINUX_MIGRATION.md) |
| SSH-агент | пароль, файл ключа, **SSH-агент** или keyboard-interactive |

Матрица и smoke: [`docs/PHASE0.md`](docs/PHASE0.md).

---

## 📦 Установка

С [Releases](../../releases/latest):

- **`Serein_1.2.5_x64-setup.exe`** — установщик Windows (меню Пуск, удаление).
- **`Serein_1.2.5_x64-portable.exe`** — один файл Windows, без установки. Положи и запусти. Настройки всё равно в `%APPDATA%\serein`.
- **`Serein_1.2.5_amd64.deb`** — пакет Debian/Ubuntu/Astra (`/usr/bin/serein`).
- **`Serein_1.2.5_amd64.AppImage`** — портативный Linux-бинарь.

Сборка **не подписана** — SmartScreen ругнётся. *Подробнее → Выполнить в любом случае*.
Что нового — [release notes](docs/RELEASE_NOTES_v1.2.5.md) и описание выпуска на GitHub.

Автообновление в конфиге заведено (`nehade.xyz/updates/terminal/`), на неподписанном
установщике на него не рассчитывай.

---

## 🛠 Стек

| Слой | Технологии |
| --- | --- |
| Оболочка | **[Tauri 2](https://tauri.app)** (Rust, системный WebView2) |
| Фронтенд | **React 18** · **TypeScript 5** · **Vite** |
| Терминал | [`@xterm/xterm`](https://xtermjs.org) |
| Редактор | [CodeMirror 6](https://codemirror.net) |
| SSH / SFTP | [`russh`](https://github.com/Eugeny/russh) · [`russh-sftp`](https://github.com/AspectUnk/russh-sftp) |
| Локальный PTY | [`portable-pty`](https://crates.io/crates/portable-pty) |
| Криптография | `aes-gcm` · `scrypt` · `ssh-key` · Windows DPAPI |

---

## 🏗 Архитектура

```
┌─────────────────────────── WebView (React) ───────────────────────────┐
│  App · TabBar · Sidebar · SftpPanel · Monitor · Docker · CodeEditor    │
│  └── src/api  ──  мост window.api  (invoke / listen)                   │
└───────────────────────────────┬───────────────────────────────────────┘
                     команды и события Tauri
┌───────────────────────────────┴───────────────────────────────────────┐
│  Rust-бэкенд (src-tauri/src)                                           │
│  ssh · ssh_agent · ssh_algos · proxycmd · serial · telnet · sftp ·     │
│  tunnels · monitor · docker · pty · term_out · store · vault ·         │
│  crypto · dpapi · keygen · importers · knownhosts · remoteedit         │
└────────────────────────────────────────────────────────────────────────┘
```

- React говорит с Rust через тонкий мост `window.api` (invoke / listen).
- Одно SSH-соединение мультиплексирует **shell + SFTP + exec + туннели**; handle берётся
  коротким async-локом, каналы не ждут друг друга.
- Секреты расшифровываются **только в Rust**, в момент подключения. Вне Windows DPAPI нет,
  и взамен ничего не пишется: base64 — не шифрование, поэтому порт должен сначала
  подключить Keychain / Secret Service.

---

## 👩‍💻 Сборка из исходников

Нужны [Rust](https://rustup.rs) (stable) и [Node.js](https://nodejs.org) 18+.
В неинтерактивном PowerShell `cargo` часто не в PATH — добавь `$env:USERPROFILE\.cargo\bin`.

Dev-сервер слушает **`127.0.0.1:1420`**, не `localhost` (IPv4/IPv6 иначе вечно «Waiting for frontend»).

```bash
npm install
npm run tauri dev
npm run tauri build
```

Установщик: `src-tauri/target/release/bundle/nsis/`.

```bash
npm run smoke
```

(`tsc --noEmit` + `cargo check`. Сценарий целиком — `docs/PHASE0.md`.)

---

## Данные и диагностика

Профили, секреты, known_hosts, vault — в `%APPDATA%\serein\` на Windows и
`~/.config/serein/` на Linux (`servers.json`, `secrets.json`, `vault.json`,
`known_hosts.json`, …). **Настройки показывают текущий путь** — это первое, что стоит
посмотреть, если список серверов вдруг пуст: запуск с другим `HOME` открывает другой профиль.

Секреты в UI-слой не едут. На Windows содержимое шифруется DPAPI и лежит в файле, на Linux —
хранится в **связке ключей**, а в файле только ссылка `kr:{uuid}`. Поэтому секреты с Windows
на Linux не переезжают, а там, где Secret Service не запущен, нужен мастер-пароль.

Запись вывода терминала включается по сессии и ложится в подкаталог `logs` без ANSI; общего
лог-файла приложения пока нет — для него смотри вывод `npm run tauri dev`.

---

## Известные ограничения

- Нет **SCP** (только SFTP) и **X11 forwarding**.
- Сборки только под **Windows x64**. `.dmg` / `.AppImage` нет.
- Установщик **без цифровой подписи**. SmartScreen будет спорить.
- Автообновление в конфиге есть, под неподписанный установщик пока не рассчитывай.
- Не цель ближайших релизов: облачный sync, мобилка, плагины, RDP/VNC, «просто чат с LLM».

Дорожная карта: надёжность → multi-host и automation → импорт из чужих клиентов. RDP/VNC пока вне планов. Подробнее — [release notes 1.2.5](docs/RELEASE_NOTES_v1.2.5.md).

---

## 📄 Лицензия

[Apache License 2.0](LICENSE) © 2026 HaDe
